//! Grid rasterization to the window surface.

mod atlas;
mod glyph_cache;

use tiny_skia::Pixmap;

use crate::fonts::FontSet;
use crate::theme::{Rgb, Theme};
use atlas::Atlas;
use glyph_cache::{GlyphCache, GlyphSlot};

pub struct Renderer {
    pixmap: Pixmap,
    pixels: Vec<u32>,
    pub fonts: FontSet,
    pub cache: GlyphCache,
    pub theme: Theme,
    padding: u32,
}

impl Renderer {
    pub fn new(fonts: FontSet, theme: Theme, padding: u32) -> Self {
        Self {
            pixmap: Pixmap::new(1, 1).expect("1x1 pixmap"),
            pixels: vec![0],
            fonts,
            cache: GlyphCache::new(),
            theme,
            padding,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.pixmap = Pixmap::new(width, height).expect("pixmap allocation");
        self.pixels = vec![0; (width as usize) * (height as usize)];
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync();
    }

    pub fn draw(&mut self) {
        self.pixmap.fill(self.theme.background.to_tiny());
    }

    pub fn draw_text(&mut self, x: f32, baseline: f32, text: &str, color: Rgb) {
        let mut pen = x;
        let mut base_pen = x;
        for ch in text.chars() {
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            let draw_pen = if width == 0 { base_pen } else { pen };
            if let Some(glyph) = self.fonts.glyph(ch) {
                if let Some(slot) = self.cache.get(&self.fonts.font, glyph, self.fonts.size_px) {
                    blit(
                        &mut self.pixmap,
                        &self.cache.atlas,
                        slot,
                        draw_pen,
                        baseline,
                        color,
                    );
                }
            }
            if width > 0 {
                base_pen = pen;
                pen += self.fonts.cell_w as f32;
            }
        }
    }

    pub fn sync(&mut self) {
        for (index, pixel) in self.pixmap.data().chunks_exact(4).enumerate() {
            self.pixels[index] =
                ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
        }
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn padding(&self) -> u32 {
        self.padding
    }
}

fn blit(
    pixmap: &mut Pixmap,
    atlas: &Atlas,
    slot: GlyphSlot,
    pen_x: f32,
    baseline: f32,
    color: Rgb,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let stride = width as usize * 4;
    let dst_x = pen_x.round() as i32 + slot.bearing_x;
    let dst_y = baseline.round() as i32 + slot.bearing_y;
    let atlas_width = atlas.width();
    let data = pixmap.data_mut();
    for row in 0..slot.rect.h as i32 {
        for col in 0..slot.rect.w as i32 {
            let x = dst_x + col;
            let y = dst_y + row;
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let mask_x = slot.rect.x as i32 + col;
            let mask_y = slot.rect.y as i32 + row;
            let coverage = atlas.data()[(mask_y as u32 * atlas_width + mask_x as u32) as usize];
            if coverage == 0 {
                continue;
            }
            let alpha = coverage as f32 / 255.0;
            let index = y as usize * stride + x as usize * 4;
            data[index] = blend(data[index], color.r, alpha);
            data[index + 1] = blend(data[index + 1], color.g, alpha);
            data[index + 2] = blend(data[index + 2], color.b, alpha);
        }
    }
}

fn blend(dst: u8, src: u8, alpha: f32) -> u8 {
    (src as f32 * alpha + dst as f32 * (1.0 - alpha)).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeConfig;

    fn default_renderer() -> Renderer {
        let fonts = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("a monospace font must exist");
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4)
    }

    #[test]
    fn resize_fills_with_background_color() {
        let mut renderer = default_renderer();
        renderer.resize(2, 1);
        assert_eq!(renderer.pixels(), [0x000c_0c0c, 0x000c_0c0c]);
    }

    #[test]
    fn zero_sized_resize_keeps_previous_pixels() {
        let mut renderer = default_renderer();
        renderer.resize(4, 3);
        let expected_len = renderer.pixels().len();
        renderer.resize(0, 0);
        assert_eq!(renderer.pixels().len(), expected_len);
    }

    #[test]
    fn draw_text_paints_foreground_pixels() {
        let mut renderer = default_renderer();
        let width = renderer.fonts.cell_w * 4;
        let height = renderer.fonts.cell_h * 2;
        renderer.resize(width, height);
        renderer.draw();
        let foreground = renderer.theme.foreground;
        renderer.draw_text(0.0, renderer.fonts.ascent, "M", foreground);
        renderer.sync();
        assert!(renderer.pixels().iter().any(|pixel| *pixel != 0x000c_0c0c));
    }
}
