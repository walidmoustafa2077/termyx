//! Rasterized glyph cache backed by a shared coverage atlas.

use std::collections::HashMap;

use ab_glyph::{Font, FontVec, GlyphId, PxScale};

use crate::fonts::em_px_scale;

use super::atlas::{Atlas, Rect};

/// Glyph masks are rasterized at this resolution multiplier and projected onto
/// the horizontal sub-pixel columns, reproducing the console host's ClearType
/// style coverage instead of plain grayscale anti-aliasing.
const RASTER_SUPERSAMPLE: u32 = 4;
/// RGB sub-pixel columns per logical pixel in the atlas mask.
const SUBPIXELS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// 0 = primary face, 1 = Arabic fallback face, 2.. = symbol fallback faces.
    pub font: u8,
    pub glyph: u16,
    pub size_px: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GlyphSlot {
    pub rect: Rect,
    pub bearing_x: i32,
    pub bearing_y: i32,
}

pub struct GlyphCache {
    pub atlas: Atlas,
    slots: HashMap<GlyphKey, Option<GlyphSlot>>,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self {
            atlas: Atlas::new(4096, 4096),
            slots: HashMap::new(),
        }
    }

    pub fn get(
        &mut self,
        font_id: u8,
        font: &FontVec,
        glyph: GlyphId,
        size_px: f32,
    ) -> Option<GlyphSlot> {
        let key = GlyphKey {
            font: font_id,
            glyph: glyph.0,
            size_px: size_px.round() as u32,
        };
        if let Some(slot) = self.slots.get(&key) {
            return *slot;
        }
        let slot = self.rasterize(font, glyph, key.size_px as f32);
        self.slots.insert(key, slot);
        slot
    }

    fn rasterize(&mut self, font: &FontVec, glyph: GlyphId, size_px: f32) -> Option<GlyphSlot> {
        // 1. Rasterize the outline at a 4x resolution so each sub-pixel column
        //    has enough samples to approximate the coverage integral.
        let big = RASTER_SUPERSAMPLE as f32;
        let outlined = font
            .outline_glyph(glyph.with_scale(PxScale::from(em_px_scale(font, size_px).y * big)))?;
        let bounds = outlined.px_bounds();
        let cols = bounds.width().ceil().max(1.0) as u32;
        let rows = bounds.height().ceil().max(1.0) as u32;
        let cells_w = cols.div_ceil(RASTER_SUPERSAMPLE);
        let cells_h = rows.div_ceil(RASTER_SUPERSAMPLE);
        if cells_w == 0 || cells_h == 0 {
            return None;
        }
        let big_w = cells_w * RASTER_SUPERSAMPLE;
        let big_h = cells_h * RASTER_SUPERSAMPLE;

        let mut big_mask = vec![0u8; (big_w * big_h) as usize];
        outlined.draw(|x, y, coverage| {
            if x < big_w && y < big_h {
                big_mask[(y * big_w + x) as usize] = (coverage * 255.0).round() as u8;
            }
        });

        // 2. Project onto the three sub-pixel columns: channel i of the logical
        //    pixel samples the horizontal coverage curve at its own offset.
        //    Vertically a single centre row is taken, matching the console
        //    host's grid-snapped rows instead of smearing over the band.
        let mask_w = cells_w * SUBPIXELS;
        let mask_h = cells_h;
        let stride = mask_w as usize;
        let mut mask = vec![0u8; stride * mask_h as usize];
        for y in 0..cells_h {
            for x in 0..cells_w {
                for sub in 0..SUBPIXELS {
                    // Sample the centre of channel i: (i + 0.5) / 3 of the cell.
                    let sample = (x as f32 + (sub as f32 + 0.5) / SUBPIXELS as f32) * big;
                    let base = (sample.floor() as u32).min(big_w - 1);
                    let next = (base + 1).min(big_w - 1);
                    let frac = sample - base as f32;
                    let y0 = (y * RASTER_SUPERSAMPLE + RASTER_SUPERSAMPLE / 2) as usize;
                    let lo = big_mask[y0 * big_w as usize + base as usize] as f32;
                    let hi = big_mask[y0 * big_w as usize + next as usize] as f32;
                    let coverage = lo + (hi - lo) * frac;
                    let index = y as usize * stride + (x * SUBPIXELS + sub) as usize;
                    mask[index] = (coverage * 255.0).round() as u8;
                }
            }
        }

        // 3. ClearType-style horizontal filter: each channel mixes itself with
        //    its sub-pixel neighbours so edge energy spreads like on an LCD
        //    instead of spiking in one channel.
        let mut filtered = vec![0u8; mask.len()];
        let taps = [0.25f32, 0.5, 0.25];
        for y in 0..mask_h as usize {
            for k in 0..stride {
                let mut acc = 0f32;
                for (t, weight) in taps.iter().enumerate() {
                    let offset = t as i32 - 1;
                    let column = (k as i32 + offset).clamp(0, stride as i32 - 1) as usize;
                    acc += weight * f32::from(mask[y * stride + column]);
                }
                filtered[y * stride + k] = (acc / taps.iter().sum::<f32>()).round() as u8;
            }
        }

        let rect = self.atlas.alloc(mask_w, mask_h)?;
        self.atlas.upload(rect, &filtered);
        Some(GlyphSlot {
            rect,
            bearing_x: (bounds.min.x / big).round() as i32,
            bearing_y: (bounds.min.y / big).round() as i32,
        })
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontSet;

    #[test]
    fn caches_the_same_glyph_slot() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        let mut cache = GlyphCache::new();
        let first = cache.get(0, &fonts.font, glyph, fonts.size_px).unwrap();
        let second = cache.get(0, &fonts.font, glyph, fonts.size_px).unwrap();
        assert_eq!(first.rect, second.rect);
        assert!(first.rect.w > 0 && first.rect.h > 0);
    }

    #[test]
    fn font_id_is_part_of_the_key() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        let mut cache = GlyphCache::new();
        assert!(cache.get(0, &fonts.font, glyph, fonts.size_px).is_some());
        assert!(cache.get(1, &fonts.font, glyph, fonts.size_px).is_some());
        assert_eq!(cache.slots.len(), 2);
    }

    #[test]
    fn missing_glyph_returns_none_and_is_cached() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id(' ');
        let mut cache = GlyphCache::new();
        assert!(cache.get(0, &fonts.font, glyph, fonts.size_px).is_none());
        assert!(cache.get(0, &fonts.font, glyph, fonts.size_px).is_none());
        let key = GlyphKey {
            font: 0,
            glyph: glyph.0,
            size_px: fonts.size_px.round() as u32,
        };
        assert!(matches!(cache.slots.get(&key), Some(&None)));
    }
}
