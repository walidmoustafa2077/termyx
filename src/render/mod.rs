//! Grid rasterization to the window surface.

mod atlas;
mod glyph_cache;
mod menu;
mod ui_text;

use ab_glyph::GlyphId;
use tiny_skia::Pixmap;

use crate::bidi;
use crate::fonts::FontSet;
use crate::icon::TabIcon;
use crate::scrollbar;
use crate::selection::Selection;
use crate::shape;
use crate::term::{Color, CursorShape, GridSnapshot, RowSnapshot, Style};
use crate::theme::{Rgb, Theme};
use crate::ui::tab_bar::{
    Rect, StripHover, TabBarLayout, CLOSE_GLYPH_SIZE, TAB_ICON_GAP, TAB_ICON_SIZE, TAB_PADDING,
    TAB_STRIP_HEIGHT, TITLE_SIZE,
};
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
    /// Whether the window is maximised, which swaps the caption's maximise
    /// glyph for the restore glyph. The app reads this from the live window
    /// before each frame because the WM can maximise it without a click.
    maximized: bool,
    /// The bundled app icon, decoded once to fall back on for a tab whose
    /// profile has no icon of its own.
    app_icon: Option<TabIcon>,
}

/// The glyph the maximise control paints for the window's current state.
///
/// `❐` (U+2750, upper-right shadowed white square) reads as the two overlapping
/// windows Windows uses for "restore"; the fallback chain on this machine maps
/// it, and `the_window_control_glyphs_resolve_to_a_face_that_maps_them` pins
/// that. The plain `□` stays for the expand state.
fn maximize_glyph(maximized: bool) -> &'static str {
    if maximized {
        "❐"
    } else {
        "□"
    }
}

