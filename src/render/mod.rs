//! Grid rasterization to the window surface.

mod atlas;
mod glyph_cache;

use ab_glyph::GlyphId;
use tiny_skia::Pixmap;

use crate::bidi;
use crate::fonts::FontSet;
use crate::scrollbar;
use crate::selection::Selection;
use crate::shape;
use crate::term::{Color, CursorShape, GridSnapshot, RowSnapshot, Style};
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
    /// Pixels reserved above the grid for the tab strip.
    top_inset: u32,
    shape_cache: shape::ShapeCache,
    /// Whether the missing-shaping warning has already been logged.
    shape_warned: bool,
    /// Whether the pointer is over the scrollbar, which widens its thumb.
    scrollbar_hover: bool,
}

impl Renderer {
    pub fn new(fonts: FontSet, theme: Theme, padding: u32) -> anyhow::Result<Self> {
        let pixmap = Pixmap::new(1, 1)
            .ok_or_else(|| anyhow::anyhow!("could not allocate the initial pixmap"))?;
        Ok(Self {
            pixmap,
            pixels: vec![0],
            fonts,
            cache: GlyphCache::new(),
            theme,
            padding,
            top_inset: 0,
            shape_cache: shape::ShapeCache::new(),
            shape_warned: false,
            scrollbar_hover: false,
        })
    }

