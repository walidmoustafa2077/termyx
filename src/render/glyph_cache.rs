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
            // 4096²: subpixel coverage triples the mask width, so the previous
            // 2048² filled after a few thousand glyphs at 32 px and then
            // silently stopped painting CJK, Arabic, and symbol-heavy cells.
            atlas: Atlas::new(4096, 4096),
            slots: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn with_atlas(atlas: Atlas) -> Self {
        Self {
            atlas,
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
        let slot = self.slot_for(font, glyph, key);
        self.slots.insert(key, slot);
        slot
    }

    /// Rasterizes `key`, reclaiming the atlas once if it has filled up.
    ///
    /// A full atlas is a resource problem, not a font one: caching it as a miss
    /// would leave that glyph blank for the rest of the process with no way to
    /// recover.
    fn slot_for(&mut self, font: &FontVec, glyph: GlyphId, key: GlyphKey) -> Option<GlyphSlot> {
        match self.rasterize(font, glyph, key.size_px as f32) {
            Rasterized::Slot(slot) => Some(slot),
            // The face has no drawable outline for this glyph at this size;
            // that is a property of the font, so remembering it is correct.
            Rasterized::NoOutline => None,
            Rasterized::Full => {
                // Every cached slot points into the atlas that is about to be
                // reused, so the slots must go with the coverage they describe.
                self.atlas.clear();
                self.slots.clear();
                match self.rasterize(font, glyph, key.size_px as f32) {
                    Rasterized::Slot(slot) => Some(slot),
                    // A glyph that cannot fit even an empty atlas is
                    // unrenderable at this size; no retry can change that.
                    Rasterized::NoOutline | Rasterized::Full => None,
                }
            }
        }
    }

    fn rasterize(&mut self, font: &FontVec, glyph: GlyphId, size_px: f32) -> Rasterized {
        // 1. Rasterize the outline at a 4x resolution so each sub-pixel column
        //    has enough samples to approximate the coverage integral.
        let big = RASTER_SUPERSAMPLE as f32;
        let Some(outlined) =
            font.outline_glyph(glyph.with_scale(PxScale::from(em_px_scale(font, size_px).y * big)))
        else {
            return Rasterized::NoOutline;
        };
        let bounds = outlined.px_bounds();
        let cols = bounds.width().ceil().max(1.0) as u32;
        let rows = bounds.height().ceil().max(1.0) as u32;
        let cells_w = cols.div_ceil(RASTER_SUPERSAMPLE);
        let cells_h = rows.div_ceil(RASTER_SUPERSAMPLE);
        if cells_w == 0 || cells_h == 0 {
            return Rasterized::NoOutline;
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
        //    Vertically every sub-row is averaged, so a pixel's alpha is the
        //    glyph's real area coverage; taking one row instead would paint
        //    horizontal strokes at full opacity and make the text too bold.
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
                    let mut coverage = 0f32;
                    for dy in 0..RASTER_SUPERSAMPLE {
                        let y0 = (y * RASTER_SUPERSAMPLE + dy) as usize;
                        let lo = big_mask[y0 * big_w as usize + base as usize] as f32;
                        let hi = big_mask[y0 * big_w as usize + next as usize] as f32;
                        coverage += lo + (hi - lo) * frac;
                    }
                    coverage /= RASTER_SUPERSAMPLE as f32;
                    let index = y as usize * stride + (x * SUBPIXELS + sub) as usize;
                    mask[index] = coverage.round() as u8;
                }
            }
        }

        let Some(rect) = self.atlas.alloc(mask_w, mask_h) else {
            return Rasterized::Full;
        };
        self.atlas.upload(rect, &mask);
        Rasterized::Slot(GlyphSlot {
            rect,
            bearing_x: (bounds.min.x / big).round() as i32,
            bearing_y: (bounds.min.y / big).round() as i32,
        })
    }
}

/// Why one rasterization pass did or did not produce a slot.
enum Rasterized {
    Slot(GlyphSlot),
    /// The face has no drawable outline for this glyph at this size.
    NoOutline,
    /// The atlas had no room; the glyph itself is fine.
    Full,
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

    #[test]
    fn a_full_atlas_is_cleared_and_the_glyph_retried() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        // A small atlas reaches exhaustion in a few allocations, so the retry
        // can be proven without filling a real one glyph by glyph.
        let mut cache = GlyphCache::with_atlas(Atlas::new(64, 64));
        while cache.atlas.alloc(64, 8).is_some() {}
        assert!(cache.atlas.alloc(1, 1).is_none(), "the atlas must be full");

        // A slot cached before the clear describes the old layout; the next
        // allocation will hand its rect to another glyph.
        let stale = GlyphKey {
            font: 0,
            glyph: glyph.0,
            size_px: 0,
        };
        cache.slots.insert(
            stale,
            Some(GlyphSlot {
                rect: Rect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                },
                bearing_x: 0,
                bearing_y: 0,
            }),
        );

        let slot = cache
            .get(0, &fonts.font, glyph, fonts.size_px)
            .expect("the exhausted atlas must be cleared and the glyph rasterized");
        assert!(slot.rect.w > 0 && slot.rect.h > 0);
        assert!(
            !cache.slots.contains_key(&stale),
            "the clear must drop slots that point into the reused atlas"
        );
        assert_eq!(cache.slots.len(), 1, "only the retried glyph is cached");
    }
}
