//! Grid rasterization to the window surface.

mod atlas;
mod glyph_cache;

use ab_glyph::GlyphId;
use tiny_skia::Pixmap;

use crate::bidi;
use crate::fonts::FontSet;
use crate::selection::Selection;
use crate::shape;
use crate::term::{Color, GridSnapshot, RowSnapshot, Style};
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
    shape_cache: shape::ShapeCache,
    /// Whether the missing-shaping warning has already been logged.
    shape_warned: bool,
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
            shape_cache: shape::ShapeCache::new(),
            shape_warned: false,
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

        self.draw_cell_backgrounds(snapshot, maps, selection, cell_w, cell_h);

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
            for run in runs {
                self.draw_shaped_run(run, line, row, cell_w, cell_h);
            }
        }

        self.draw_decorations(snapshot, maps, cell_w, cell_h);

        if snapshot.display_offset == 0 {
            if let Some(map) = maps.get(snapshot.cursor.row) {
                let visual = map.cursor_visual(&snapshot.cursor);
                let x = padding + visual as f32 * cell_w;
                let y = padding + snapshot.cursor.row as f32 * cell_h;
                self.fill_rect(x, y, 2.0, cell_h, self.theme.cursor);
            }
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
            let y = self.padding as f32 + row as f32 * cell_h;
            for (logical, cell) in line.cells.iter().enumerate() {
                let x = self.padding as f32 + map.visual_of(logical) as f32 * cell_w;
                let (_, background) = self.cell_colors(cell.style);
                if cell.style.bg.is_some() || cell.style.inverse {
                    self.fill_rect(x, y, cell_w, cell_h, background);
                }
                if selection.is_some_and(|selection| selection.contains(row, logical)) {
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
        cell_w: f32,
        cell_h: f32,
    ) {
        let padding = self.padding as f32;
        // The cell grid comes from the primary face; a system Arabic fallback
        // with a taller ascent may paint a pixel or two above the cell top.
        let baseline = padding + row as f32 * cell_h + self.fonts.ascent;
        let units = run.units_per_em.max(1) as f32;
        let scale = self.fonts.size_px / units;
        let font_id = u8::from(run.arabic);
        let (font, _, _) = self.fonts.font_for(run.arabic);

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
            let (offset, scale_x) = fit_cluster(natural, slot_w);
            let mut pen = padding + slot_start as f32 * cell_w + offset;

            for glyph in &cluster.glyphs {
                // ab_glyph ids are u16; an out-of-range id becomes .notdef.
                let id = GlyphId(u16::try_from(glyph.glyph_id).unwrap_or(0));
                if let Some(slot) = self.cache.get(font_id, font, id, self.fonts.size_px) {
                    let x = pen + glyph.x_offset as f32 * scale * scale_x;
                    let y = baseline - glyph.y_offset as f32 * scale;
                    blit_scaled(
                        &mut self.pixmap,
                        &self.cache.atlas,
                        slot,
                        x,
                        y,
                        color,
                        scale_x,
                    );
                }
                pen += glyph.x_advance as f32 * scale * scale_x;
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
            let y = self.padding as f32 + row as f32 * cell_h;
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

fn blit_scaled(
    pixmap: &mut Pixmap,
    atlas: &Atlas,
    slot: GlyphSlot,
    pen_x: f32,
    baseline: f32,
    color: Rgb,
    scale_x: f32,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let stride = width as usize * 4;
    let dst_w = (slot.rect.w as f32 * scale_x).round().max(1.0) as i32;
    let dst_x = pen_x.round() as i32 + (slot.bearing_x as f32 * scale_x).round() as i32;
    let dst_y = baseline.round() as i32 + slot.bearing_y;
    let atlas_width = atlas.width();
    let data = pixmap.data_mut();
    for row in 0..slot.rect.h as i32 {
        for col in 0..dst_w {
            let x = dst_x + col;
            let y = dst_y + row;
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let src_col = (col as f32 / scale_x).floor().min((slot.rect.w - 1) as f32) as u32;
            let mask_x = slot.rect.x + src_col;
            let mask_y = slot.rect.y as i32 + row;
            let coverage = atlas.data()[(mask_y as u32 * atlas_width + mask_x) as usize];
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
    use crate::config::{BaseDirection, ThemeConfig};
    use crate::term::{Affinity, Cell, CursorState, LogicalPos, RowSnapshot, Style};

    fn default_renderer() -> Renderer {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("a monospace font must exist");
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4)
            .expect("renderer must be constructible")
    }

    fn arabic_renderer() -> Option<Renderer> {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
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
            },
            display_offset: 0,
        }
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
            },
            display_offset: 0,
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
        assert!(
            columns.iter().any(|column| *column < start + cell_w),
            "the ligature must paint in the first cell: {columns:?}"
        );
        assert!(
            columns
                .iter()
                .any(|column| *column >= start + cell_w && *column < start + 2 * cell_w),
            "the ligature must cross into the second cell: {columns:?}"
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
}