    /// Resizes the drawing surface. Returns `false` for a zero size or when the
    /// allocation fails, in which case the previous pixmap is kept.
    pub fn resize(&mut self, width: u32, height: u32) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        let Some(pixmap) = Pixmap::new(width, height) else {
            tracing::error!(
                width,
                height,
                "pixmap allocation failed; keeping previous size"
            );
            return false;
        };
        self.pixmap = pixmap;
        self.pixels = vec![0; (width as usize) * (height as usize)];
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync();
        true
    }

    pub fn draw(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        selection: Option<&Selection>,
    ) {
        self.shape_cache.clear();
        self.draw_background();
        let cell_w = self.fonts.cell_w as f32;
        let cell_h = self.fonts.cell_h as f32;
        let padding = self.padding as f32;
        let top = self.grid_top();

        self.draw_cell_backgrounds(snapshot, maps, selection, cell_w, cell_h);

        let cursor = if snapshot.display_offset == 0 && snapshot.cursor.visible {
            maps.get(snapshot.cursor.row).map(|map| {
                (
                    snapshot.cursor.row,
                    map.cursor_visual(&snapshot.cursor),
                    snapshot.cursor.shape,
                )
            })
        } else {
            None
        };
        if let Some((row, visual, shape)) = cursor {
            let x = padding + visual as f32 * cell_w;
            let y = top + row as f32 * cell_h;
            match shape {
                CursorShape::Block => {
                    self.fill_rect(x, y, cell_w, cell_h, self.theme.cursor);
                }
                CursorShape::Beam => {
                    let width = (cell_w / 8.0).max(1.0);
                    self.fill_rect(x, y, width, cell_h, self.theme.cursor);
                }
                CursorShape::Underline => {
                    self.fill_rect(x, y + cell_h - 2.0, cell_w, 2.0, self.theme.cursor);
                }
                CursorShape::HollowBlock => {
                    self.fill_rect(x, y, cell_w, 1.0, self.theme.cursor);
                    self.fill_rect(x, y + cell_h - 1.0, cell_w, 1.0, self.theme.cursor);
                    self.fill_rect(x, y, 1.0, cell_h, self.theme.cursor);
                    self.fill_rect(x + cell_w - 1.0, y, 1.0, cell_h, self.theme.cursor);
                }
            }
        }

        let mut shaped_rows: Vec<Vec<shape::ShapedRun>> = Vec::with_capacity(snapshot.lines.len());
        if let Some(faces) = shape::Faces::from_fonts(&self.fonts) {
            for (row, line) in snapshot.lines.iter().enumerate() {
                let Some(map) = maps.get(row) else {
                    shaped_rows.push(Vec::new());
                    continue;
                };
                let text = bidi::RowText::flatten(line);
                shaped_rows.push(shape::shape_row(&text, map, &faces, &mut self.shape_cache));
            }
        } else {
            if !self.shape_warned {
                self.shape_warned = true;
                tracing::warn!(
                    "font data could not be parsed for shaping; drawing backgrounds only"
                );
            }
            shaped_rows.resize_with(snapshot.lines.len(), Vec::new);
        }

        for (row, runs) in shaped_rows.iter().enumerate() {
            let Some(line) = snapshot.lines.get(row) else {
                continue;
            };
            let rtl_base = maps.get(row).is_some_and(|map| map.rtl_base());
            for run in runs {
                self.draw_shaped_run(run, line, row, cursor, rtl_base);
            }
        }

        self.draw_decorations(snapshot, maps, cell_w, cell_h);

        if let Some(geom) = scrollbar::geometry(
            self.pixmap.width(),
            self.padding + self.top_inset,
            self.fonts.cell_h,
            snapshot.rows,
            snapshot.history,
            snapshot.display_offset,
            self.scrollbar_hover,
        ) {
            let color = self.theme.indexed(8);
            if self.scrollbar_hover {
                let track_w = 2.0;
                let track_x = geom.x + (geom.width - track_w) / 2.0;
                let track = Rgb::new(
                    blend(self.theme.background.r, color.r, 0.4),
                    blend(self.theme.background.g, color.g, 0.4),
                    blend(self.theme.background.b, color.b, 0.4),
                );
                self.fill_rect(track_x, geom.track_y, track_w, geom.track_h, track);
            }
            self.fill_rect(geom.x, geom.thumb_y, geom.width, geom.thumb_h, color);
        }
    }

    fn draw_cell_backgrounds(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        selection: Option<&Selection>,
        cell_w: f32,
        cell_h: f32,
    ) {
        for (row, line) in snapshot.lines.iter().enumerate() {
            let Some(map) = maps.get(row) else {
                continue;
            };
            let y = self.grid_top() + row as f32 * cell_h;
            for (logical, cell) in line.cells.iter().enumerate() {
                let x = self.padding as f32 + map.visual_of(logical) as f32 * cell_w;
                let (_, background) = self.cell_colors(cell.style);
                if cell.style.bg.is_some() || cell.style.inverse {
                    self.fill_rect(x, y, cell_w, cell_h, background);
                }
                if selection.is_some_and(|selection| {
                    selection.is_selected(row, logical, map.visual_of(logical), maps)
                }) {
                    self.fill_rect(x, y, cell_w, cell_h, self.theme.selection);
                }
            }
        }
    }

    fn draw_shaped_run(
        &mut self,
        run: &shape::ShapedRun,
        line: &RowSnapshot,
        row: usize,
        cursor: Option<(usize, usize, CursorShape)>,
        rtl_base: bool,
    ) {
        let cell_w = self.fonts.cell_w as f32;
        let cell_h = self.fonts.cell_h as f32;
        let padding = self.padding as f32;
        let top = self.grid_top();
        // Share one baseline across the primary and Arabic faces so Arabic
        // sits on the same line as Latin instead of floating above it.
        let baseline = top + row as f32 * cell_h + self.fonts.line_ascent();
        let units = run.units_per_em.max(1) as f32;
        let face_scale = if run.arabic {
            self.fonts.arabic.as_ref().map_or(1.0, |face| face.scale)
        } else {
            1.0
        };
        let scale = self.fonts.size_px * face_scale / units;
        // Shape the whole Arabic run continuously. Snapping each cluster to
        // its own cell leaves side-bearing gaps that break cursive joins.
        let arabic_natural: f32 = run
            .clusters
            .iter()
            .flat_map(|cluster| cluster.glyphs.iter())
            .map(|glyph| glyph.x_advance as f32 * scale)
            .sum();
        // Connected Arabic forms are usually narrower than their cells, so
        // let the run keep its natural advances and anchor it to the
        // paragraph's start edge. That keeps the Arabic next to the preceding
        // Latin text in an LTR line, and keeps an RTL line pinned to the right,
        // without stretching the letterforms. Only shrink a run that would
        // overflow.
        let arabic_span = (run.slots.end - run.slots.start) as f32 * cell_w;
        let arabic_scale_x = if arabic_natural > arabic_span && arabic_natural > 0.0 {
            arabic_span / arabic_natural
        } else {
            1.0
        };
        let mut arabic_pen = if rtl_base {
            padding + run.slots.end as f32 * cell_w - arabic_natural * arabic_scale_x
        } else {
            padding + run.slots.start as f32 * cell_w
        };

        for cluster in &run.clusters {
            let Some(first) = line.cells.get(cluster.cells.start) else {
                continue;
            };
            if first.style.hidden {
                continue;
            }
            if cluster.cells.len() == 1 && bidi::is_bidi_control_only(&first.text) {
                continue;
            }
            let (color, _) = self.cell_colors(first.style);
            let under_cursor = cursor.is_some_and(|(cursor_row, visual, shape)| {
                shape == CursorShape::Block && cursor_row == row && cluster.slots.contains(&visual)
            });
            let color = if under_cursor {
                self.theme.background
            } else {
                color
            };
            let slot_start = cluster.slots.start.max(run.slots.start);
            let slot_end = cluster.slots.end.min(run.slots.end);
            if slot_start >= slot_end {
                continue;
            }
            let slot_w = (slot_end - slot_start) as f32 * cell_w;
            let natural: f32 = cluster
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance as f32 * scale)
                .sum();
            let (offset, scale_x) = if run.arabic {
                (0.0, arabic_scale_x)
            } else {
                fit_cluster(natural, slot_w)
            };
            let mut pen = if run.arabic {
                arabic_pen
            } else {
                padding + slot_start as f32 * cell_w + offset
            };

            // A half block is a cell-sized tile, not ordinary text. The console
            // host snaps it to the cell; drawing the font outline leaves a
            // one-pixel seam against the neighbouring background because the
            // em square starts below the line box.
            if !run.arabic && cluster.text == "\u{2580}" {
                self.fill_rect(
                    padding + slot_start as f32 * cell_w,
                    top + row as f32 * cell_h,
                    slot_w,
                    cell_h / 2.0,
                    color,
                );
                continue;
            }

            for (glyph_index, glyph) in cluster.glyphs.iter().enumerate() {
                let mut font_id = u8::from(run.arabic);
                let (mut font, _) = self.fonts.font_for(run.arabic);
                // ab_glyph ids are u16; an out-of-range id becomes .notdef.
                let mut id = GlyphId(u16::try_from(glyph.glyph_id).unwrap_or(0));
                // Only the cluster's first glyph may switch faces: later glyphs
                // share the cluster's text, so looking them up would paint the
                // base character's fallback glyph a second time.
                if glyph_index == 0 && glyph.glyph_id == 0 {
                    if let Some((symbol_face, symbol_glyph)) = self.fonts.symbol_for(&cluster.text)
                    {
                        font_id = symbol_face;
                        (font, _) = self.fonts.font_for_id(symbol_face);
                        id = symbol_glyph;
                    }
                }
                if let Some(slot) =
                    self.cache
                        .get(font_id, font, id, self.fonts.size_px * face_scale)
                {
                    let x = pen + glyph.x_offset as f32 * scale * scale_x;
                    let y = baseline - glyph.y_offset as f32 * scale;
                    blit_scaled(
                        &mut self.pixmap,
                        &self.cache.atlas,
                        slot,
                        x,
                        y,
                        Ink {
                            color,
                            bold: first.style.bold,
                        },
                        scale_x,
                    );
                }
                pen += glyph.x_advance as f32 * scale * scale_x;
            }
            if run.arabic {
                arabic_pen = pen;
            }
        }
    }

    fn draw_decorations(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        cell_w: f32,
        cell_h: f32,
    ) {
        for (row, line) in snapshot.lines.iter().enumerate() {
            let Some(map) = maps.get(row) else {
                continue;
            };
            let y = self.grid_top() + row as f32 * cell_h;
            for (logical, cell) in line.cells.iter().enumerate() {
                if cell.style.hidden {
                    continue;
                }
                let (foreground, _) = self.cell_colors(cell.style);
                let x = self.padding as f32 + map.visual_of(logical) as f32 * cell_w;
                let span = cell_w * cell.width.max(1) as f32;
                if cell.style.underline {
                    self.fill_rect(
                        x,
                        (y + self.fonts.ascent + 1.0).min(y + cell_h - 1.0),
                        span,
                        1.0,
                        foreground,
                    );
                }
                if cell.style.strikethrough {
                    self.fill_rect(x, y + (cell_h / 2.0).round(), span, 1.0, foreground);
                }
            }
        }
    }

    fn draw_background(&mut self) {
        // The background covers the whole pixmap, so a bulk fill is cheaper
        // than walking it as a rectangle, and a measurable slice of the frame.
        self.pixmap.fill(self.theme.background.to_tiny());
    }

    fn resolve(&self, color: Color) -> Rgb {
        match color {
            Color::Default => self.theme.foreground,
            Color::Indexed(index) => self.theme.indexed(index),
            Color::Rgb(r, g, b) => Rgb::new(r, g, b),
        }
    }

    /// Resolved `(glyph, background)` colors. An inverse cell swaps the pair,
    /// so its glyphs take the background color and its fill takes the
    /// foreground color.
    fn cell_colors(&self, style: Style) -> (Rgb, Rgb) {
        let foreground = self.resolve(style.fg);
        let background = style
            .bg
            .map(|bg| self.resolve(bg))
            .unwrap_or(self.theme.background);
        if style.inverse {
            (background, foreground)
        } else {
            (foreground, background)
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

    /// Pixels reserved above the grid for the tab strip. Drawing offsets every
    /// grid row and the scrollbar's track by this amount.
    pub fn set_top_inset(&mut self, inset: u32) {
        self.top_inset = inset;
    }

    pub fn top_inset(&self) -> u32 {
        self.top_inset
    }

    /// Y of the grid's first row: the window padding plus the tab strip.
    fn grid_top(&self) -> f32 {
        (self.padding + self.top_inset) as f32
    }

    /// Marks whether the pointer is over the scrollbar. A hovered scrollbar
    /// widens its thumb and shows the track.
    pub fn set_scrollbar_hover(&mut self, hover: bool) {
        self.scrollbar_hover = hover;
    }
}

/// Centers a cluster that fits its slot and scales one that does not.
fn fit_cluster(natural: f32, slot_w: f32) -> (f32, f32) {
    if slot_w <= 0.0 || natural <= 0.0 {
        return (0.0, 1.0);
    }
    if natural > slot_w * 1.05 {
        (0.0, slot_w / natural)
    } else {
        ((slot_w - natural) / 2.0, 1.0)
    }
}

struct Ink {
    color: Rgb,
    bold: bool,
}

fn blit_scaled(
    pixmap: &mut Pixmap,
    atlas: &Atlas,
    slot: GlyphSlot,
    pen_x: f32,
    baseline: f32,
    ink: Ink,
    scale_x: f32,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let stride = width as usize * 4;
    let src_w = slot.rect.w / SUBPIXELS;
    let dst_w = (src_w as f32 * scale_x).round().max(1.0) as i32;
    let dst_x = pen_x.round() as i32 + (slot.bearing_x as f32 * scale_x).round() as i32;
    let dst_y = baseline.round() as i32 + slot.bearing_y;
    let atlas_width = atlas.width() as usize;
    let atlas_data = atlas.data();
    let gamma = gamma_table();
    let color = ink.color;
    // Nearly every glyph is one cell wide, so its columns map straight through
    // and the per-pixel float division can be skipped.
    let unscaled = (scale_x - 1.0).abs() < f32::EPSILON;
    let data = pixmap.data_mut();
    for row in 0..slot.rect.h as i32 {
        let y = dst_y + row;
        if y < 0 || y >= height {
            continue;
        }
        let mask_row = (slot.rect.y as i32 + row) as usize * atlas_width;
        for col in 0..dst_w {
            let x = dst_x + col;
            if x < 0 || x >= width {
                continue;
            }
            let src_col = if unscaled {
                col as u32
            } else {
                (col as f32 / scale_x).floor().min((src_w - 1) as f32) as u32
            };
            let mask = mask_row + slot.rect.x as usize + src_col as usize * SUBPIXELS as usize;
            let mut c0 = atlas_data[mask];
            let mut c1 = atlas_data[mask + 1];
            let mut c2 = atlas_data[mask + 2];
            if ink.bold && src_col > 0 {
                // Synthetic bold: borrow coverage from the logical left
                // neighbour so stems thicken without a separate bold face.
                c0 = c0.max(atlas_data[mask - 3]);
                c1 = c1.max(atlas_data[mask - 2]);
                c2 = c2.max(atlas_data[mask - 1]);
            }
            let sum = usize::from(c0) + usize::from(c1) + usize::from(c2);
            if sum == 0 {
                continue;
            }
            let alpha = u32::from(gamma[sum]);
            let index = y as usize * stride + x as usize * 4;
            data[index] = blend_u8(data[index], color.r, alpha);
            data[index + 1] = blend_u8(data[index + 1], color.g, alpha);
            data[index + 2] = blend_u8(data[index + 2], color.b, alpha);
        }
    }
}

const SUBPIXELS: u32 = 3;

// Pixels are premultiplied RGBA over an opaque background (alpha stays 255),
// so blending only the colour channels is correct.
fn blend(dst: u8, src: u8, alpha: f32) -> u8 {
    (src as f32 * alpha + dst as f32 * (1.0 - alpha)).round() as u8
}

/// Integer counterpart of [`blend`] for the glyph path, where a per-pixel
/// floating-point multiply shows up in the full-frame budget.
#[inline]
fn blend_u8(dst: u8, src: u8, alpha: u32) -> u8 {
    ((u32::from(src) * alpha + u32::from(dst) * (255 - alpha) + 127) / 255) as u8
}

/// Horizontal sub-pixel coverages average into one grayscale alpha. Windows
/// Terminal renders small text with grayscale antialiasing, so deriving a
/// separate red/green/blue alpha from each sub-pixel column would tint every
/// edge with red and blue fringes the user does not see in the console host.
/// The gamma keeps thin stems visible on a dark background.
fn gamma_table() -> &'static [u8; 766] {
    const GAMMA: f32 = 1.2;
    // `powf` dominates a full-frame redraw, and the mean only has 766
    // reachable values, so evaluate it once per sum instead of per pixel.
    static LUT: std::sync::OnceLock<[u8; 766]> = std::sync::OnceLock::new();
    LUT.get_or_init(|| {
        let mut table = [0; 766];
        for (sum, alpha) in table.iter_mut().enumerate() {
            *alpha = ((sum as f32 / 765.0).powf(GAMMA) * 255.0).round() as u8;
        }
        table
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BaseDirection, ThemeConfig};
    use crate::term::{
        Affinity, Cell, Color, CursorShape, CursorState, LogicalPos, RowSnapshot, Style,
    };

    #[test]
    fn glyph_edges_are_grayscale() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = single_cell_snapshot(Cell {
            text: "M".into(),
            width: 1,
            style: Style::default(),
        });
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let width = renderer.pixmap.width() as usize;
        let background = packed(renderer.theme.background);
        let mut painted = 0;
        let mut tinted = 0;
        for (index, px) in renderer.pixels().iter().enumerate() {
            let px = *px;
            if index % width == 0 || px == background {
                continue;
            }
            painted += 1;
            let r = (px >> 16) & 0xFF;
            let g = (px >> 8) & 0xFF;
            let b = px & 0xFF;
            if r != g || g != b {
                tinted += 1;
            }
        }
        assert!(painted > 0, "the glyph must paint some pixels");
        assert_eq!(tinted, 0, "glyph edges must be neutral gray, not tinted");
    }

    #[test]
    fn upper_half_block_fills_the_top_half_of_the_cell() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h as usize;
        let padding = renderer.padding() as usize;
        renderer.resize(cell_w * 2, cell_h as u32 * 2);
        let snapshot = single_cell_snapshot(Cell {
            text: "\u{2580}".into(),
            width: 1,
            style: Style {
                fg: Color::Rgb(30, 30, 30),
                ..Style::default()
            },
        });
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let width = renderer.pixmap.width() as usize;
        let background = packed(renderer.theme.background);
        let at = |y: usize| renderer.pixels()[y * width + padding];
        assert_ne!(
            at(padding),
            background,
            "the block must reach the cell's top edge"
        );
        assert_ne!(at(padding + cell_h / 2 - 1), background);
        assert_eq!(
            at(padding + cell_h - 1),
            background,
            "the cell's lower half must stay empty"
        );
    }

    #[test]
    fn the_grid_starts_below_the_top_inset() {
        let mut renderer = default_renderer();
        renderer.resize(200, 120);
        renderer.set_top_inset(32);
        assert_eq!(renderer.top_inset(), 32);

        let cell_h = renderer.fonts.cell_h;
        let padding = renderer.padding();
        let snapshot = single_cell_snapshot(Cell {
            text: "x".into(),
            width: 1,
            style: Style::default(),
        });
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();

        let rows = painted_rows(&renderer);
        let first = *rows.first().expect("the glyph must paint some ink");
        assert!(
            first >= (padding + 32) as usize,
            "nothing may paint inside the strip (topmost ink at row {first})"
        );
        assert!(
            first < (padding + 32 + cell_h) as usize,
            "the glyph must paint in the first row below the inset (topmost ink at row {first})"
        );
    }

    fn default_renderer() -> Renderer {
        let fonts = FontSet::load("Consolas", "Arial", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Arial", 14.0, 1.2))
            .expect("a monospace font must exist");
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4)
            .expect("renderer must be constructible")
    }

    fn arabic_renderer() -> Option<Renderer> {
        let fonts = FontSet::load("Consolas", "Arial", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Arial", 14.0, 1.2))
            .ok()?;
        if !fonts.has_arabic() {
            return None;
        }
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4).ok()
    }

    fn packed(color: Rgb) -> u32 {
        ((color.r as u32) << 16) | ((color.g as u32) << 8) | color.b as u32
    }

    fn single_cell_snapshot(cell: Cell) -> GridSnapshot {
        GridSnapshot {
            cols: 1,
            rows: 1,
            lines: vec![RowSnapshot {
                cells: vec![cell],
                wrapped: false,
            }],
            cursor: CursorState {
                row: 1,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        }
    }

    #[test]
    fn synthetic_bold_thickens_stems() {
        let mut atlas = Atlas::new(8, 4);
        // Two logical pixels wide: the first carries a stem, the second is
        // empty, so synthetic bold has a left neighbour to borrow from.
        let rect = atlas.alloc(6, 2).expect("atlas space");
        atlas.upload(rect, &[255, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let slot = GlyphSlot {
            rect,
            bearing_x: 0,
            bearing_y: 0,
        };
        let color = Rgb::new(255, 255, 255);

        let mut plain = Pixmap::new(8, 8).expect("pixmap");
        plain.fill(tiny_skia::Color::BLACK);
        blit_scaled(
            &mut plain,
            &atlas,
            slot,
            1.0,
            3.0,
            Ink { color, bold: false },
            1.0,
        );

        let mut bold = Pixmap::new(8, 8).expect("pixmap");
        bold.fill(tiny_skia::Color::BLACK);
        blit_scaled(
            &mut bold,
            &atlas,
            slot,
            1.0,
            3.0,
            Ink { color, bold: true },
            1.0,
        );

        let ink = |pixmap: &Pixmap| {
            pixmap
                .data()
                .chunks_exact(4)
                .filter(|pixel| pixel[0] > 0)
                .count()
        };
        assert!(ink(&bold) > ink(&plain), "bold must add stem coverage");
    }

    fn cursor_snapshot(shape: CursorShape, visible: bool) -> GridSnapshot {
        let mut snapshot = single_cell_snapshot(Cell {
            text: " ".into(),
            width: 1,
            style: Style::default(),
        });
        snapshot.cursor.row = 0;
        snapshot.cursor.shape = shape;
        snapshot.cursor.visible = visible;
        snapshot
    }

    /// Draws a single `"M"` cell with the cursor at the cell origin and copies
    /// the resulting pixels out.
    fn render_m_with_cursor(
        renderer: &mut Renderer,
        shape: CursorShape,
        visible: bool,
    ) -> Vec<u32> {
        let mut snapshot = single_cell_snapshot(Cell {
            text: "M".into(),
            width: 1,
            style: Style::default(),
        });
        snapshot.cursor.row = 0;
        snapshot.cursor.shape = shape;
        snapshot.cursor.visible = visible;
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        renderer.pixels().to_vec()
    }

    fn row_snapshot(cells: Vec<Cell>) -> GridSnapshot {
        let cols = cells.len();
        GridSnapshot {
            cols,
            rows: 1,
            lines: vec![RowSnapshot {
                cells,
                wrapped: false,
            }],
            cursor: CursorState {
                row: 1,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        }
    }

    fn scroll_snapshot(rows: usize, history: usize, offset: usize) -> GridSnapshot {
        let cell = Cell {
            text: " ".into(),
            width: 1,
            style: Style::default(),
        };
        GridSnapshot {
            cols: 1,
            rows,
            lines: (0..rows)
                .map(|_| RowSnapshot {
                    cells: vec![cell.clone()],
                    wrapped: false,
                })
                .collect(),
            cursor: CursorState {
                row: rows,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: false,
                blinking: false,
            },
            display_offset: offset,
            history,
        }
    }

    fn painted_rows(renderer: &Renderer) -> Vec<usize> {
        let background = packed(renderer.theme.background);
        let width = renderer.pixmap.width() as usize;
        renderer
            .pixels()
            .iter()
            .enumerate()
            .filter(|(_, pixel)| **pixel != background)
            .map(|(index, _)| index / width)
            .collect()
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
        assert!(renderer.resize(4, 3));
        let expected_len = renderer.pixels().len();
        assert!(!renderer.resize(0, 0));
        assert_eq!(renderer.pixels().len(), expected_len);
    }

    #[test]
    fn fit_cluster_centers_narrow_and_shrinks_wide_clusters() {
        assert_eq!(fit_cluster(5.0, 10.0), (2.5, 1.0));
        let (offset, scale) = fit_cluster(20.0, 10.0);
        assert_eq!(offset, 0.0);
        assert!((scale - 0.5).abs() < f32::EPSILON);
        assert_eq!(fit_cluster(0.0, 10.0), (0.0, 1.0));
    }

    #[test]
    fn latin_cluster_is_painted_at_its_cell_slot() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 4, cell_h * 2);
        let snapshot = row_snapshot(vec![
            Cell {
                text: " ".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: " ".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "M".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: " ".into(),
                width: 1,
                style: Style::default(),
            },
        ]);
        renderer.draw(&snapshot, &[VisualMap::identity(4)], None);
        renderer.sync();
        let padding = renderer.padding();
        let columns = painted_columns(&renderer);
        assert!(!columns.is_empty());
        assert!(columns
            .iter()
            .all(|column| *column + 1 >= padding + 2 * cell_w && *column < padding + 3 * cell_w));
    }

    #[test]
    fn lam_alef_ligature_spans_two_cells_without_shifting_following_text() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 4, cell_h * 2);
        let snapshot = row_snapshot(vec![
            Cell {
                text: "ل".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "ا".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "x".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: " ".into(),
                width: 1,
                style: Style::default(),
            },
        ]);
        let maps = [VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        renderer.draw(&snapshot, &maps, None);
        renderer.sync();
        let start = renderer.padding();
        let columns = painted_columns(&renderer);
        let ligature: Vec<u32> = columns
            .iter()
            .copied()
            .filter(|column| *column >= start && *column < start + 2 * cell_w)
            .collect();
        assert!(
            !ligature.is_empty(),
            "the ligature must paint inside its two-cell slot: {columns:?}"
        );
        assert!(
            ligature.len() <= 2 * cell_w as usize,
            "the ligature must not overflow its two-cell slot: {ligature:?}"
        );
        assert!(
            columns
                .iter()
                .any(|column| *column >= start + 2 * cell_w && *column < start + 3 * cell_w),
            "the following glyph must stay in cell 2: {columns:?}"
        );
        assert!(
            columns.iter().all(|column| *column < start + 3 * cell_w),
            "nothing may paint past the row: {columns:?}"
        );
    }

    #[test]
    fn arabic_run_starts_next_to_preceding_latin_text() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        // Long enough that right-aligning the shaped run would leave several
        // empty cells between the Latin text and the Arabic.
        let arabic = "السلامةبليبتين";
        let mut cells = vec![Cell {
            text: "x".into(),
            width: 1,
            style: Style::default(),
        }];
        cells.extend(arabic.chars().map(|ch| Cell {
            text: ch.to_string(),
            width: 1,
            style: Style::default(),
        }));
        let cols = cells.len() as u32;
        renderer.resize(cell_w * (cols + 2), cell_h * 2);
        let snapshot = row_snapshot(cells);
        let maps = [VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        renderer.draw(&snapshot, &maps, None);
        renderer.sync();

        let columns = painted_columns(&renderer);
        let arabic_start = renderer.padding() + cell_w;
        assert!(
            columns
                .iter()
                .any(|column| (arabic_start..arabic_start + 2 * cell_w).contains(column)),
            "Arabic must begin within two cells of the Latin text: {columns:?}"
        );
    }

    #[test]
    fn arabic_run_keeps_its_natural_width_instead_of_filling_its_slots() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        let arabic = "السلامةبليبتين";
        let cells: Vec<Cell> = arabic
            .chars()
            .map(|ch| Cell {
                text: ch.to_string(),
                width: 1,
                style: Style::default(),
            })
            .collect();
        let cols = cells.len() as u32;
        renderer.resize(cell_w * (cols + 2), cell_h * 2);
        let snapshot = row_snapshot(cells);
        let map = VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr);
        let run = &map.runs()[0];
        let run_end = renderer.padding() + run.slots.end as u32 * cell_w;
        renderer.draw(&snapshot, &[map], None);
        renderer.sync();

        let columns = painted_columns(&renderer);
        let rightmost = *columns.last().expect("Arabic must paint");
        assert!(
            rightmost + cell_w < run_end,
            "Arabic must keep natural advances and leave at least a cell unused \
             at the run end (rightmost {rightmost}, run end {run_end}): {columns:?}"
        );
    }

    #[test]
    fn arabic_run_stays_right_pinned_in_rtl_base_text() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        let arabic = "السلامةبليبتين";
        let cells: Vec<Cell> = arabic
            .chars()
            .map(|ch| Cell {
                text: ch.to_string(),
                width: 1,
                style: Style::default(),
            })
            .collect();
        let cols = cells.len() as u32;
        renderer.resize(cell_w * (cols + 2), cell_h * 2);
        let snapshot = row_snapshot(cells);
        let map = VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Rtl);
        assert!(map.rtl_base());
        let run = &map.runs()[0];
        let run_start = renderer.padding() + run.slots.start as u32 * cell_w;
        renderer.draw(&snapshot, &[map], None);
        renderer.sync();

        let columns = painted_columns(&renderer);
        let leftmost = *columns.first().expect("Arabic must paint");
        assert!(
            leftmost > run_start + 2 * cell_w,
            "RTL text must leave its unused cells on the left, not squeeze to the \
             start (leftmost {leftmost}, run start {run_start}): {columns:?}"
        );
    }

    #[test]
    fn selection_fills_visual_slots_of_logical_cells() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 7, cell_h * 2);
        let snapshot = row_snapshot(vec![
            Cell {
                text: "a".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "b".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "c".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "م".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "ر".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: "ح".into(),
                width: 1,
                style: Style::default(),
            },
            Cell {
                text: " ".into(),
                width: 1,
                style: Style::default(),
            },
        ]);
        let mut selection = Selection::new(LogicalPos { row: 0, col: 4 });
        selection.update(LogicalPos { row: 0, col: 5 });
        let maps = [VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        renderer.draw(&snapshot, &maps, Some(&selection));
        renderer.sync();
        let selection_color = packed(renderer.theme.selection);
        let padding = renderer.padding();
        let width = renderer.pixmap.width() as usize;
        for slot in 0..7 {
            let x = padding + slot * cell_w + 1;
            let y = padding + 1;
            let pixel = renderer.pixels()[y as usize * width + x as usize];
            // Logical 4 and 5 land on visual slots 4 and 3: the reversed range
            // proves selection is filled by visual placement, not logical index.
            let selected = (3..5).contains(&slot);
            assert_eq!(
                pixel == selection_color,
                selected,
                "slot {slot} selection state"
            );
        }
    }

    #[test]
    fn block_selection_fills_a_rectangle() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        // One extra column of room keeps the padding outside the sampled cells.
        renderer.resize(cell_w * 4, cell_h * 2);
        let snapshot = GridSnapshot {
            cols: 3,
            rows: 2,
            lines: vec![
                RowSnapshot {
                    cells: vec![
                        Cell {
                            text: "a".into(),
                            width: 1,
                            style: Style::default(),
                        },
                        Cell {
                            text: "b".into(),
                            width: 1,
                            style: Style::default(),
                        },
                        Cell {
                            text: "c".into(),
                            width: 1,
                            style: Style::default(),
                        },
                    ],
                    wrapped: false,
                },
                RowSnapshot {
                    cells: vec![
                        Cell {
                            text: "d".into(),
                            width: 1,
                            style: Style::default(),
                        },
                        Cell {
                            text: "e".into(),
                            width: 1,
                            style: Style::default(),
                        },
                        Cell {
                            text: "f".into(),
                            width: 1,
                            style: Style::default(),
                        },
                    ],
                    wrapped: false,
                },
            ],
            cursor: CursorState {
                row: 2,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: false,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        };
        let maps: Vec<VisualMap> = (0..2).map(|_| VisualMap::identity(3)).collect();
        let mut selection = Selection::block(LogicalPos { row: 0, col: 0 });
        selection.update(LogicalPos { row: 1, col: 1 });
        renderer.draw(&snapshot, &maps, Some(&selection));
        renderer.sync();
        let padding = renderer.padding();
        let width = renderer.pixmap.width() as usize;
        let at = |x: u32, y: u32| renderer.pixels()[y as usize * width + x as usize];
        // Sample the cell corner so glyph ink never covers the selection fill.
        let selected = at(padding + 1, padding + 1);
        let outside_row = at(padding + 2 * cell_w + 1, padding + 1);
        let outside = at(padding + 2 * cell_w + 1, padding + cell_h + 1);
        assert_eq!(selected, packed(renderer.theme.selection));
        // Row 0 col 2 is inside a Simple selection but outside this rectangle.
        assert_ne!(outside_row, packed(renderer.theme.selection));
        assert_ne!(outside, packed(renderer.theme.selection));
    }

    #[test]
    fn cursor_is_a_filled_block() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let mut snapshot = single_cell_snapshot(Cell {
            text: " ".into(),
            width: 1,
            style: Style::default(),
        });
        snapshot.cursor.row = 0;
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let width = renderer.pixmap.width() as usize;
        let row = padding * width;
        assert_eq!(
            renderer.pixels()[row + padding],
            packed(renderer.theme.cursor)
        );
        assert_eq!(
            renderer.pixels()[row + padding + cell_w as usize - 1],
            packed(renderer.theme.cursor),
            "the block must span the whole cell, not a thin bar"
        );
    }

    #[test]
    fn hidden_cursor_paints_nothing() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = cursor_snapshot(CursorShape::Block, false);
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let index = padding * renderer.pixmap.width() as usize + padding;
        assert_eq!(renderer.pixels()[index], packed(renderer.theme.background));
    }

    #[test]
    fn beam_cursor_is_a_narrow_left_bar() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = cursor_snapshot(CursorShape::Beam, true);
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let width = renderer.pixmap.width() as usize;
        let row = padding * width;
        assert_eq!(
            renderer.pixels()[row + padding],
            packed(renderer.theme.cursor)
        );
        let beam_width = (cell_w as f32 / 8.0).max(1.0) as usize;
        let mid = padding + cell_h as usize / 2;
        assert_eq!(
            renderer.pixels()[mid * width + padding + beam_width - 1],
            packed(renderer.theme.cursor),
            "the beam must be exactly beam_width pixels wide"
        );
        assert_ne!(
            renderer.pixels()[mid * width + padding + beam_width],
            packed(renderer.theme.cursor),
            "nothing may paint beyond the beam width"
        );
    }

    #[test]
    fn underline_cursor_sits_on_the_bottom_edge() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = cursor_snapshot(CursorShape::Underline, true);
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let width = renderer.pixmap.width() as usize;
        let bottom = padding + cell_h as usize - 1;
        assert_eq!(
            renderer.pixels()[bottom * width + padding + 2],
            packed(renderer.theme.cursor)
        );
        assert_eq!(
            renderer.pixels()[(bottom - 1) * width + padding + 2],
            packed(renderer.theme.cursor),
            "the underline must be two pixels thick"
        );
        assert_eq!(
            renderer.pixels()[(bottom - 2) * width + padding + 2],
            packed(renderer.theme.background),
            "the third row from the bottom stays background"
        );
        assert_eq!(
            renderer.pixels()[padding * width + padding + 2],
            packed(renderer.theme.background)
        );
    }

    #[test]
    fn hollow_cursor_draws_an_outline() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = cursor_snapshot(CursorShape::HollowBlock, true);
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let width = renderer.pixmap.width() as usize;
        let right = padding + cell_w as usize - 1;
        let bottom = padding + cell_h as usize - 1;
        assert_eq!(
            renderer.pixels()[padding * width + padding],
            packed(renderer.theme.cursor)
        );
        assert_eq!(
            renderer.pixels()[bottom * width + right],
            packed(renderer.theme.cursor)
        );
        let center = (padding + cell_h as usize / 2) * width + padding + cell_w as usize / 2;
        assert_eq!(renderer.pixels()[center], packed(renderer.theme.background));
    }

    #[test]
    fn only_block_cursor_inverts_the_glyph() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let background = packed(renderer.theme.background);

        // Record every pixel the glyph paints over the cell background with the
        // cursor hidden, then require those same pixels to stay untouched under
        // a beam cursor and to change under a block cursor.
        let hidden = render_m_with_cursor(&mut renderer, CursorShape::Block, false);
        let glyph: Vec<usize> = hidden
            .iter()
            .enumerate()
            .filter(|(_, pixel)| **pixel != background)
            .map(|(index, _)| index)
            .collect();
        assert!(!glyph.is_empty(), "the M cell must paint ink");

        let block = render_m_with_cursor(&mut renderer, CursorShape::Block, true);
        assert!(
            glyph.iter().any(|index| block[*index] != hidden[*index]),
            "a block cursor must invert the glyph"
        );

        let beam = render_m_with_cursor(&mut renderer, CursorShape::Beam, true);
        // The beam bar itself overwrites the leftmost glyph pixel column, so
        // only pixels outside it must stay untouched.
        let padding = renderer.padding() as usize;
        let width = renderer.pixmap.width() as usize;
        let beam_w = (cell_w as f32 / 8.0).max(1.0) as usize;
        assert!(
            glyph
                .iter()
                .all(|index| beam[*index] == hidden[*index] || index % width < padding + beam_w),
            "a beam cursor must leave the glyph unchanged outside the bar"
        );
    }

    #[test]
    fn bidi_control_cells_paint_nothing() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = row_snapshot(vec![Cell {
            text: "\u{200F}".into(),
            width: 1,
            style: Style::default(),
        }]);
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        assert!(painted_rows(&renderer).is_empty());
    }

    #[test]
    fn inverse_cells_swap_background_and_foreground() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let style = Style {
            inverse: true,
            ..Style::default()
        };
        let cell = Cell {
            text: " ".into(),
            width: 1,
            style,
        };
        renderer.draw(&single_cell_snapshot(cell), &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let index = padding * renderer.pixmap.width() as usize + padding;
        assert_eq!(
            renderer.pixels()[index],
            packed(renderer.theme.foreground),
            "an inverse blank cell paints the foreground color"
        );
    }

    #[test]
    fn inverse_cells_paint_glyphs_and_decorations_in_the_swapped_color() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let fill = renderer.theme.indexed(1);
        let ink = renderer.theme.indexed(2);
        let style = Style {
            fg: Color::Indexed(1),
            bg: Some(Color::Indexed(2)),
            inverse: true,
            underline: true,
            ..Style::default()
        };
        let cell = Cell {
            text: "M".into(),
            width: 1,
            style,
        };
        renderer.draw(&single_cell_snapshot(cell), &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding() as usize;
        let width = renderer.pixmap.width() as usize;
        assert_eq!(
            renderer.pixels()[padding * width + padding],
            packed(fill),
            "an inverse cell fills with its foreground color"
        );
        assert!(
            renderer.pixels().iter().any(|pixel| *pixel == packed(ink)),
            "the glyph ink must use the swapped background color"
        );
        let underline_y = (renderer.padding() as f32 + renderer.fonts.ascent + 1.0)
            .min(renderer.padding() as f32 + cell_h as f32 - 1.0)
            .round() as usize;
        assert_eq!(
            renderer.pixels()[underline_y * width + padding],
            packed(ink),
            "decorations must use the swapped background color"
        );
    }

    #[test]
    fn hidden_cells_paint_no_glyph() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let style = Style {
            hidden: true,
            ..Style::default()
        };
        let cell = Cell {
            text: "M".into(),
            width: 1,
            style,
        };
        renderer.draw(&single_cell_snapshot(cell), &[VisualMap::identity(1)], None);
        renderer.sync();
        assert!(painted_rows(&renderer).is_empty());
    }

    #[test]
    fn underline_is_a_single_line_below_the_baseline() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let style = Style {
            underline: true,
            ..Style::default()
        };
        let cell = Cell {
            text: " ".into(),
            width: 1,
            style,
        };
        renderer.draw(&single_cell_snapshot(cell), &[VisualMap::identity(1)], None);
        renderer.sync();
        let rows = painted_rows(&renderer);
        assert!(!rows.is_empty());
        let first = rows[0];
        assert!(rows.iter().all(|row| *row == first), "one-pixel rule");
        let baseline = renderer.padding() as f32 + renderer.fonts.ascent;
        let expected = (baseline + 1.0).min(renderer.padding() as f32 + cell_h as f32 - 1.0);
        assert_eq!(first, expected.round() as usize);
    }

    #[test]
    fn strikethrough_is_a_single_line_across_the_cell() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let style = Style {
            strikethrough: true,
            ..Style::default()
        };
        let cell = Cell {
            text: " ".into(),
            width: 1,
            style,
        };
        renderer.draw(&single_cell_snapshot(cell), &[VisualMap::identity(1)], None);
        renderer.sync();
        let rows = painted_rows(&renderer);
        assert!(!rows.is_empty());
        let first = rows[0];
        assert!(rows.iter().all(|row| *row == first), "one-pixel rule");
        let expected = renderer.padding() as f32 + (cell_h as f32 / 2.0).round();
        assert_eq!(first, expected as usize);
    }

    #[test]
    fn scrollbar_thumb_is_painted() {
        let mut renderer = default_renderer();
        renderer.resize(200, 300);
        let snapshot = scroll_snapshot(10, 20, 5);
        let maps: Vec<_> = (0..snapshot.rows)
            .map(|_| VisualMap::identity(snapshot.cols))
            .collect();
        renderer.draw(&snapshot, &maps, None);
        renderer.sync();

        let geom = crate::scrollbar::geometry(
            renderer.pixmap.width(),
            renderer.padding,
            renderer.fonts.cell_h,
            snapshot.rows,
            snapshot.history,
            snapshot.display_offset,
            false,
        )
        .expect("history should show a scrollbar");
        let x = (geom.x + geom.width / 2.0).round() as u32;
        let y = (geom.thumb_y + geom.thumb_h / 2.0).round() as usize;
        let width = renderer.pixmap.width() as usize;
        assert_eq!(
            renderer.pixels()[y * width + x as usize],
            packed(renderer.theme.indexed(8))
        );
    }

    #[test]
    fn hovering_widens_the_scrollbar_thumb() {
        let mut renderer = default_renderer();
        renderer.resize(200, 300);
        let snapshot = scroll_snapshot(10, 20, 5);
        let maps: Vec<_> = (0..snapshot.rows)
            .map(|_| VisualMap::identity(snapshot.cols))
            .collect();
        renderer.set_scrollbar_hover(true);
        renderer.draw(&snapshot, &maps, None);
        renderer.sync();

        let rest = crate::scrollbar::geometry(
            renderer.pixmap.width(),
            renderer.padding,
            renderer.fonts.cell_h,
            snapshot.rows,
            snapshot.history,
            snapshot.display_offset,
            false,
        )
        .expect("history should show a scrollbar");
        let hovered = crate::scrollbar::geometry(
            renderer.pixmap.width(),
            renderer.padding,
            renderer.fonts.cell_h,
            snapshot.rows,
            snapshot.history,
            snapshot.display_offset,
            true,
        )
        .expect("history should show a scrollbar");
        let y = (hovered.thumb_y + hovered.thumb_h / 2.0).round() as usize;
        let width = renderer.pixmap.width() as usize;
        let left = hovered.x.round() as usize;
        // The wider thumb reaches left of where the resting thumb stops.
        assert!(left < rest.x.round() as usize);
        assert_eq!(
            renderer.pixels()[y * width + left],
            packed(renderer.theme.indexed(8))
        );
    }

    #[test]
    fn symbol_fallback_paints_a_missing_glyph() {
        use ab_glyph::Font;
        let mut renderer = default_renderer();
        if renderer.fonts.symbol_for("✕").is_none() {
            eprintln!("skipping: no symbol fallback font installed");
            return;
        }
        if renderer.fonts.font.glyph_id('✕').0 != 0 {
            eprintln!("skipping: the primary face already covers U+2715");
            return;
        }
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = single_cell_snapshot(Cell {
            text: "✕".into(),
            width: 1,
            style: Style::default(),
        });
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        let padding = renderer.padding();
        let columns = painted_columns(&renderer);
        assert!(
            !columns.is_empty(),
            "the fallback glyph must paint foreground ink"
        );
        assert!(
            columns
                .iter()
                .all(|column| *column >= padding && *column < padding + 2 * cell_w),
            "the glyph must paint in its own cell: {columns:?}"
        );

        // Without the symbol faces the same cell rasterizes differently, so the
        // ink above came from the fallback path rather than from .notdef.
        let with_fallback = renderer.pixels().to_vec();
        renderer.fonts.symbols.clear();
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        assert_ne!(
            with_fallback,
            renderer.pixels(),
            "the symbol face must change the raster"
        );
    }

    #[test]
    fn missing_trailing_glyphs_do_not_reuse_the_base_glyph() {
        use ab_glyph::Font;
        let mut with = default_renderer();
        if with.fonts.symbols.is_empty() {
            eprintln!("skipping: no symbol fallback font installed");
            return;
        }
        // The primary face must map the base digit but not the keycap mark, so
        // the shaped cluster has a missing trailing glyph.
        if with.fonts.font.glyph_id('1').0 == 0 || with.fonts.font.glyph_id('\u{20E3}').0 != 0 {
            eprintln!("skipping: the primary face lacks the keycap coverage this test needs");
            return;
        }
        if with.fonts.symbol_for("1").is_none() {
            eprintln!("skipping: no symbol face maps the cluster's base glyph");
            return;
        }
        let mut without = default_renderer();
        without.fonts.symbols.clear();

        let snapshot = single_cell_snapshot(Cell {
            text: "1\u{20E3}".into(),
            width: 1,
            style: Style::default(),
        });
        let cell_w = with.fonts.cell_w;
        let cell_h = with.fonts.cell_h;
        for renderer in [&mut with, &mut without] {
            renderer.resize(cell_w * 2, cell_h * 2);
            renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
            renderer.sync();
        }
        assert_eq!(
            with.pixels(),
            without.pixels(),
            "a missing trailing glyph must not paint the cluster's base glyph again"
        );
        assert!(
            !painted_columns(&with).is_empty(),
            "the base glyph must still paint"
        );
    }

    /// M5 performance gate (spec §11): a full 200x50 redraw must stay under
    /// 8 ms. Ignored by default because it measures rather than asserts
    /// behavior; run it in release:
    ///
    /// `cargo test --release full_frame_redraw -- --ignored --nocapture`
    #[test]
    #[ignore = "performance measurement; run with --release --ignored --nocapture"]
    fn full_frame_redraw_at_200x50_stays_within_budget() {
        use std::time::{Duration, Instant};

        let mut renderer = default_renderer();
        let cols = 200usize;
        let rows = 50usize;
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        let padding = renderer.padding();
        assert!(renderer.resize(
            cols as u32 * cell_w + 2 * padding,
            rows as u32 * cell_h + 2 * padding
        ));

        let text = "The quick brown fox jumps over the lazy dog. 0123456789 ";
        let line = RowSnapshot {
            cells: text
                .chars()
                .cycle()
                .take(cols)
                .map(|ch| Cell {
                    text: ch.to_string(),
                    width: 1,
                    style: Style::default(),
                })
                .collect(),
            wrapped: false,
        };
        let snapshot = GridSnapshot {
            cols,
            rows,
            lines: vec![line; rows],
            cursor: CursorState {
                row: rows - 1,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        };
        let maps = vec![VisualMap::identity(cols); rows];

        // Warm the glyph atlas so the measurement covers drawing, not discovery.
        renderer.draw(&snapshot, &maps, None);

        const FRAMES: usize = 40;
        let mut samples = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let start = Instant::now();
            renderer.draw(&snapshot, &maps, None);
            samples.push(start.elapsed());
        }
        samples.sort_unstable();
        let median = samples[FRAMES / 2];
        let fastest = samples[0];
        let slowest = samples[FRAMES - 1];
        println!("full frame 200x50: median {median:?}, fastest {fastest:?}, slowest {slowest:?}");
        // A shared CI box has scheduling spikes, so the fastest frame is the
        // honest measure of the renderer's own cost; the median has to hold too.
        assert!(
            fastest < Duration::from_millis(8),
            "a 200x50 redraw took {fastest:?} at best, the budget is 8 ms"
        );
        assert!(
            median < Duration::from_millis(8),
            "a 200x50 redraw took {median:?} at the median, the budget is 8 ms"
        );
    }
}