/// The fill and glyph colour for one caption control.
///
/// Split out of the painter so the hover/resting colour table is testable
/// without reading pixels. Only the control under the pointer is filled: the
/// close uses the accent red, the other two the strip's hover tone, and every
/// resting glyph uses the inactive title colour.
fn caption_colors(
    theme: &Theme,
    target: StripHover,
    hover: Option<StripHover>,
) -> (Option<Rgb>, Rgb) {
    let hovered = hover == Some(target);
    let fill = hovered.then(|| {
        if target == StripHover::CloseWindow {
            theme.caption_close_hover
        } else {
            theme.tab_hover
        }
    });
    let glyph = if hovered {
        theme.tab_title
    } else {
        theme.tab_title_inactive
    };
    (fill, glyph)
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
            maximized: false,
            app_icon: crate::icon::app_icon(),
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

    /// Paints the tab strip band described by `layout`. Runs after the grid so
    /// a window narrower than the strip never shows grid through it.
    ///
    /// `ui_px` is the physical chrome em size, already scaled by the window's
    /// DPI factor; `active` indexes the tab the window shows, `icons` carries
    /// one square image per tab, index-aligned, and `hover` is the strip target
    /// under the pointer, if any. A tab with no icon of its own stamps the
    /// bundled app icon instead, so the strip never shows a hole.
    pub fn draw_chrome(
        &mut self,
        layout: &TabBarLayout,
        ui_px: f32,
        active: usize,
        icons: &[Option<TabIcon>],
        hover: Option<StripHover>,
    ) {
        if layout.strip_h <= 0.0 || layout.width <= 0.0 {
            return;
        }
        self.fill_band(layout);
        self.draw_tabs(layout, active, hover);
        self.draw_buttons(layout, ui_px, hover);
        // The rule goes down after the buttons: a hovered button's fill starts
        // at the top of the strip, and painting it later would erase a window-
        // wide rule behind that button.
        self.draw_band_rule(layout, active);
        // The caption controls paint last. Their hover fill stops one pixel
        // above the strip's bottom edge, so painting them after the rule cannot
        // erase it, and the rule (already down) cannot cut through a control.
        self.draw_caption_buttons(layout, ui_px, hover);
        self.draw_titles(layout, ui_px, active, icons, hover);
    }

    /// The reserved band is chrome, not grid: it gets its own colour across
    /// the full width, whichever tabs happen to sit on it.
    fn fill_band(&mut self, layout: &TabBarLayout) {
        self.fill_rect(0.0, 0.0, layout.width, layout.strip_h, self.theme.tab_bar);
    }

    /// One fill per tab, then the active tab's outline, then the backdrop
    /// behind a hovered close button.
    ///
    /// The tab body floats on the strip: WT insets it by 8 px at the top and
    /// rounds its top corners, so the band shows above it. Only the active tab
    /// gets a fill and an outline — its fill is the terminal background, which
    /// makes it read as a cut-out — while an inactive tab paints nothing at
    /// all and lets the strip show through, exactly as WT does.
    fn draw_tabs(&mut self, layout: &TabBarLayout, active: usize, hover: Option<StripHover>) {
        let hovered = match hover {
            Some(StripHover::Tab(index)) => Some(index),
            _ => None,
        };
        let radius = 6.0 * layout.strip_h / TAB_STRIP_HEIGHT;
        for tab in &layout.tabs {
            let color = if tab.index == active {
                self.theme.tab_active
            } else if hovered == Some(tab.index) {
                self.theme.tab_hover
            } else {
                continue;
            };
            self.fill_round_top(tab.body, radius.min(tab.body.w / 2.0), color);
        }
        if let Some(tab) = layout.tabs.iter().find(|tab| tab.index == active) {
            self.stroke_tab(tab.body, radius.min(tab.body.w / 2.0));
        }
        self.draw_close_backdrop(layout, hovered);
    }

    /// The 1 px `#292929` outline of the active tab: its top edge and both
    /// sides. The bottom stays open so the tab meets the terminal flush, which
    /// is what makes it read as a cut-out rather than a card.
    fn stroke_tab(&mut self, rect: Rect, radius: f32) {
        let color = self.theme.tab_border;
        self.fill_rect(
            rect.x + radius,
            rect.y,
            (rect.w - 2.0 * radius).max(0.0),
            1.0,
            color,
        );
        let side = (rect.h - radius).max(0.0);
        self.fill_rect(rect.x, rect.y + radius, 1.0, side, color);
        self.fill_rect(rect.x + rect.w - 1.0, rect.y + radius, 1.0, side, color);
    }

    /// Paints the rounded square behind a `✕` while its tab is hovered, so the
    /// hit target reads as the button it already is. The active tab shows the
    /// glyph at rest; every tab shows the backdrop while under the pointer.
    fn draw_close_backdrop(&mut self, layout: &TabBarLayout, hover: Option<usize>) {
        let Some(index) = hover else {
            return;
        };
        let Some(tab) = layout.tabs.iter().find(|tab| tab.index == index) else {
            return;
        };
        if tab.close.w <= 0.0 {
            return;
        }
        let radius = 4.0 * layout.strip_h / TAB_STRIP_HEIGHT;
        self.fill_round_top(
            tab.close,
            radius.min(tab.close.w / 2.0),
            self.theme.tab_close_hover,
        );
    }

    /// The 1 px line that closes the band against the grid.
    ///
    /// It runs under every tab except the active one: that tab's background
    /// reaches the strip's bottom edge unbroken, which is how the reference
    /// makes the selected tab merge into the terminal.
    fn draw_band_rule(&mut self, layout: &TabBarLayout, active: usize) {
        let y = layout.strip_h - 1.0;
        let color = self.theme.tab_border;
        let active_span = layout
            .tabs
            .iter()
            .find(|tab| tab.index == active)
            .map(|tab| (tab.rect.x, tab.rect.x + tab.rect.w));
        match active_span {
            Some((left, right)) => {
                self.fill_rect(0.0, y, left, 1.0, color);
                self.fill_rect(right, y, layout.width - right, 1.0, color);
            }
            None => self.fill_rect(0.0, y, layout.width, 1.0, color),
        }
    }

    /// The `+` and `⌄` glyphs, centred in their buttons, with WT's single 1 px
    /// separator between them.
    ///
    /// Measured in WT, the only line on this run of strip is the boundary
    /// between `+` and `⌄`, painted `#3D3D3D` (`tab_separator`). There is no
    /// line at the `+` button's left edge, and none at the tab's right border
    /// beyond the tab's own `#292929` outline. The separator goes down after
    /// the hover fill so a hovered button keeps its edge.
    fn draw_buttons(&mut self, layout: &TabBarLayout, ui_px: f32, hover: Option<StripHover>) {
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let foreground = self.theme.foreground;
        let buttons = [
            ("+", layout.plus, StripHover::Plus),
            ("⌄", layout.chevron, StripHover::Chevron),
        ];
        for (glyph, rect, target) in buttons {
            if hover == Some(target) {
                self.fill_round_top(rect, 4.0 * k, self.theme.tab_hover);
            }
            ui_text::draw_text_centred(self, glyph, rect, ui_px, foreground);
        }
        // One separator only, at the `+`/`⌄` boundary. Measured `#3D3D3D`
        // there, so use `tab_separator`, not the tab's darker outline.
        self.fill_rect(
            layout.chevron.x,
            0.0,
            1.0,
            layout.strip_h,
            self.theme.tab_separator,
        );
    }

    /// The minimise, maximise and close controls at the strip's right end.
    ///
    /// Only the control under the pointer is filled: the close uses the accent
    /// red, the other two the strip's hover tone, and a resting control has no
    /// fill. The fill stops one pixel above the band rule so painting these
    /// after [`Renderer::draw_band_rule`] leaves it intact. The third control
    /// shows a restore glyph while the window is maximised: `□` there would
    /// claim that a click still expands an already-expanded window.
    fn draw_caption_buttons(
        &mut self,
        layout: &TabBarLayout,
        ui_px: f32,
        hover: Option<StripHover>,
    ) {
        let controls = [
            ("—", layout.minimize, StripHover::Minimize),
            (
                maximize_glyph(self.maximized),
                layout.maximize,
                StripHover::Maximize,
            ),
            ("✕", layout.close_window, StripHover::CloseWindow),
        ];
        for (glyph, rect, target) in controls {
            let (fill, color) = caption_colors(&self.theme, target, hover);
            if let Some(fill) = fill {
                self.fill_rect(rect.x, rect.y, rect.w, (rect.h - 1.0).max(0.0), fill);
            }
            ui_text::draw_text_centred(self, glyph, rect, ui_px, color);
        }
    }

    /// Tab titles, each with its own icon on the left and the close glyph on
    /// the active tab or whichever tab the pointer is over.
    ///
    /// Every tab stamps the icon of the shell it runs, so the strip identifies
    /// the program at a glance; a tab whose icon could not be read falls back
    /// to the bundled app icon. The inactive title has its own chrome grey
    /// rather than the grid's ANSI palette, so the tab on screen reads as the
    /// active one even when both tabs share a band.
    fn draw_titles(
        &mut self,
        layout: &TabBarLayout,
        ui_px: f32,
        active: usize,
        icons: &[Option<TabIcon>],
        hover: Option<StripHover>,
    ) {
        // One scale factor for the whole chrome budget, so the painter and
        // `TabBarLayout` cannot drift apart at another DPI.
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let tab_padding = TAB_PADDING * k;
        let icon_size = TAB_ICON_SIZE * k;
        let title_left = (TAB_PADDING + TAB_ICON_SIZE + TAB_ICON_GAP) * k;
        for tab in &layout.tabs {
            // A crowded strip can shrink a tab below the icon's own slot; the
            // blit would then spill into the neighbour, so skip it exactly as
            // the layout drops a close box that will not fit.
            let icon = icons
                .get(tab.index)
                .and_then(|icon| icon.as_ref())
                .or(self.app_icon.as_ref())
                .filter(|_| tab.rect.w >= tab_padding + icon_size);
            if let Some(icon) = icon {
                let icon_x = tab.rect.x + tab_padding;
                // Content is positioned on the tab body, not the band: WT's
                // icon sits centred in its 32 px card.
                let icon_y = tab.body.y + (tab.body.h - icon_size) / 2.0;
                blit_tab_icon(&mut self.pixmap, icon, icon_x, icon_y, icon_size);
            }
            let title = layout.titles.get(tab.index).map_or("", String::as_str);
            // A tab is sized as its title plus `TITLE_SIZE`, so the title owns
            // exactly what the chrome budget leaves.
            let title_width = (tab.rect.w - TITLE_SIZE * k).max(0.0);
            let color = if tab.index == active {
                self.theme.tab_title
            } else {
                self.theme.tab_title_inactive
            };
            // 9 px below the body's top edge puts the 14 px ink on the line WT
            // measured at y=20..29 inside the tab body.
            let title_top = tab.body.y + 9.0 * k;
            ui_text::draw_text_ellipsis(
                self,
                title,
                tab.rect.x + title_left,
                title_top,
                ui_px,
                color,
                title_width,
            );
            let shows_close = tab.index == active || hover == Some(StripHover::Tab(tab.index));
            if shows_close && tab.close.w > 0.0 {
                ui_text::draw_text_centred(
                    self,
                    "✕",
                    tab.close,
                    CLOSE_GLYPH_SIZE * k,
                    self.theme.tab_close,
                );
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

    /// Fills `rect` with its top two corners rounded by `radius`, leaving the
    /// bottom corners square so a tab meets the strip's rule flush.
    fn fill_round_top(&mut self, rect: Rect, radius: f32, color: Rgb) {
        let radius = radius.min(rect.w / 2.0).min(rect.h).max(0.0);
        for dy in 0..radius.round() as i32 {
            // Distance from the corner circle's centre to this row's middle.
            let y = radius - dy as f32 - 0.5;
            let inset = radius - (radius * radius - y * y).max(0.0).sqrt();
            self.fill_rect(
                rect.x + inset,
                rect.y + dy as f32,
                (rect.w - 2.0 * inset).max(1.0),
                1.0,
                color,
            );
        }
        self.fill_rect(
            rect.x,
            rect.y + radius,
            rect.w,
            (rect.h - radius).max(0.0),
            color,
        );
    }

    pub fn fonts(&self) -> &FontSet {
        &self.fonts
    }

    /// Swaps in a font set rasterized for a new display scale factor.
    ///
    /// The glyph atlas is keyed by pixel size, so it is dropped: reusing it
    /// would blit glyphs rasterized for the previous DPI. The caller must also
    /// reset the top inset and resize, because both are physical sizes too.
    pub fn set_fonts(&mut self, fonts: FontSet) {
        self.fonts = fonts;
        self.cache = GlyphCache::new();
    }

    /// Font data for a glyph-cache face id.
    pub(crate) fn fonts_face(&self, id: u8) -> (&ab_glyph::FontVec, u32) {
        self.fonts.font_for_id(id)
    }

    pub(crate) fn cache_get(&mut self, font: u8, id: GlyphId, size_px: u32) -> Option<GlyphSlot> {
        let (face, _index) = self.fonts.font_for_id(font);
        self.cache.get(font, face, id, size_px as f32)
    }

    pub(crate) fn blit(&mut self, slot: GlyphSlot, x: f32, y: f32, color: Rgb) {
        blit_scaled(
            &mut self.pixmap,
            &self.cache.atlas,
            slot,
            x,
            y,
            Ink { color, bold: false },
            1.0,
        );
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

    /// Marks whether the window is maximised. The caption's third-state
    /// distinction is a glyph, not a colour, so only the painter needs it.
    pub fn set_maximized(&mut self, maximized: bool) {
        self.maximized = maximized;
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

/// Stamps `icon` into `pixmap` as a `size` px square at `(x, y)`, box-averaging
/// each source block so a 256 px icon stays smooth at 16 px.
///
/// Free rather than a method: the icon is stored on the renderer, and reading
/// it through `&self` while the pixmap needs `&mut self` cannot borrow-check.
fn blit_tab_icon(pixmap: &mut Pixmap, icon: &TabIcon, x: f32, y: f32, size: f32) {
    let side = size.round().max(1.0) as u32;
    let src = icon.size.max(1);
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let dst_x = x.round() as i32;
    let dst_y = y.round() as i32;
    let data = pixmap.data_mut();
    for row in 0..side {
        let py = dst_y + row as i32;
        if py < 0 || py >= height {
            continue;
        }
        // The blocks partition the source, and the slot is a small fraction of
        // the icon, so every block holds at least one row and column.
        let sy0 = (row as usize * src / side as usize).min(src - 1);
        let sy1 = ((row as usize + 1) * src / side as usize)
            .max(sy0 + 1)
            .min(src);
        for col in 0..side {
            let px = dst_x + col as i32;
            if px < 0 || px >= width {
                continue;
            }
            let sx0 = (col as usize * src / side as usize).min(src - 1);
            let sx1 = ((col as usize + 1) * src / side as usize)
                .max(sx0 + 1)
                .min(src);
            // Average premultiplied colour so fully transparent pixels do not
            // darken the edge they are blended into.
            let mut channels = [0u32; 3];
            let mut alpha = 0u32;
            let mut count = 0u32;
            for sy in sy0..sy1 {
                for sx in sx0..sx1 {
                    let index = (sy * src + sx) * 4;
                    let a = u32::from(icon.rgba[index + 3]);
                    for (channel, value) in channels.iter_mut().zip(&icon.rgba[index..index + 3]) {
                        *channel += u32::from(*value) * a;
                    }
                    alpha += a;
                    count += 1;
                }
            }
            if alpha == 0 {
                continue;
            }
            let coverage = (alpha + count / 2) / count;
            if coverage == 0 {
                continue;
            }
            let index = (py as usize * width as usize + px as usize) * 4;
            for (offset, channel) in channels.iter().enumerate() {
                data[index + offset] =
                    blend_u8(data[index + offset], (channel / alpha) as u8, coverage);
            }
        }
    }
}

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

    /// Shorthand for the strip cases that inspect the renderer's own fallback
    /// icon: they pass no per-tab icon and expect the bundled one.
    fn draw_chrome(
        renderer: &mut Renderer,
        layout: &TabBarLayout,
        ui_px: f32,
        active: usize,
        hover: Option<StripHover>,
    ) {
        renderer.draw_chrome(layout, ui_px, active, &[], hover);
    }

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
    fn the_active_tab_is_the_body_colour_while_its_neighbour_is_the_band() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);

        // Both probes sit inside the tab body, below the rounded top corners.
        let active = pixel_at(&renderer, layout.tabs[0].body.x + 4.0, 16.0);
        let inactive = pixel_at(&renderer, layout.tabs[1].body.x + 4.0, 16.0);
        assert_eq!(active, renderer.theme.tab_active);
        assert_eq!(inactive, renderer.theme.tab_bar);
        assert_ne!(active, inactive);
    }

    #[test]
    fn an_inactive_tab_title_is_dimmer_than_the_active_one() {
        let mut renderer = default_renderer();
        renderer.resize(600, 120);
        // The same label on both tabs makes the glyph peaks identical, so only
        // the title colour can explain a difference between them.
        let layout = TabBarLayout::new(600.0, TAB_STRIP_HEIGHT, &["one", "one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);

        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let title_rect = |index: usize| {
            let tab = &layout.tabs[index];
            crate::ui::tab_bar::Rect {
                x: tab.body.x + 34.0 * k,
                y: tab.body.y + 9.0 * k,
                w: (tab.body.w - 34.0 * k - 24.0 * k).max(0.0),
                h: 16.0 * k,
            }
        };
        let active = brightest_red(&renderer, title_rect(0));
        let inactive = brightest_red(&renderer, title_rect(1));
        // Both tabs paint the same glyph at the same offset, so the inactive
        // peak reveals the coverage of the peak pixel; the active peak must then
        // be that same coverage of its own chrome colour over the active tab's
        // fill. If the active title reused the inactive token, the two peaks
        // would be equal instead.
        let theme = &renderer.theme;
        let muted = theme.tab_title_inactive.r;
        let coverage =
            (inactive as f32 - theme.tab_bar.r as f32) / (muted as f32 - theme.tab_bar.r as f32);
        assert!(coverage > 0.0, "the inactive title must leave some ink");
        let expected = blend(theme.tab_active.r, theme.tab_title.r, coverage) as f32;
        assert!(
            (active as f32 - expected).abs() <= 2.0,
            "the active title must paint its chrome white over the active tab: \
             expected about {expected}, got {active} (the inactive token would peak near {muted})"
        );
        assert!(
            theme.tab_title.r > theme.tab_title_inactive.r,
            "the active title must be the brighter of the two tokens"
        );
    }

    #[test]
    fn a_border_line_splits_adjacent_tabs() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let boundary = layout.tabs[1].rect.x;
        // The active tab owns the outline, so its right edge is the last pixel
        // before its neighbour; the inactive tab starts flush at `boundary`
        // with no edge of its own.
        assert_eq!(
            pixel_at(&renderer, boundary - 1.0, 16.0),
            renderer.theme.tab_border
        );
        assert_eq!(
            pixel_at(&renderer, boundary, 16.0),
            renderer.theme.tab_bar,
            "an inactive tab must not paint a divider of its own"
        );
    }

    #[test]
    fn hovering_the_active_tab_paints_a_backdrop_behind_the_close_button() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        let close = layout.tabs[0].close;
        // A corner the `✕` glyph itself does not cover.
        let square = (close.x + 1.0, close.y + 1.0);

        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        assert_ne!(
            pixel_at(&renderer, square.0, square.1),
            renderer.theme.tab_close_hover,
            "the close button has no backdrop when the pointer is away"
        );

        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Tab(0)),
        );
        assert_eq!(
            pixel_at(&renderer, square.0, square.1),
            renderer.theme.tab_close_hover,
            "hovering the active tab must fill the square with its own colour"
        );
        assert_eq!(
            count_color(&renderer, close, renderer.theme.tab_hover),
            0,
            "the square must not reuse the tab-body hover colour"
        );
    }

    /// Brightest red channel inside `rect`. Comparing peak ink rather than a
    /// single pixel makes the title-colour test independent of antialiasing
    /// coverage and glyph side bearings.
    fn brightest_red(renderer: &Renderer, rect: crate::ui::tab_bar::Rect) -> u8 {
        (rect.y as u32..(rect.y + rect.h) as u32)
            .flat_map(|y| (rect.x as u32..(rect.x + rect.w) as u32).map(move |x| (x, y)))
            .map(|(x, y)| pixel_at(renderer, x as f32, y as f32).r)
            .max()
            .unwrap_or(0)
    }

    fn count_color(renderer: &Renderer, rect: crate::ui::tab_bar::Rect, color: Rgb) -> usize {
        (rect.y as u32..(rect.y + rect.h) as u32)
            .flat_map(|y| (rect.x as u32..(rect.x + rect.w) as u32).map(move |x| (x, y)))
            .filter(|(x, y)| pixel_at(renderer, *x as f32, *y as f32) == color)
            .count()
    }

    #[test]
    fn the_plus_button_paints_a_glyph() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let rect = layout.plus;
        let bar = renderer.theme.tab_bar;
        let border = renderer.theme.tab_border;
        let separator = renderer.theme.tab_separator;
        // The band behind the button is its own colour, and the rect carries a
        // divider and the bottom rule; ignore all three so only glyph ink can
        // make the assertion pass.
        let painted = (0..rect.w as u32).any(|dx| {
            (0..rect.h as u32).any(|dy| {
                let pixel = pixel_at(&renderer, rect.x + dx as f32, rect.y + dy as f32);
                pixel != bar && pixel != border && pixel != separator
            })
        });
        assert!(painted);
    }

    #[test]
    fn the_plus_and_chevron_fill_with_the_hover_tone() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        // The strip's hover fill is `#1f1f1f`; pin the literal, not the theme
        // field, so a palette rename cannot silently drop it.
        let hover = Rgb::new(0x1f, 0x1f, 0x1f);
        // A corner inside the button, clear of the glyph inset and the rule.
        let plus = (layout.plus.x + 2.0, 2.0);
        let chevron = (layout.chevron.x + 2.0, 2.0);

        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Plus),
        );
        assert_eq!(
            pixel_at(&renderer, plus.0, plus.1),
            hover,
            "a hovered + must fill with the strip's hover tone"
        );
        assert_eq!(
            pixel_at(&renderer, chevron.0, chevron.1),
            renderer.theme.tab_bar,
            "the un-hovered chevron must stay flat"
        );

        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Chevron),
        );
        assert_eq!(
            pixel_at(&renderer, chevron.0, chevron.1),
            hover,
            "a hovered ⌄ must fill with the strip's hover tone"
        );
        assert_eq!(
            pixel_at(&renderer, plus.0, plus.1),
            renderer.theme.tab_bar,
            "leaving + must clear its fill"
        );
    }

    #[test]
    fn the_strip_band_leaves_no_unpainted_pixel() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let sentinel = Rgb::new(255, 0, 255);
        renderer.pixmap.fill(sentinel.to_tiny());
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Tab(1)),
        );
        for y in 0..layout.strip_h as u32 {
            for x in 0..layout.width as u32 {
                assert_ne!(
                    pixel_at(&renderer, x as f32, y as f32),
                    sentinel,
                    "the strip must paint every pixel at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn the_strip_ends_with_a_border_rule() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        // Under the `+`, clear of the active tab's open bottom edge and of the
        // hover fill the button only paints under the pointer.
        assert_eq!(
            pixel_at(&renderer, layout.plus.x + 1.0, layout.strip_h - 1.0),
            renderer.theme.tab_border
        );
    }

    #[test]
    fn the_active_tab_keeps_an_open_bottom_edge() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let tab = layout.tabs[0].body;
        // The reference tab is raised: top and both sides are stroked, but its
        // bottom edge stays the tab's own fill so it merges into the terminal.
        assert_eq!(
            pixel_at(&renderer, tab.x + 40.0, tab.y),
            renderer.theme.tab_border,
            "the top edge must be stroked"
        );
        assert_eq!(
            pixel_at(&renderer, tab.x, tab.y + 16.0),
            renderer.theme.tab_border,
            "the left edge must be stroked"
        );
        assert_eq!(
            pixel_at(&renderer, tab.x + tab.w - 1.0, tab.y + 16.0),
            renderer.theme.tab_border,
            "the right edge must be stroked"
        );
        for x in tab.x as u32 + 1..(tab.x + tab.w) as u32 - 1 {
            assert_eq!(
                pixel_at(&renderer, x as f32, layout.strip_h - 1.0),
                renderer.theme.tab_active,
                "the active tab's bottom edge must not be stroked at column {x}"
            );
        }
    }

    #[test]
    fn an_inactive_tab_has_no_outline_and_no_close_ink() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let inactive = &layout.tabs[1];
        // No stroke anywhere on an inactive tab: it is flat strip colour.
        for (x, y) in [
            (inactive.rect.x + inactive.rect.w / 2.0, inactive.rect.y),
            (inactive.rect.x, inactive.rect.y + 16.0),
            (
                inactive.rect.x + inactive.rect.w - 1.0,
                inactive.rect.y + 16.0,
            ),
        ] {
            assert_eq!(
                pixel_at(&renderer, x, y),
                renderer.theme.tab_bar,
                "an inactive tab must not be outlined at ({x}, {y})"
            );
        }
        // Its `✕` is only painted while the tab is hovered, so the close box
        // holds nothing but the tab's own background.
        assert_eq!(
            count_color(&renderer, inactive.close, renderer.theme.tab_bar),
            (inactive.close.w * inactive.close.h) as usize,
            "an inactive, un-hovered close box must hold its background only"
        );
    }

    #[test]
    fn no_divider_stands_between_the_last_tab_and_the_plus() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        // Both tabs are inactive; WT puts a divider on the `+` button's own left
        // edge, not in the band between the last tab and the button, so this
        // column is plain band.
        assert_eq!(
            pixel_at(&renderer, layout.plus.x - 1.0, 16.0),
            renderer.theme.tab_bar
        );
    }

    #[test]
    fn the_strip_rule_survives_a_hovered_button() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Plus),
        );
        let y = layout.strip_h - 1.0;
        for x in layout.plus.x as u32..(layout.plus.x + layout.plus.w) as u32 {
            assert_eq!(
                pixel_at(&renderer, x as f32, y),
                renderer.theme.tab_border,
                "a hovered + erased the band rule at column {x}"
            );
        }
    }

    #[test]
    fn the_window_control_glyphs_resolve_to_a_face_that_maps_them() {
        // The three caption controls go through the same per-character resolver
        // as the tab strip's `✕`/`⌄`; an unmapped glyph would paint `.notdef`.
        use ab_glyph::Font;

        let renderer = default_renderer();
        for ch in ['—', '□', '❐', '✕'] {
            let (face, glyph) = renderer.fonts().chrome_glyph(ch);
            let (font, _index) = renderer.fonts_face(face);
            assert_ne!(glyph.0, 0, "the resolved glyph for {ch:?} must exist");
            assert_ne!(
                font.glyph_id(ch).0,
                0,
                "face {face} must rasterize {ch:?}, not .notdef"
            );
            assert!(
                renderer
                    .fonts()
                    .ui_width(&ch.to_string(), crate::fonts::UI_SIZE_PX)
                    > 0.0,
                "{ch:?} must have a positive advance"
            );
        }
    }

    #[test]
    fn the_maximise_control_paints_a_distinct_restore_glyph() {
        // The control must not claim "expand" while the window is already
        // maximised. Compare the whole control cell rather than one pixel: the
        // glyph's centre can be background for both faces.
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        let painted = |renderer: &mut Renderer, maximized: bool| {
            renderer.set_maximized(maximized);
            draw_chrome(renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
            let rect = layout.maximize;
            let mut pixels = Vec::new();
            for y in rect.y as u32..(rect.y + rect.h) as u32 {
                for x in rect.x as u32..(rect.x + rect.w) as u32 {
                    pixels.push(pixel_at(renderer, x as f32, y as f32));
                }
            }
            pixels
        };

        assert_ne!(
            painted(&mut renderer, false),
            painted(&mut renderer, true),
            "a maximised window must paint the restore glyph, not `□`"
        );
    }

    #[test]
    fn a_hovered_caption_button_fills_and_leaves_the_band_rule() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::CloseWindow),
        );
        let close = layout.close_window;
        // A corner clear of the centred glyph and of the bottom rule.
        assert_eq!(
            pixel_at(&renderer, close.x + 2.0, 2.0),
            renderer.theme.caption_close_hover,
            "the close control must fill with the accent red while hovered"
        );
        assert_eq!(
            pixel_at(&renderer, close.x + 2.0, close.h - 1.0),
            renderer.theme.tab_border,
            "the hover fill must stop above the band rule"
        );

        // The other two use the strip's own hover tone, and leaving the
        // controls clears their fill.
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Minimize),
        );
        assert_eq!(
            pixel_at(&renderer, layout.minimize.x + 2.0, 2.0),
            renderer.theme.tab_hover,
            "a hovered minimise control uses the strip hover tone"
        );
        assert_eq!(
            pixel_at(&renderer, close.x + 2.0, 2.0),
            renderer.theme.tab_bar,
            "the close fill must clear when the pointer leaves it"
        );
    }

    #[test]
    fn caption_control_colours_follow_the_hover_state() {
        // One row per control: hover fill, then the same token pair a resting
        // and a hovered glyph must use. The close is the only red fill.
        let theme = Theme::from_config(&ThemeConfig::default());
        let table = [
            (StripHover::Minimize, theme.tab_hover),
            (StripHover::Maximize, theme.tab_hover),
            (StripHover::CloseWindow, theme.caption_close_hover),
        ];
        assert_ne!(theme.caption_close_hover, theme.tab_hover);
        for (target, hover_fill) in table {
            let (fill, glyph) = caption_colors(&theme, target, None);
            assert_eq!(fill, None, "a resting control paints no fill");
            assert_eq!(glyph, theme.tab_title_inactive);

            let (fill, glyph) = caption_colors(&theme, target, Some(target));
            assert_eq!(fill, Some(hover_fill));
            assert_eq!(glyph, theme.tab_title);

            // Another target's hover must leave this control resting.
            let (fill, glyph) = caption_colors(&theme, target, Some(StripHover::Tab(0)));
            assert_eq!(fill, None);
            assert_eq!(glyph, theme.tab_title_inactive);
        }
    }

    #[test]
    fn a_tab_without_a_close_box_paints_no_close() {
        let mut renderer = default_renderer();
        renderer.resize(100, 120);
        // 18 px per tab: narrower than the padding plus the close box, so the
        // layout drops the target and the painter must drop the square and glyph
        // with it instead of bleeding into the neighbour.
        let layout = TabBarLayout::new(100.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        assert_eq!(layout.tabs[0].close.w, 0.0, "the layout must drop the box");
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Tab(0)),
        );
        let tab = layout.tabs[0].rect;
        assert_eq!(
            count_color(&renderer, tab, renderer.theme.tab_close_hover),
            0,
            "no backdrop may paint where there is no close target"
        );
        assert_eq!(
            count_color(&renderer, tab, renderer.theme.tab_close),
            0,
            "no glyph may paint where there is no close target"
        );
    }

    #[test]
    fn a_hovered_inactive_tab_uses_the_hover_colour() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Tab(1)),
        );
        let tab = layout.tabs[1].rect;
        // The right padding is clear of the title and, since tab 1 is not
        // active, of any close glyph.
        assert_eq!(
            pixel_at(&renderer, tab.x + tab.w - 4.0, 16.0),
            renderer.theme.tab_hover
        );
    }

    #[test]
    fn the_active_tab_close_box_paints_ink() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let close = layout.tabs[0].close;
        let background = renderer.theme.tab_active;
        let painted = (0..close.h as u32).any(|dy| {
            (0..close.w as u32).any(|dx| {
                pixel_at(&renderer, close.x + dx as f32, close.y + dy as f32) != background
            })
        });
        assert!(
            painted,
            "the active close button must paint its glyph over the tab"
        );
    }

    #[test]
    fn the_title_clip_stops_before_the_close_box() {
        // One tab gets `(318 - 64 - 138 - 24 - 4 - 8) / 1`: the width left
        // after the buttons, the caption controls, the drag gutter, the gap
        // before `+` and the strip's left margin. That is wide enough to show a
        // title and its ellipsis, though the title itself is far longer than
        // the tab can hold.
        let layout = TabBarLayout::new(
            318.0,
            TAB_STRIP_HEIGHT,
            &["a very long tab title that cannot fit"],
        );
        let tab = layout.tabs[0].rect;
        assert_eq!(tab.w, 80.0);
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let title_left = tab.x + (TAB_PADDING + TAB_ICON_SIZE + TAB_ICON_GAP) * k;
        let title_width = (tab.w - TITLE_SIZE * k).max(0.0);
        assert!(
            title_left + title_width <= layout.tabs[0].close.x,
            "the title clip ends at {}, inside the close box that starts at {}",
            title_left + title_width,
            layout.tabs[0].close.x
        );
    }

    #[test]
    fn an_inactive_tab_shows_its_close_glyph_only_when_hovered() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let plain = pixel_at(&renderer, layout.tabs[1].close.x + 8.0, 16.0);
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(crate::ui::tab_bar::StripHover::Tab(1)),
        );
        let hovered = pixel_at(&renderer, layout.tabs[1].close.x + 3.0, 16.0);
        assert_eq!(plain, renderer.theme.tab_bar);
        assert_ne!(hovered, renderer.theme.tab_bar);
    }

    /// Ink bounding box inside `rect`, as `(left, top, right, bottom)` in
    /// physical pixels, excluding any pixel equal to `background`.
    ///
    /// Centring tests compare this against the rect's centre instead of a
    /// single pixel, so antialiasing and glyph side bearings cannot mask a
    /// glyph that sits too high or too low.
    fn ink_bounds(
        renderer: &Renderer,
        rect: crate::ui::tab_bar::Rect,
        background: Rgb,
    ) -> (f32, f32, f32, f32) {
        let (mut left, mut top, mut right, mut bottom) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for dy in 0..rect.h as u32 {
            for dx in 0..rect.w as u32 {
                let x = rect.x + dx as f32;
                let y = rect.y + dy as f32;
                if pixel_at(renderer, x, y) != background {
                    left = left.min(x);
                    top = top.min(y);
                    right = right.max(x);
                    bottom = bottom.max(y);
                }
            }
        }
        (left, top, right, bottom)
    }

    /// Asserts the ink in `rect` is centred on both axes to within one pixel.
    fn assert_centred(renderer: &Renderer, rect: crate::ui::tab_bar::Rect, background: Rgb) {
        let (left, top, right, bottom) = ink_bounds(renderer, rect, background);
        assert!(left <= right, "the glyph must paint ink");
        let ink_x = (left + right) / 2.0;
        let ink_y = (top + bottom) / 2.0;
        let box_x = rect.x + rect.w / 2.0;
        let box_y = rect.y + rect.h / 2.0;
        assert!(
            (ink_x - box_x).abs() <= 1.0,
            "ink centre x {ink_x} must sit within 1 px of the box centre {box_x}"
        );
        assert!(
            (ink_y - box_y).abs() <= 1.0,
            "ink centre y {ink_y} must sit within 1 px of the box centre {box_y}"
        );
    }

    #[test]
    fn the_plus_glyph_is_vertically_centred() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        // The band rule crosses the button's last row and the divider runs down
        // its first column; both are chrome, not glyph ink, so measure the
        // button without them.
        let button = crate::ui::tab_bar::Rect {
            x: layout.plus.x + 1.0,
            w: layout.plus.w - 1.0,
            h: layout.plus.h - 1.0,
            ..layout.plus
        };
        assert_centred(&renderer, button, renderer.theme.tab_bar);
    }

    #[test]
    fn the_chevron_glyph_is_vertically_centred() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let button = crate::ui::tab_bar::Rect {
            x: layout.chevron.x + 1.0,
            w: layout.chevron.w - 1.0,
            h: layout.chevron.h - 1.0,
            ..layout.chevron
        };
        assert_centred(&renderer, button, renderer.theme.tab_bar);
    }

    #[test]
    fn the_close_glyph_is_vertically_centred() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        assert_centred(&renderer, layout.tabs[0].close, renderer.theme.tab_active);
    }

    #[test]
    fn the_close_glyph_is_painted_at_the_small_chrome_size() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let close = layout.tabs[0].close;

        // Paint the same cross at the title size on the same fill. Comparing
        // painted ink with painted ink keeps antialiasing and the glyph's side
        // bearings out of the comparison: only the smaller em size makes the
        // real `✕` narrower.
        let mut reference = default_renderer();
        reference.resize(400, 120);
        reference.pixmap.fill(reference.theme.tab_active.to_tiny());
        let reference_color = reference.theme.tab_title;
        ui_text::draw_text_centred(
            &mut reference,
            "✕",
            close,
            crate::fonts::UI_SIZE_PX,
            reference_color,
        );
        let ink_width = |renderer: &Renderer| {
            let (left, _, right, _) = ink_bounds(renderer, close, renderer.theme.tab_active);
            right - left + 1.0
        };
        let painted = ink_width(&renderer);
        let at_title_size = ink_width(&reference);
        assert!(
            painted < at_title_size,
            "the ✕ ink must be narrower than at the title size ({painted} vs {at_title_size} px)"
        );
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let (left, _, right, _) = renderer
            .fonts()
            .chrome_ink_bounds("✕", CLOSE_GLYPH_SIZE * k)
            .expect("the close glyph must have an outline");
        assert!(
            (painted - (right - left)).abs() <= 1.0,
            "the painted ✕ must match the {CLOSE_GLYPH_SIZE} px ink width"
        );
    }

    #[test]
    fn the_close_glyph_uses_the_tab_close_grey() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let close = layout.tabs[0].close;
        let peak = brightest_red(&renderer, close);
        // WT paints the `✕` grey, never white. The peak may fall a little short
        // of the token if no pixel reaches full coverage, so allow a few units
        // below it but never above.
        assert!(
            peak <= renderer.theme.tab_close.r + 1,
            "the ✕ must not be brighter than the tab-close grey, got {peak}"
        );
        assert!(
            peak > renderer.theme.tab_bar.r,
            "the ✕ must still stand out on the #0C0C0C tab, got {peak}"
        );
        assert!(renderer.theme.tab_close.r < renderer.theme.tab_title.r);
    }

    #[test]
    fn only_the_active_tab_has_its_top_and_side_borders() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        // The second tab is the one the window shows, so the outline follows
        // the active index rather than the leftmost tab.
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 1, None);
        let tab = layout.tabs[1].body;
        let border = renderer.theme.tab_border;
        assert_eq!(
            pixel_at(&renderer, tab.x + 50.0, tab.y),
            border,
            "the active tab's top edge must be stroked"
        );
        assert_eq!(
            pixel_at(&renderer, tab.x, 16.0),
            border,
            "the active tab's left edge must be stroked"
        );
        assert_eq!(
            pixel_at(&renderer, tab.x + tab.w - 1.0, 16.0),
            border,
            "the active tab's right edge must be stroked"
        );
    }

    #[test]
    fn a_tab_has_rounded_top_corners() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let tab = layout.tabs[0].body;
        // The corner falls outside the rounded fill, so the band shows through
        // instead of the active tab's own colour.
        assert_eq!(
            pixel_at(&renderer, tab.x, tab.y),
            renderer.theme.tab_bar,
            "the top-left corner must be rounded away"
        );
    }

    #[test]
    fn the_strip_matches_the_windows_terminal_measurements() {
        let mut renderer = default_renderer();
        renderer.resize(1200, 120);
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let theme = &renderer.theme;
        let body = layout.tabs[0].body;
        let centre_x = body.x + body.w / 2.0;

        // (a) The band right of the tab is the strip fill, not the body colour.
        assert_eq!(pixel_at(&renderer, 600.0, 10.0), theme.tab_bar);

        // (b) The active tab is the terminal background: a cut-out in the band.
        assert_eq!(
            pixel_at(&renderer, centre_x, body.y + body.h / 2.0),
            theme.tab_active
        );
        assert_eq!(theme.tab_active, theme.background);

        // (c) 8 px of strip sit above the tab, so y=2 is band and the first row
        // of the body is the tab colour.
        assert_eq!(pixel_at(&renderer, centre_x, 2.0), theme.tab_bar);
        assert_eq!(
            pixel_at(&renderer, centre_x, body.y + 1.0),
            theme.tab_active
        );

        // (d) The bounding box's top-left corner is rounded, so the band shows
        // there; 10 px along the same row is the outline.
        assert_eq!(pixel_at(&renderer, body.x, body.y), theme.tab_bar);
        assert_eq!(pixel_at(&renderer, body.x + 10.0, body.y), theme.tab_border);

        // (e) The only separator on this run of strip is the `+`/`⌄` boundary,
        // measured `#3D3D3D` in WT. Nothing is drawn at the `+` button's left
        // edge, and the separator column is full height (the band rule and the
        // strip's own edges are outside 1..strip_h-1).
        for y in 1..layout.strip_h as u32 - 1 {
            assert_eq!(
                pixel_at(&renderer, layout.plus.x, y as f32),
                theme.tab_bar,
                "nothing may be drawn at the + button's left edge (y={y})"
            );
            assert_eq!(
                pixel_at(&renderer, layout.chevron.x, y as f32),
                theme.tab_separator,
                "the +/⌄ boundary must be the separator at y={y}"
            );
        }
    }

    #[test]
    fn an_icon_never_paints_outside_a_tab_that_cannot_hold_it() {
        let mut renderer = default_renderer();
        renderer.resize(200, 120);
        // A strip too narrow to give either of two tabs its own icon slot:
        // each tab is 20 px wide while the icon alone wants 28.
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(242.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let second = layout.tabs[1].rect;
        for y in 1..layout.strip_h as u32 - 1 {
            for x in 1..second.w as u32 - 1 {
                assert_eq!(
                    pixel_at(&renderer, second.x + x as f32, y as f32),
                    renderer.theme.tab_bar,
                    "icon ink from the first tab reached ({x}, {y}) in the second"
                );
            }
        }
    }

    #[test]
    fn the_active_tab_paints_its_app_icon() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);

        // The icon slot starts 12 logical px into the tab and is 16 px square,
        // centred in the 32 px body, so its middle row is y=24.
        let icon_pixel = pixel_at(&renderer, layout.tabs[0].body.x + 20.0, 24.0);
        assert_ne!(
            icon_pixel, renderer.theme.tab_active,
            "the icon slot is empty"
        );
        // The inactive tab stamps the same icon over its own fill.
        assert_ne!(
            pixel_at(&renderer, layout.tabs[1].body.x + 20.0, 24.0),
            renderer.theme.tab_bar,
            "every tab, not only the active one, carries the icon"
        );
    }

    #[test]
    fn a_tab_stamps_its_own_icon_instead_of_the_app_icon() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        // Solid red: no pixel of this can come from the bundled app icon.
        let red = crate::icon::TabIcon {
            rgba: [255, 0, 0, 255].repeat(16 * 16),
            size: 16,
        };
        renderer.draw_chrome(&layout, crate::fonts::UI_SIZE_PX, 0, &[Some(red)], None);
        // The icon slot starts 12 px into the tab and is 16 px square.
        assert_eq!(
            pixel_at(&renderer, layout.tabs[0].rect.x + 20.0, 16.0),
            Rgb::new(255, 0, 0),
            "the tab's own icon must replace the app icon"
        );
    }

    /// The pixmap pixel at a physical coordinate. Reads the pixmap rather than
    /// the synced buffer so a test can inspect a drawing pass before `sync`.
    fn pixel_at(renderer: &Renderer, x: f32, y: f32) -> Rgb {
        let width = renderer.pixmap.width() as usize;
        let index = (y as usize * width + x as usize) * 4;
        let data = renderer.pixmap.data();
        Rgb::new(data[index], data[index + 1], data[index + 2])
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

    pub(crate) fn default_renderer() -> Renderer {
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

    pub(crate) fn packed(color: Rgb) -> u32 {
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
    fn the_scrollbar_track_starts_below_the_top_inset() {
        let mut renderer = default_renderer();
        renderer.resize(200, 300);
        renderer.set_top_inset(32);
        // Scrolled to the oldest history: the thumb rests at the track's top.
        let snapshot = scroll_snapshot(10, 20, 20);
        let maps: Vec<_> = (0..snapshot.rows)
            .map(|_| VisualMap::identity(snapshot.cols))
            .collect();
        renderer.draw(&snapshot, &maps, None);
        renderer.sync();

        let top = *painted_rows(&renderer)
            .first()
            .expect("the thumb must paint");
        assert_eq!(
            top,
            (renderer.padding + renderer.top_inset) as usize,
            "the track must begin below the strip, not under it"
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
    ///
    /// The grid-only number is kept under its historical label; the gate that
    /// decides pass/fail is the full frame, which also pays for the tab strip
    /// and the dropdown a real frame can carry.
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
        let width = cols as u32 * cell_w + 2 * padding;
        let height = rows as u32 * cell_h + 2 * padding + TAB_STRIP_HEIGHT as u32;
        assert!(renderer.resize(width, height));
        // A real window reserves the strip above the grid; without the inset the
        // grid would start at the top edge and the measurement would skip the
        // chrome's actual layout.
        renderer.set_top_inset(TAB_STRIP_HEIGHT as u32);

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

        let ui_px = crate::fonts::UI_SIZE_PX;
        let layout = TabBarLayout::new(width as f32, TAB_STRIP_HEIGHT, &["one", "two"]);
        let menu = {
            use crate::ui::menu::{MenuAction, MenuItem, MenuLayout};
            let items = vec![
                MenuItem::fixed(MenuAction::Settings, "Settings", ""),
                MenuItem::fixed(MenuAction::About, "About", ""),
            ];
            MenuLayout::new(layout.chevron, items, width as f32, height as f32)
        };

        // Warm the glyph atlas so the measurement covers drawing, not discovery.
        renderer.draw(&snapshot, &maps, None);
        draw_chrome(&mut renderer, &layout, ui_px, 0, None);
        renderer.draw_menu(&menu, ui_px);

        const FRAMES: usize = 40;
        let mut render_samples = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let start = Instant::now();
            renderer.draw(&snapshot, &maps, None);
            render_samples.push(start.elapsed());
        }
        render_samples.sort_unstable();
        let render_median = render_samples[FRAMES / 2];
        let fastest = render_samples[0];
        let slowest = render_samples[FRAMES - 1];
        println!(
            "full frame 200x50: median {render_median:?}, fastest {fastest:?}, slowest {slowest:?}"
        );

        let mut frame_samples = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let start = Instant::now();
            renderer.draw(&snapshot, &maps, None);
            draw_chrome(&mut renderer, &layout, ui_px, 0, None);
            frame_samples.push(start.elapsed());
        }
        frame_samples.sort_unstable();
        let frame_median = frame_samples[FRAMES / 2];
        let frame_fastest = frame_samples[0];
        let frame_slowest = frame_samples[FRAMES - 1];
        println!(
            "full frame with chrome 200x50: median {frame_median:?}, fastest {frame_fastest:?}, \
             slowest {frame_slowest:?}"
        );

        // One frame with the dropdown open covers the third painter a real
        // frame can pay for.
        let start = Instant::now();
        renderer.draw(&snapshot, &maps, None);
        draw_chrome(&mut renderer, &layout, ui_px, 0, None);
        renderer.draw_menu(&menu, ui_px);
        println!("full frame with chrome and dropdown: {:?}", start.elapsed());

        // A shared CI box has scheduling spikes, so the fastest frame is the
        // honest measure of the renderer's own cost; the median has to hold too.
        assert!(
            fastest < Duration::from_millis(8),
            "a 200x50 redraw took {fastest:?} at best, the budget is 8 ms"
        );
        assert!(
            render_median < Duration::from_millis(8),
            "a 200x50 redraw took {render_median:?} at the median, the budget is 8 ms"
        );
        assert!(
            frame_median < Duration::from_millis(8),
            "a 200x50 frame with chrome took {frame_median:?} at the median, the budget is 8 ms"
        );
    }
}
