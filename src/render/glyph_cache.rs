//! Rasterized glyph cache backed by a shared coverage atlas.

use std::collections::HashMap;

use ab_glyph::{Font, FontVec, GlyphId, PxScale};

use super::atlas::{Atlas, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
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
            atlas: Atlas::new(2048, 2048),
            slots: HashMap::new(),
        }
    }

    pub fn get(&mut self, font: &FontVec, glyph: GlyphId, size_px: f32) -> Option<GlyphSlot> {
        let key = GlyphKey {
            glyph: glyph.0,
            size_px: size_px.round() as u32,
        };
        if let Some(slot) = self.slots.get(&key) {
            return *slot;
        }
        let slot = self.rasterize(font, glyph, size_px);
        self.slots.insert(key, slot);
        slot
    }

    fn rasterize(&mut self, font: &FontVec, glyph: GlyphId, size_px: f32) -> Option<GlyphSlot> {
        let scale = PxScale::from(size_px);
        let outlined = font.outline_glyph(glyph.with_scale(scale))?;
        let bounds = outlined.px_bounds();
        let width = bounds.width().ceil() as u32;
        let height = bounds.height().ceil() as u32;
        if width == 0 || height == 0 {
            return None;
        }
        let rect = self.atlas.alloc(width, height)?;
        let mut mask = vec![0u8; (width * height) as usize];
        outlined.draw(|x, y, coverage| {
            if x < width && y < height {
                mask[(y * width + x) as usize] = (coverage * 255.0).round() as u8;
            }
        });
        self.atlas.upload(rect, &mask);
        Some(GlyphSlot {
            rect,
            bearing_x: bounds.min.x.round() as i32,
            bearing_y: bounds.min.y.round() as i32,
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
        let fonts = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        let mut cache = GlyphCache::new();
        let first = cache.get(&fonts.font, glyph, fonts.size_px).unwrap();
        let second = cache.get(&fonts.font, glyph, fonts.size_px).unwrap();
        assert_eq!(first.rect, second.rect);
        assert!(first.rect.w > 0 && first.rect.h > 0);
    }

    #[test]
    fn missing_glyph_returns_none_and_is_cached() {
        let fonts = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id(' ');
        let mut cache = GlyphCache::new();
        assert!(cache.get(&fonts.font, glyph, fonts.size_px).is_none());
        assert!(cache.get(&fonts.font, glyph, fonts.size_px).is_none());
    }
}
