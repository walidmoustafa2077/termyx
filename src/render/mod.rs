//! Grid rasterization to the window surface.

mod atlas;
mod glyph_cache;

use tiny_skia::Pixmap;

use crate::fonts::FontSet;
use crate::term::{Color, GridSnapshot};
use crate::theme::{Rgb, Theme};
use crate::visual::VisualMap;
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

    pub fn draw(&mut self, snapshot: &GridSnapshot, maps: &[VisualMap]) {
        self.draw_background();

        let cell_w = self.fonts.cell_w as f32;
        let cell_h = self.fonts.cell_h as f32;
        let ascent = self.fonts.ascent;
        let padding = self.padding as f32;
        let cursor_color = self.theme.cursor;

        for (row, line) in snapshot.lines.iter().enumerate() {
            let Some(map) = maps.get(row) else {
                continue;
            };
            let y = padding + row as f32 * cell_h;
            for (logical, cell) in line.cells.iter().enumerate() {
                let x = padding + map.visual_of(logical) as f32 * cell_w;
                if let Some(bg) = cell.style.bg {
                    let color = self.resolve(bg);
                    self.fill_rect(x, y, cell_w, cell_h, color);
                }
                if cell.width > 0 && !cell.text.is_empty() {
                    let color = self.resolve(cell.style.fg);
                    self.draw_text(x, y + ascent, &cell.text, color);
                }
            }
        }

        if snapshot.display_offset == 0 {
            if let Some(map) = maps.get(snapshot.cursor.row) {
                let visual = map.cursor_visual(&snapshot.cursor);
                let x = padding + visual as f32 * cell_w;
                let y = padding + snapshot.cursor.row as f32 * cell_h;
                self.fill_rect(x, y, 2.0, cell_h, cursor_color);
            }
        }
    }

    fn draw_background(&mut self) {
        let width = self.pixmap.width() as f32;
        let height = self.pixmap.height() as f32;
        let color = self.theme.background;
        self.fill_rect(0.0, 0.0, width, height, color);
    }

    fn resolve(&self, color: Color) -> Rgb {
        match color {
            Color::Default => self.theme.foreground,
            Color::Indexed(index) => self.theme.indexed(index),
            Color::Rgb(r, g, b) => Rgb::new(r, g, b),
        }
    }

    fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Rgb) {
        let pixmap_w = self.pixmap.width() as i32;
        let pixmap_h = self.pixmap.height() as i32;
        let x0 = (x.round() as i32).clamp(0, pixmap_w);
        let y0 = (y.round() as i32).clamp(0, pixmap_h);
        let x1 = ((x + width).round() as i32).clamp(0, pixmap_w);
        let y1 = ((y + height).round() as i32).clamp(0, pixmap_h);
        let data = self.pixmap.data_mut();
        for py in y0..y1 {
            for px in x0..x1 {
                let index = ((py * pixmap_w + px) * 4) as usize;
                data[index] = color.r;
                data[index + 1] = color.g;
                data[index + 2] = color.b;
                data[index + 3] = 255;
            }
        }
    }

    pub fn fonts(&self) -> &FontSet {
        &self.fonts
    }

    pub fn draw_text(&mut self, x: f32, baseline: f32, text: &str, color: Rgb) {
        let mut pen = x;
        let mut base_pen = x;
        for ch in text.chars() {
            // Control characters report no width; treat them as one cell so
            // the pen still advances. Combining marks report a width of zero.
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
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
                pen += width as f32 * self.fonts.cell_w as f32;
            }
        }
    }

    /// Copies the pixmap into the CPU pixel buffer. Must run after drawing and
    /// before [`Renderer::pixels`] is read.
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

// Pixels are premultiplied RGBA over an opaque background (alpha stays 255),
// so blending only the colour channels is correct.
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

    /// Sorted, deduplicated pixel columns that differ from the background.
    fn painted_columns(renderer: &Renderer) -> Vec<u32> {
        let background = renderer.theme.background;
        let background =
            ((background.r as u32) << 16) | ((background.g as u32) << 8) | background.b as u32;
        let width = renderer.pixmap.width();
        let mut columns: Vec<u32> = renderer
            .pixels()
            .iter()
            .enumerate()
            .filter(|(_, pixel)| **pixel != background)
            .map(|(index, _)| index as u32 % width)
            .collect();
        columns.sort_unstable();
        columns.dedup();
        columns
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
        let foreground = renderer.theme.foreground;
        renderer.draw_text(0.0, renderer.fonts.ascent, "M", foreground);
        renderer.sync();
        assert!(renderer.pixels().iter().any(|pixel| *pixel != 0x000c_0c0c));
    }

    #[test]
    fn glyphs_advance_exactly_one_cell() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let height = renderer.fonts.cell_h * 2;
        renderer.resize(cell_w * 3, height);
        let foreground = renderer.theme.foreground;
        renderer.draw_text(0.0, renderer.fonts.ascent, "MM", foreground);
        renderer.sync();
        let columns = painted_columns(&renderer);
        // The second glyph starts in its own cell, so its leftmost painted
        // column lies on or after the boundary between cell 0 and cell 1.
        let second_glyph_left = columns
            .iter()
            .copied()
            .find(|column| *column >= cell_w)
            .expect("the second glyph must be painted");
        assert!(second_glyph_left >= cell_w - 1);
        assert!(*columns.last().unwrap() < cell_w * 2);
    }

    #[test]
    fn combining_marks_do_not_advance_the_pen() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let height = renderer.fonts.cell_h * 2;
        renderer.resize(cell_w * 3, height);
        let foreground = renderer.theme.foreground;
        renderer.draw_text(0.0, renderer.fonts.ascent, "e\u{0301}", foreground);
        renderer.sync();
        let columns = painted_columns(&renderer);
        assert!(!columns.is_empty(), "the base glyph must be painted");
        assert!(*columns.last().unwrap() < cell_w);
    }
}
