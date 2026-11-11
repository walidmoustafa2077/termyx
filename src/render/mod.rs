//! Grid rasterization to the window surface.

mod atlas;
mod context_menu;
mod find_bar;
mod glyph_cache;
mod menu;
mod rename;
mod tooltip;
mod ui_text;

use ab_glyph::GlyphId;
use tiny_skia::Pixmap;

use crate::bidi;
use crate::find::FindState;
use crate::fonts::FontSet;
use crate::icon::TabIcon;
use crate::scrollbar;
use crate::selection::Selection;
use crate::shape;
use crate::term::{Color, CursorShape, GridSnapshot, RowSnapshot, Style};
use crate::theme::{Rgb, Theme};
use crate::ui::tab_bar::{
    Rect, StripHover, TabBarLayout, TAB_ICON_GAP, TAB_ICON_INSET, TAB_ICON_SIZE, TAB_RADIUS,
    TAB_STRIP_HEIGHT, TAB_TITLE_SIZE, TITLE_SIZE,
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
    /// The display scale factor, which sizes the scrollbar's thumb, its
    /// minimum length and the gutter reserved for it.
    scale_factor: f32,
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
/// Both come from the icon face and match Windows Terminal's: the same two
/// overlapping squares the system draws for restore and expand.
fn maximize_glyph(maximized: bool) -> &'static str {
    if maximized {
        crate::fonts::GLYPH_RESTORE
    } else {
        crate::fonts::GLYPH_MAXIMIZE
    }
}

/// The fill and glyph colour for one caption control.
///
/// Split out of the painter so the hover/resting colour table is testable
/// without reading pixels. Only the control under the pointer is filled: the
/// close uses the accent red, the other two the control hover tone, and every
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
            theme.control_hover
        }
    });
    let glyph = if hovered {
        theme.tab_title
    } else {
        theme.tab_title_inactive
    };
    (fill, glyph)
}

/// The hover pill WT draws under a strip control: 32x24 px, centred
/// horizontally on the control and vertically on the 32 px tab body rather
/// than the 40 px band, so it sits 4 px below the band's centre.
fn hover_pill(layout: &TabBarLayout, rect: Rect) -> Rect {
    let k = layout.strip_h / TAB_STRIP_HEIGHT;
    Rect {
        x: rect.x + (rect.w - 32.0 * k) / 2.0,
        y: layout.strip_h / 2.0 - 8.0 * k,
        w: 32.0 * k,
        h: 24.0 * k,
    }
}

/// What one grid row's runs need to pick their ink: the row's visual map, the
/// logical selection, and the row index both are keyed on. Bundled because a
/// run painter that took them separately would need eight arguments.
struct RunRow<'a> {
    maps: &'a [VisualMap],
    selection: Option<&'a Selection>,
    index: usize,
}

impl RunRow<'_> {
    /// Whether the logical `cell` on this row falls inside the selection.
    fn is_selected(&self, cell: usize) -> bool {
        let Some(selection) = self.selection else {
            return false;
        };
        let Some(map) = self.maps.get(self.index) else {
            return false;
        };
        selection.is_selected(self.index, cell, map.visual_of(cell), self.maps)
    }
}

/// Sub-samples per axis when antialiasing a rounded corner. Four is the
/// smallest count that keeps a 8 px radius from showing stair steps at 100 %.
const CORNER_SAMPLES: u32 = 4;

/// Share of the pixel at (`x`, `y`) that lies inside the rounded rectangle
/// whose corner is centred on (`cx`, `cy`).
///
/// `right`/`bottom` mirror the corner: to the rect's interior the rounded
/// corner cuts away the part beyond the circle, so a sample is inside when it
/// has passed the centre on the axis that points into the rect, or when it is
/// within the radius. Windows Terminal's own corners fade over a few pixels
/// (`#333333` → `#2E2E2E` → `#0C0C0C` in one row); a hard step is what made our
/// tab look like a cut-out pasted on the strip.
fn corner_coverage(
    x: f32,
    y: f32,
    cx: f32,
    cy: f32,
    radius: f32,
    right: bool,
    bottom: bool,
) -> f32 {
    let mut hits = 0;
    for sy in 0..CORNER_SAMPLES {
        for sx in 0..CORNER_SAMPLES {
            let sample_x = x + (sx as f32 + 0.5) / CORNER_SAMPLES as f32;
            let sample_y = y + (sy as f32 + 0.5) / CORNER_SAMPLES as f32;
            let inside_x = if right {
                sample_x <= cx
            } else {
                sample_x >= cx
            };
            let inside_y = if bottom {
                sample_y <= cy
            } else {
                sample_y >= cy
            };
            if inside_x || inside_y {
                hits += 1;
                continue;
            }
            let dx = sample_x - cx;
            let dy = sample_y - cy;
            if dx * dx + dy * dy <= radius * radius {
                hits += 1;
            }
        }
    }
    hits as f32 / (CORNER_SAMPLES * CORNER_SAMPLES) as f32
}

impl Renderer {
    /// Builds a renderer with a one-pixel surface.
    ///
    /// `padding` is the window margin in physical pixels and `scale_factor` the
    /// display's scale: the caller has already applied the factor to the
    /// configured logical padding, and the scrollbar keeps the factor to size
    /// its own physical metrics. Every coordinate below is physical.
    pub fn new(
        fonts: FontSet,
        theme: Theme,
        padding: u32,
        scale_factor: f32,
    ) -> anyhow::Result<Self> {
        let pixmap = Pixmap::new(1, 1)
            .ok_or_else(|| anyhow::anyhow!("could not allocate the initial pixmap"))?;
        Ok(Self {
            pixmap,
            pixels: vec![0],
            fonts,
            cache: GlyphCache::new(),
            theme,
            padding,
            scale_factor,
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
        find: Option<&FindState>,
    ) {
        self.draw_with_spellcheck(snapshot, maps, selection, find, &[]);
    }

    /// Draws the grid, then the spellcheck underlines for `spans`. Separate
    /// from [`Renderer::draw`] so the 34 existing call sites keep their four
    /// arguments until the app is switched over.
    pub fn draw_with_spellcheck(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        selection: Option<&Selection>,
        find: Option<&FindState>,
        spellcheck: &[crate::spellcheck::Misspelling],
    ) {
        self.shape_cache.clear();
        self.draw_background();
        let cell_w = self.fonts.cell_w as f32;
        let cell_h = self.fonts.cell_h as f32;
        let padding = self.padding as f32;
        let top = self.grid_top();

        self.draw_cell_backgrounds(snapshot, maps, selection, cell_w, cell_h);

        if let Some(find) = find {
            for (index, hit) in find.matches.iter().enumerate() {
                let row = hit.line + snapshot.display_offset as i32;
                if row < 0 || row as usize >= snapshot.lines.len() {
                    continue;
                }
                let color = if index == find.current {
                    self.theme.find_current
                } else {
                    self.theme.find_match
                };
                // Map logical columns to visual columns with the row's VisualMap.
                let cols = snapshot.lines[row as usize].cells.len();
                let start = hit.start.min(cols);
                let end = hit.end.min(cols);
                let map = &maps[row as usize];
                let mut columns: Vec<usize> = (start..end).map(|col| map.visual_of(col)).collect();
                columns.sort_unstable();
                columns.dedup();
                for col in columns {
                    self.fill_rect(
                        padding + col as f32 * cell_w,
                        top + row as f32 * cell_h,
                        cell_w,
                        cell_h,
                        color,
                    );
                }
            }
        }

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
                self.draw_shaped_run(
                    run,
                    line,
                    RunRow {
                        maps,
                        selection,
                        index: row,
                    },
                    cursor,
                    rtl_base,
                );
            }
        }

        self.draw_decorations(snapshot, maps, cell_w, cell_h);

        self.draw_spellcheck_underlines(snapshot, maps, spellcheck, cell_w, cell_h);

        if let Some(geom) = scrollbar::geometry(
            self.pixmap.width(),
            self.padding + self.top_inset,
            self.fonts.cell_h,
            scrollbar::BarState {
                rows: snapshot.rows,
                history: snapshot.history,
                offset: snapshot.display_offset,
                hovered: self.scrollbar_hover,
            },
            self.scale_factor,
        ) {
            let color = self.theme.indexed(8);
            if self.scrollbar_hover {
                let track_w = 2.0 * self.scale_factor;
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
        colors: &[Option<Rgb>],
        hover: Option<StripHover>,
    ) {
        if layout.strip_h <= 0.0 || layout.width <= 0.0 {
            return;
        }
        self.fill_band(layout);
        self.draw_tabs(layout, active, colors, hover);
        self.draw_buttons(layout, ui_px, hover);
        // The rule goes down after the buttons: a hovered button's fill starts
        // at the top of the strip, and painting it later would erase a window-
        // wide rule behind that button.
        self.draw_band_rule(layout, active);
        // The caption controls paint last. Their hover fill stops one pixel
        // above the strip's bottom edge, so painting them after the rule cannot
        // erase it, and the rule (already down) cannot cut through a control.
        self.draw_caption_buttons(layout, hover);
        self.draw_titles(layout, active, icons, colors, hover);
    }

    /// The reserved band is chrome, not grid: it gets its own colour across
    /// the full width, whichever tabs happen to sit on it.
    fn fill_band(&mut self, layout: &TabBarLayout) {
        self.fill_rect(0.0, 0.0, layout.width, layout.strip_h, self.theme.tab_bar);
    }

    /// One fill for the active tab, then the pill behind a hovered close
    /// button.
    ///
    /// The tab body floats on the strip: WT insets it by 8 px at the top and
    /// rounds its top corners, so the band shows above it. Only the active tab
    /// gets a fill — its fill is the terminal background, which makes it read
    /// as a cut-out — while an inactive tab paints nothing at all, even under
    /// the pointer: WT highlights the control being hit, not the tab card
    /// (measured: hovering a tab body changes no pixel).
    ///
    /// There is no outline. Measured on WT 1.24, the edge of the active tab
    /// goes `#333333` → `#2E2E2E` → `#0C0C0C` across one pixel: a blend of the
    /// rounded edge with the band, not a `#292929` stroke. Drawing a stroke
    /// made the tab read as a card pasted on the strip.
    fn draw_tabs(
        &mut self,
        layout: &TabBarLayout,
        active: usize,
        colors: &[Option<Rgb>],
        hover: Option<StripHover>,
    ) {
        let radius = TAB_RADIUS * layout.strip_h / TAB_STRIP_HEIGHT;
        for tab in &layout.tabs {
            let color = colors.get(tab.index).copied().flatten();
            let fill = color.or_else(|| (tab.index == active).then_some(self.theme.tab_active));
            if let Some(fill) = fill {
                let radius = radius.min(tab.body.w / 2.0);
                self.fill_round_top(tab.body, radius, fill);
            }
        }
        self.draw_close_backdrop(layout, hover);
    }

    /// The 32x24 pill behind a `✕` while the pointer is over the glyph.
    ///
    /// WT highlights the hit target, not the 16 px glyph box: measured as a
    /// 32 px wide, 24 px tall rounded rect centred on the close glyph and
    /// filled `#1A1A1A`, a faint scrim that reads as a recess on both the band
    /// and the active tab. Only the pointer over the glyph shows it — hovering
    /// elsewhere on the same tab changes no pixel — so the hover is matched
    /// against [`StripHover::Close`], not against the tab.
    fn draw_close_backdrop(&mut self, layout: &TabBarLayout, hover: Option<StripHover>) {
        let Some(StripHover::Close(index)) = hover else {
            return;
        };
        let Some(tab) = layout.tabs.iter().find(|tab| tab.index == index) else {
            return;
        };
        if tab.close.w <= 0.0 {
            return;
        }
        let pill = hover_pill(layout, tab.close);
        self.fill_round_rect(
            pill,
            4.0 * layout.strip_h / TAB_STRIP_HEIGHT,
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
            (
                crate::fonts::GLYPH_ADD,
                layout.plus,
                StripHover::Plus,
                ui_px,
            ),
            (
                crate::fonts::GLYPH_CHEVRON,
                layout.chevron,
                StripHover::Chevron,
                crate::fonts::CONTROL_ICON_PX * k,
            ),
        ];
        for (glyph, rect, target, glyph_px) in buttons {
            let hovered = hover == Some(target);
            let color = if hovered {
                self.theme.control_hover_ink
            } else {
                foreground
            };
            let pill = hover_pill(layout, rect);
            if hovered {
                self.fill_round_rect(pill, 4.0 * k, self.theme.control_hover);
            }
            // Centred in the pill, not the full-height button: WT's `+` ink is
            // centred on the 32 px tab body, so a glyph centred in the 40 px
            // band sits 4 px high inside its own highlight.
            ui_text::draw_text_centred(self, glyph, pill, glyph_px, color);
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
    /// red, the other two the control hover tone, and a resting control has no
    /// fill. The fill stops one pixel above the band rule so painting these
    /// after [`Renderer::draw_band_rule`] leaves it intact. The third control
    /// shows a restore glyph while the window is maximised: the expand glyph
    /// there would claim that a click still expands an already-expanded window.
    /// The glyphs are the icon face's, the same ones the system title bar draws.
    fn draw_caption_buttons(&mut self, layout: &TabBarLayout, hover: Option<StripHover>) {
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let controls = [
            (
                crate::fonts::GLYPH_MINIMIZE,
                layout.minimize,
                StripHover::Minimize,
                0.0,
            ),
            (
                maximize_glyph(self.maximized),
                layout.maximize,
                StripHover::Maximize,
                k,
            ),
            (
                crate::fonts::GLYPH_CLOSE,
                layout.close_window,
                StripHover::CloseWindow,
                k,
            ),
        ];
        for (glyph, rect, target, drop) in controls {
            let (fill, color) = caption_colors(&self.theme, target, hover);
            if let Some(fill) = fill {
                self.fill_rect(rect.x, rect.y, rect.w, (rect.h - 1.0).max(0.0), fill);
            }
            // The controls are centred on their own ink boxes, so each lands on
            // the reference row only with its own nudge. WT paints the minimise
            // bar on row 20, which is what ink centring already gives, and the
            // boxed glyphs on rows 16..25, one row below where centring puts
            // them.
            let centred = Rect {
                y: rect.y + drop,
                ..rect
            };
            ui_text::draw_text_centred(
                self,
                glyph,
                centred,
                crate::fonts::CONTROL_ICON_PX * k,
                color,
            );
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
        active: usize,
        icons: &[Option<TabIcon>],
        colors: &[Option<Rgb>],
        hover: Option<StripHover>,
    ) {
        // One scale factor for the whole chrome budget, so the painter and
        // `TabBarLayout` cannot drift apart at another DPI.
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let icon_inset = TAB_ICON_INSET * k;
        let icon_size = TAB_ICON_SIZE * k;
        let title_left = (TAB_ICON_INSET + TAB_ICON_SIZE + TAB_ICON_GAP) * k;
        for tab in &layout.tabs {
            // A crowded strip can shrink a tab below the icon's own slot; the
            // blit would then spill into the neighbour, so skip it exactly as
            // the layout drops a close box that will not fit.
            let icon = icons
                .get(tab.index)
                .and_then(|icon| icon.as_ref())
                .or(self.app_icon.as_ref())
                .filter(|_| tab.rect.w >= icon_inset + icon_size);
            if let Some(icon) = icon {
                let icon_x = tab.rect.x + icon_inset;
                // Content is positioned on the tab body, not the band: WT's
                // icon sits centred in its 32 px card. The card's centre lands
                // one pixel low of centre because the ink runs 19..30 while the
                // band's centre row is 24: centring the box alone put the ink on
                // 18..29, so the box shifts down to share the title's row.
                let icon_y = tab.body.y + (tab.body.h - icon_size) / 2.0 + k;
                blit_tab_icon(&mut self.pixmap, icon, icon_x, icon_y, icon_size);
            }
            let title = layout.titles.get(tab.index).map_or("", String::as_str);
            // A tab is sized as its title plus `TITLE_SIZE`, so the title owns
            // exactly what the chrome budget leaves.
            let title_width = (tab.rect.w - TITLE_SIZE * k).max(0.0);
            let color = colors.get(tab.index).copied().flatten();
            let title_color = match color {
                Some(color) => self.theme.ink_on(color),
                None if tab.index == active => self.theme.tab_title,
                None => self.theme.tab_title_inactive,
            };
            // The title is a size below the rest of the chrome: at `ui_px` its
            // ink is 12 px tall and the tab reads visibly larger than the
            // reference, whose title is a 10 px cap band.
            let title_px = TAB_TITLE_SIZE * k;
            // 9 px below the body's top edge puts the cap band on the rows WT
            // measured (21..29 inside the 32 px tab body) and keeps the outline
            // on whole pixels, so the double pass below cannot blur it across a
            // tenth row.
            let title_top = tab.body.y + 9.0 * k;
            // WinUI renders the tab header through DirectWrite's grayscale
            // rasterizer, whose gamma darkening gives a measured 2 px stem where
            // the same 12 px regular face here draws 1 px. The semibold cut is
            // the closest weight this renderer can reach, and a second pass half
            // a pixel across thickens the stem edges further without moving the
            // advance or the clip. Measured, that lands 6 % above the
            // reference's ink; one pass alone lands 29 % below it.
            let style = ui_text::TextStyle {
                face: crate::fonts::SEMIBOLD_FONT_ID,
                px: title_px,
                color: title_color,
            };
            for dx in [0.0, 0.5 * k] {
                ui_text::draw_text_ellipsis_in(
                    self,
                    title,
                    tab.rect.x + title_left + dx,
                    title_top,
                    style,
                    title_width,
                );
            }
            let closes = hover == Some(StripHover::Close(tab.index));
            let shows_close =
                tab.index == active || closes || hover == Some(StripHover::Tab(tab.index));
            if shows_close && tab.close.w > 0.0 {
                // The glyph brightens only while its own pill is shown: WT
                // leaves the resting `✕` grey even when the tab body under the
                // pointer is hovered.
                let ink = if closes {
                    self.theme.tab_close_hover_ink
                } else {
                    title_color
                };
                // The tab's own ✕ is the icon face's close, not a text glyph:
                // the text `✕` at 10 px leaves a few strokes of ink and reads
                // as a hollow box in the pill, while WT's is a 9x9 cross.
                ui_text::draw_text_centred(
                    self,
                    crate::fonts::GLYPH_CLOSE,
                    tab.close,
                    crate::fonts::TAB_CLOSE_ICON_PX * k,
                    ink,
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
        row: RunRow<'_>,
        cursor: Option<(usize, usize, CursorShape)>,
        rtl_base: bool,
    ) {
        let cell_w = self.fonts.cell_w as f32;
        let cell_h = self.fonts.cell_h as f32;
        let padding = self.padding as f32;
        let top = self.grid_top();
        // Share one baseline across the primary and Arabic faces so Arabic
        // sits on the same line as Latin instead of floating above it.
        let baseline = top + row.index as f32 * cell_h + self.fonts.line_ascent();
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
                shape == CursorShape::Block
                    && cursor_row == row.index
                    && cluster.slots.contains(&visual)
            });
            // WT inverts the glyphs against the selection band instead of
            // tinting them, so a selected cluster switches to the band's
            // contrast ink. Decided from the cluster's first cell: a cluster is
            // one grapheme, and WT never splits one across the selection edge.
            let selected = row.is_selected(cluster.cells.start);
            let color = if under_cursor {
                self.theme.background
            } else if selected {
                self.theme.selection_text()
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
                    top + row.index as f32 * cell_h,
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

    /// Paints one underline band at the bottom of every visual cell a span
    /// covers. Runs after the decorations and before the scrollbar so it sits
    /// over glyph ink but under the chrome.
    fn draw_spellcheck_underlines(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        spans: &[crate::spellcheck::Misspelling],
        cell_w: f32,
        cell_h: f32,
    ) {
        if spans.is_empty() {
            return;
        }
        let thickness = (2.0 * self.scale_factor).round().max(1.0);

        for span in spans {
            let row = span.row as i32 + snapshot.display_offset as i32;
            if row < 0 || row as usize >= snapshot.lines.len() {
                continue;
            }
            let Some(map) = maps.get(row as usize) else {
                continue;
            };
            let cols = snapshot.lines[row as usize].cells.len();
            if span.col >= cols {
                continue;
            }
            let start = span.col;
            let end = (span.col + span.len.max(1)).min(cols);
            // Map every logical column, not just the span's ends, so bidi
            // reordering and wide cells land on the right visual cells.
            let mut columns: Vec<usize> = (start..end).map(|col| map.visual_of(col)).collect();
            columns.sort_unstable();
            columns.dedup();
            let y = self.grid_top() + row as f32 * cell_h + (cell_h - thickness);
            for col in columns {
                self.fill_rect(
                    self.padding as f32 + col as f32 * cell_w,
                    y,
                    cell_w,
                    thickness,
                    self.theme.spellcheck_underline,
                );
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

    /// Blends `color` into one pixel by `alpha` instead of overwriting it, so a
    /// rounded corner can fade into whatever it is drawn over.
    fn blend_pixel(&mut self, x: f32, y: f32, color: Rgb, alpha: f32) {
        let pixmap_w = self.pixmap.width() as i32;
        let pixmap_h = self.pixmap.height() as i32;
        let px = x.round() as i32;
        let py = y.round() as i32;
        if px < 0 || py < 0 || px >= pixmap_w || py >= pixmap_h {
            return;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let index = ((py * pixmap_w + px) * 4) as usize;
        let alpha = (alpha * 255.0).round() as u32;
        let data = self.pixmap.data_mut();
        data[index] = blend_u8(data[index], color.r, alpha);
        data[index + 1] = blend_u8(data[index + 1], color.g, alpha);
        data[index + 2] = blend_u8(data[index + 2], color.b, alpha);
        data[index + 3] = 255;
    }

    /// Fills `rect` with its top two corners rounded by `radius`, leaving the
    /// bottom corners square so a tab meets the strip's rule flush.
    fn fill_round_top(&mut self, rect: Rect, radius: f32, color: Rgb) {
        let radius = radius.min(rect.w / 2.0).min(rect.h).max(0.0);
        self.fill_rect(
            rect.x,
            rect.y + radius,
            rect.w,
            (rect.h - radius).max(0.0),
            color,
        );
        let rows = radius.round() as usize;
        // One extra column each side carries the fade into the band.
        let caps = (radius + 1.0).min(rect.w / 2.0) as usize;
        for dy in 0..rows {
            let y = rect.y + dy as f32;
            let middle = (rect.w - 2.0 * caps as f32).max(0.0);
            self.fill_rect(rect.x + caps as f32, y, middle, 1.0, color);
            for dx in 0..caps {
                let left = corner_coverage(
                    rect.x + dx as f32,
                    y,
                    rect.x + radius,
                    rect.y + radius,
                    radius,
                    false,
                    false,
                );
                self.blend_pixel(rect.x + dx as f32, y, color, left);
                let right = corner_coverage(
                    rect.x + rect.w - 1.0 - dx as f32,
                    y,
                    rect.x + rect.w - radius,
                    rect.y + radius,
                    radius,
                    true,
                    false,
                );
                self.blend_pixel(rect.x + rect.w - 1.0 - dx as f32, y, color, right);
            }
        }
    }

    /// Fills `rect` with all four corners rounded by `radius`, for the hover
    /// pills that float inside the strip rather than sitting on its rule.
    fn fill_round_rect(&mut self, rect: Rect, radius: f32, color: Rgb) {
        let radius = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
        self.fill_rect(
            rect.x,
            rect.y + radius,
            rect.w,
            (rect.h - 2.0 * radius).max(0.0),
            color,
        );
        let rows = radius.round() as usize;
        let caps = (radius + 1.0).min(rect.w / 2.0) as usize;
        for dy in 0..rows {
            let top = rect.y + dy as f32;
            let bottom = rect.y + rect.h - 1.0 - dy as f32;
            let middle = (rect.w - 2.0 * caps as f32).max(0.0);
            self.fill_rect(rect.x + caps as f32, top, middle, 1.0, color);
            self.fill_rect(rect.x + caps as f32, bottom, middle, 1.0, color);
            for dx in 0..caps {
                for (x, y, is_right, is_bottom) in [
                    (rect.x + dx as f32, top, false, false),
                    (rect.x + rect.w - 1.0 - dx as f32, top, true, false),
                    (rect.x + dx as f32, bottom, false, true),
                    (rect.x + rect.w - 1.0 - dx as f32, bottom, true, true),
                ] {
                    let cx = if is_right {
                        rect.x + rect.w - radius
                    } else {
                        rect.x + radius
                    };
                    let cy = if is_bottom {
                        rect.y + rect.h - radius
                    } else {
                        rect.y + radius
                    };
                    let alpha = corner_coverage(x, y, cx, cy, radius, is_right, is_bottom);
                    self.blend_pixel(x, y, color, alpha);
                }
            }
        }
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

    /// The window margin around the grid, in physical pixels.
    pub fn padding(&self) -> u32 {
        self.padding
    }

    /// Sets the window margin in physical pixels. Called when the display
    /// scale changes so the margin grows with the text and the strip.
    pub fn set_padding(&mut self, padding: u32) {
        self.padding = padding;
    }

    /// The display scale factor the scrollbar metrics are derived from.
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    /// Updates the display scale. Called on a display change with the event's
    /// factor, before the grid is rebuilt, so the reserved gutter and the
    /// thumb resize in the same frame as the fonts.
    pub fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor;
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
        renderer.draw_chrome(layout, ui_px, active, &[], &[], hover);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
    fn tab_color_fills_the_tab_body() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        let color = Rgb::new(120, 40, 200);
        let colors = vec![Some(color), None];
        renderer.draw_chrome(&layout, crate::fonts::UI_SIZE_PX, 0, &[], &colors, None);
        // Both probes sit below the rounded top corners.
        let colored = pixel_at(&renderer, layout.tabs[0].body.x + 4.0, 16.0);
        assert_eq!(
            colored, color,
            "a tab with a color must fill with that color, not the active tab's"
        );
        let uncolored = pixel_at(&renderer, layout.tabs[1].body.x + 4.0, 16.0);
        assert_eq!(
            uncolored, renderer.theme.tab_bar,
            "a tab with no color keeps the band behind it"
        );
    }

    #[test]
    fn tab_color_switches_the_title_ink_for_contrast() {
        // A dark tab reads the grid's light foreground, the same as an active
        // tab's own title.
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        renderer.draw_chrome(
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            &[],
            &[Some(Rgb::new(20, 20, 20))],
            None,
        );
        let title_rect = {
            let tab = &layout.tabs[0];
            crate::ui::tab_bar::Rect {
                x: tab.body.x + 34.0,
                y: tab.body.y + 9.0,
                w: (tab.body.w - 34.0 - 24.0).max(0.0),
                h: 16.0,
            }
        };
        let peak = brightest_red(&renderer, title_rect);
        assert!(
            peak >= renderer.theme.foreground.r - 2,
            "a dark tab's title must be the bright foreground ink (peak red {peak})"
        );

        // A light tab flips to the grid's own dark background.
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        renderer.draw_chrome(
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            &[],
            &[Some(Rgb::new(240, 240, 240))],
            None,
        );
        let title_rect = {
            let tab = &layout.tabs[0];
            crate::ui::tab_bar::Rect {
                x: tab.body.x + 34.0,
                y: tab.body.y + 9.0,
                w: (tab.body.w - 34.0 - 24.0).max(0.0),
                h: 16.0,
            }
        };
        let trough = darkest_red(&renderer, title_rect);
        assert!(
            trough <= renderer.theme.background.r + 2,
            "a light tab's title must be the dark background ink (trough red {trough})"
        );
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
    fn adjacent_tabs_meet_without_a_divider() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let boundary = layout.tabs[1].rect.x;
        // Measured: WT draws no divider between tabs — the active tab's fill
        // ends, and the strip colour of the inactive neighbour is the contrast.
        // A stroke here was the line that made the tab look pasted on.
        assert_eq!(
            pixel_at(&renderer, boundary - 1.0, 16.0),
            renderer.theme.tab_active
        );
        assert_eq!(
            pixel_at(&renderer, boundary, 16.0),
            renderer.theme.tab_bar,
            "an inactive tab must not paint a divider of its own"
        );
    }

    #[test]
    fn hovering_a_tab_body_repaints_nothing() {
        let mut renderer = default_renderer();
        renderer.resize(600, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(600.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        // Below the title ink and away from the icon and the close box.
        let body = (layout.tabs[1].rect.x + 60.0, 34.0);

        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        assert_eq!(pixel_at(&renderer, body.0, body.1), renderer.theme.tab_bar);

        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Tab(1)),
        );
        assert_eq!(
            pixel_at(&renderer, body.0, body.1),
            renderer.theme.tab_bar,
            "WT highlights the control under the pointer, not the tab body"
        );
    }

    #[test]
    fn hovering_the_close_glyph_paints_a_pill_wider_than_the_glyph_box() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        let close = layout.tabs[0].close;
        // Eight pixels left of the glyph box: inside WT's 32 px pill and
        // outside the 16 px close box.
        let left_of_glyph = (close.x - 4.0, close.y + 8.0);
        // The pill's rounded corner, which must stay unpainted.
        let pill_corner = (close.x - 8.0, 12.0);

        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        assert_eq!(
            pixel_at(&renderer, left_of_glyph.0, left_of_glyph.1),
            renderer.theme.tab_active
        );

        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Close(0)),
        );
        assert_eq!(
            pixel_at(&renderer, left_of_glyph.0, left_of_glyph.1),
            renderer.theme.tab_close_hover,
            "the pill is centred on the glyph and eight pixels wider on each side"
        );
        assert_eq!(
            pixel_at(&renderer, pill_corner.0, pill_corner.1),
            renderer.theme.tab_active,
            "the pill's corners are rounded"
        );
        assert_eq!(
            count_color(&renderer, close, renderer.theme.control_hover),
            0,
            "the close pill must not reuse the control hover tone"
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

    /// Darkest red channel inside `rect`: the inverse of [`brightest_red`], for
    /// reading ink that must stay darker than a light fill.
    fn darkest_red(renderer: &Renderer, rect: crate::ui::tab_bar::Rect) -> u8 {
        (rect.y as u32..(rect.y + rect.h) as u32)
            .flat_map(|y| (rect.x as u32..(rect.x + rect.w) as u32).map(move |x| (x, y)))
            .map(|(x, y)| pixel_at(renderer, x as f32, y as f32).r)
            .min()
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
        // The control hover pill is `#3b3b3b`; pin the literal, not the theme
        // field, so a palette rename cannot silently drop it.
        let hover = Rgb::new(0x3b, 0x3b, 0x3b);
        // Three pixels inside the pill's left edge, clear of the centred glyph
        // and of the separator painted at the chevron's left border.
        let plus = (layout.plus.x + 3.0, 24.0);
        let chevron = (layout.chevron.x + 3.0, 24.0);

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
    fn the_active_tab_is_a_flat_fill_with_an_open_bottom() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let tab = layout.tabs[0].body;
        // Measured on WT: the tab is a plain fill of the terminal colour with no
        // stroke on any edge. The outline we drew was the "pasted card" look.
        for (x, y, edge) in [
            (tab.x + 40.0, tab.y, "top"),
            (tab.x, tab.y + 16.0, "left"),
            (tab.x + tab.w - 1.0, tab.y + 16.0, "right"),
        ] {
            assert_eq!(
                pixel_at(&renderer, x, y),
                renderer.theme.tab_active,
                "the active tab's {edge} edge must not be stroked"
            );
        }
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
        // The caption controls and the strip's `+`/`⌄`/`✕` go through the same
        // per-character resolver; an unmapped glyph would paint `.notdef`.
        use ab_glyph::Font;

        let renderer = default_renderer();
        for glyph in [
            crate::fonts::GLYPH_MINIMIZE,
            crate::fonts::GLYPH_MAXIMIZE,
            crate::fonts::GLYPH_RESTORE,
            crate::fonts::GLYPH_CLOSE,
            crate::fonts::GLYPH_ADD,
            crate::fonts::GLYPH_CHEVRON,
        ] {
            let ch = glyph.chars().next().expect("a control glyph is one char");
            let (face, id) = renderer.fonts().chrome_glyph(ch);
            assert_eq!(
                face,
                crate::fonts::ICON_FONT_ID,
                "the system icon face must supply the control glyph {ch:?}"
            );
            let (font, _index) = renderer.fonts_face(face);
            assert_ne!(id.0, 0, "the resolved glyph for {ch:?} must exist");
            assert_ne!(
                font.glyph_id(ch).0,
                0,
                "face {face} must rasterize {ch:?}, not .notdef"
            );
            assert!(
                renderer.fonts().ui_width(glyph, crate::fonts::UI_SIZE_PX) > 0.0,
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
            "a maximised window must paint the restore glyph, not the expand one"
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

        // The other two use the control hover tone, and leaving the
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
            renderer.theme.control_hover,
            "a hovered minimise control uses the control hover tone"
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
            (StripHover::Minimize, theme.control_hover),
            (StripHover::Maximize, theme.control_hover),
            (StripHover::CloseWindow, theme.caption_close_hover),
        ];
        assert_ne!(theme.caption_close_hover, theme.control_hover);
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
        // layout drops the target and the painter must drop the pill and glyph
        // with it instead of bleeding into the neighbour.
        let layout = TabBarLayout::new(100.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        assert_eq!(layout.tabs[0].close.w, 0.0, "the layout must drop the box");
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Close(0)),
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
    fn hovering_an_inactive_tab_shows_its_close_glyph() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        let close = layout.tabs[1].close;
        // The inactive tab body is the bare band, so any ink in the close box is
        // the glyph. Compared against the band rather than the ink colour
        // because a 10 px glyph never fills a whole pixel.
        let painted = |renderer: &Renderer| {
            (0..close.h as u32).any(|dy| {
                (0..close.w as u32).any(|dx| {
                    pixel_at(renderer, close.x + dx as f32, close.y + dy as f32)
                        != renderer.theme.tab_bar
                })
            })
        };
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        assert!(
            !painted(&renderer),
            "an inactive tab hides its close glyph until the pointer arrives"
        );

        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Tab(1)),
        );
        assert!(
            painted(&renderer),
            "hovering the tab must reveal its close glyph"
        );
        assert_eq!(
            count_color(&renderer, close, renderer.theme.tab_close_hover_ink),
            0,
            "the glyph stays grey until the pointer is over the glyph itself"
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
        let title_left = tab.x + (TAB_ICON_INSET + TAB_ICON_SIZE + TAB_ICON_GAP) * k;
        let title_width = (tab.w - TITLE_SIZE * k).max(0.0);
        assert!(
            title_left + title_width <= layout.tabs[0].close.x,
            "the title clip ends at {}, inside the close box that starts at {}",
            title_left + title_width,
            layout.tabs[0].close.x
        );
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
    fn the_plus_glyph_centres_on_the_tab_body_not_the_band() {
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
        // WT's `+` is centred on the 32 px tab body (y=24), not on the 40 px
        // band (y=20): centring it in the button leaves it 4 px high inside the
        // highlight the pointer brings with it.
        let pill = hover_pill(&layout, layout.plus);
        assert_centred(&renderer, pill, renderer.theme.control_hover);
    }

    #[test]
    fn the_chevron_glyph_centres_on_the_tab_body_not_the_band() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(
            &mut renderer,
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            Some(StripHover::Chevron),
        );
        let pill = hover_pill(&layout, layout.chevron);
        // The chevron's separator runs down the button's first column, which
        // the pill shares; it is chrome, not glyph ink.
        let pill = crate::ui::tab_bar::Rect {
            x: pill.x + 1.0,
            w: pill.w - 1.0,
            ..pill
        };
        assert_centred(&renderer, pill, renderer.theme.control_hover);
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
        let (left, _, right, _) = ink_bounds(&renderer, close, renderer.theme.tab_active);
        let painted = right - left + 1.0;

        // The painted cross must be the icon face's own outline at the size the
        // tab asks for, not some other em size that happens to look close.
        let k = layout.strip_h / TAB_STRIP_HEIGHT;
        let size = crate::fonts::TAB_CLOSE_ICON_PX * k;
        let (ink_left, _, ink_right, _) = renderer
            .fonts()
            .chrome_ink_bounds(crate::fonts::GLYPH_CLOSE, size)
            .expect("the close glyph must have an outline");
        assert!(
            (painted - (ink_right - ink_left)).abs() <= 1.0,
            "the painted ✕ must match the {size} px ink width, got {painted}"
        );
    }

    #[test]
    fn the_close_glyph_shares_the_title_ink() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 0, None);
        let close = layout.tabs[0].close;
        let peak = brightest_red(&renderer, close);
        // The resting ✕ shares the title's ink so it stays readable on a
        // colored tab. On the uncolored active tab that ink is the title white,
        // so the ✕ must be white, not the old grey.
        assert!(
            peak >= renderer.theme.tab_title.r - 2,
            "the ✕ must match the title ink, got {peak}"
        );
        assert!(
            peak > renderer.theme.tab_bar.r,
            "the ✕ must still stand out on the #0C0C0C tab, got {peak}"
        );
    }

    #[test]
    fn the_active_index_moves_the_tab_fill() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout =
            crate::ui::tab_bar::TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        // The second tab is the one the window shows, so the fill follows the
        // active index rather than the leftmost tab.
        draw_chrome(&mut renderer, &layout, crate::fonts::UI_SIZE_PX, 1, None);
        let first = layout.tabs[0].body;
        let second = layout.tabs[1].body;
        assert_eq!(
            pixel_at(&renderer, first.x + 50.0, 16.0),
            renderer.theme.tab_bar,
            "an inactive tab stays strip colour"
        );
        assert_eq!(
            pixel_at(&renderer, second.x + 50.0, 16.0),
            renderer.theme.tab_active,
            "the active tab is filled with the terminal background"
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
        // there, and the corner is antialiased: a pixel on the curve is a blend
        // of band and tab, never one or the other.
        assert_eq!(pixel_at(&renderer, body.x, body.y), theme.tab_bar);
        assert_eq!(pixel_at(&renderer, body.x + 10.0, body.y), theme.tab_active);
        let corner = pixel_at(&renderer, body.x + 4.0, body.y);
        assert!(
            corner.r > theme.tab_active.r && corner.r < theme.tab_bar.r,
            "the corner must fade into the band, got {corner:?}"
        );

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
        // each tab is 2 px wide while the icon inset and glyph alone want 26.
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

        // The icon slot starts 10 logical px into the tab and is 16 px square,
        // centred in the 32 px body with its one-pixel drop, so its middle row
        // is y=24.
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
        renderer.draw_chrome(
            &layout,
            crate::fonts::UI_SIZE_PX,
            0,
            &[Some(red)],
            &[],
            None,
        );
        // The icon slot starts 10 px into the tab and is 16 px square.
        assert_eq!(
            pixel_at(&renderer, layout.tabs[0].rect.x + 20.0, 24.0),
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4, 1.0)
            .expect("renderer must be constructible")
    }

    fn arabic_renderer() -> Option<Renderer> {
        let fonts = FontSet::load("Consolas", "Arial", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Arial", 14.0, 1.2))
            .ok()?;
        if !fonts.has_arabic() {
            return None;
        }
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4, 1.0).ok()
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
            input_start: None,
            history: 0,
        }
    }

    /// One row holding one cell per `char` of `text`, numbered `cols` wide.
    /// `single_cell_snapshot` cannot back the multi-column spellcheck cases
    /// because it hard-codes `cols: 1` and ignores the cell's width.
    fn text_snapshot(text: &str) -> GridSnapshot {
        let cells: Vec<Cell> = text
            .chars()
            .map(|c| Cell {
                text: c.to_string(),
                width: 1,
                style: Style::default(),
            })
            .collect();
        let cols = cells.len();
        GridSnapshot {
            cols,
            rows: 1,
            lines: vec![RowSnapshot {
                cells,
                wrapped: false,
            }],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                // The block cursor would cover the underline band; the
                // spellcheck cases only care about the decoration pass.
                visible: false,
                blinking: false,
            },
            display_offset: 0,
            input_start: None,
            history: 0,
        }
    }

    /// Sizes the surface to hold `snapshot` in one row plus padding and draws
    /// it through the span-aware entry point.
    fn render_spellcheck(
        renderer: &mut Renderer,
        snapshot: &GridSnapshot,
        spans: &[crate::spellcheck::Misspelling],
    ) {
        resize_and_draw(renderer, snapshot, |renderer, maps| {
            renderer.draw_with_spellcheck(snapshot, maps, None, None, spans);
        });
    }

    /// Resizes and draws `snapshot` with one identity `VisualMap` per row,
    /// letting `paint` choose the entry point.
    fn resize_and_draw(
        renderer: &mut Renderer,
        snapshot: &GridSnapshot,
        paint: impl FnOnce(&mut Renderer, &[VisualMap]),
    ) {
        let width = renderer.padding * 2 + renderer.fonts.cell_w * snapshot.cols as u32 + 8;
        let height = renderer.padding * 2 + renderer.fonts.cell_h * 2;
        renderer.resize(width, height);
        let maps: Vec<VisualMap> = (0..snapshot.lines.len())
            .map(|_| VisualMap::identity(snapshot.cols))
            .collect();
        paint(renderer, &maps);
    }

    /// The pixel at the bottom row of a cell, inside the underline band.
    fn underline_pixel(renderer: &Renderer, col: usize) -> Rgb {
        let x = renderer.padding as f32 + col as f32 * renderer.fonts.cell_w as f32 + 1.0;
        let y = renderer.grid_top() + renderer.fonts.cell_h as f32 - 1.0;
        pixel_at(renderer, x, y)
    }

    #[test]
    fn draws_underline_under_a_span() {
        let mut renderer = default_renderer();
        let snapshot = text_snapshot("ab");
        let spans = vec![crate::spellcheck::Misspelling {
            row: 0,
            col: 0,
            len: 2,
        }];
        render_spellcheck(&mut renderer, &snapshot, &spans);

        assert_eq!(
            underline_pixel(&renderer, 0),
            renderer.theme.spellcheck_underline,
            "the span must paint the bottom of its first cell"
        );
        assert_eq!(
            underline_pixel(&renderer, 1),
            renderer.theme.spellcheck_underline,
            "the span must cover its second cell too"
        );
        // Only the bottom band is painted, not the whole cell.
        assert_ne!(
            pixel_at(
                &renderer,
                renderer.padding as f32 + 1.0,
                renderer.grid_top() + renderer.fonts.cell_h as f32 / 2.0,
            ),
            renderer.theme.spellcheck_underline,
            "the middle of the cell must stay undecorated"
        );
    }

    #[test]
    fn underlines_the_shifted_row_when_scrolled() {
        let mut renderer = default_renderer();
        let mut snapshot = text_snapshot("ab");
        let second = RowSnapshot {
            cells: snapshot.lines[0].cells.clone(),
            wrapped: false,
        };
        snapshot.lines.push(second);
        snapshot.rows = 2;
        snapshot.display_offset = 1;

        let spans = vec![crate::spellcheck::Misspelling {
            row: 0,
            col: 0,
            len: 2,
        }];
        render_spellcheck(&mut renderer, &snapshot, &spans);

        // The span's line 0 is one row up in history, so it must land on the
        // second visible row once the offset is added.
        let x = renderer.padding as f32 + 1.0;
        let cell_h = renderer.fonts.cell_h as f32;
        assert_eq!(
            pixel_at(&renderer, x, renderer.grid_top() + 2.0 * cell_h - 1.0),
            renderer.theme.spellcheck_underline,
            "the shifted span must paint the second visible row"
        );
        assert_eq!(
            pixel_at(&renderer, x, renderer.grid_top() + cell_h - 1.0),
            renderer.theme.background,
            "the first visible row must stay clean"
        );
    }

    #[test]
    fn draws_nothing_without_spans() {
        let mut renderer = default_renderer();
        let snapshot = text_snapshot("ab");
        render_spellcheck(&mut renderer, &snapshot, &[]);
        let without = renderer.pixmap.data().to_vec();
        assert_eq!(
            underline_pixel(&renderer, 0),
            renderer.theme.background,
            "no span means no decoration in the band"
        );

        // `draw` delegates to the span-aware pass with an empty slice, so the
        // two entry points must agree pixel for pixel.
        resize_and_draw(&mut renderer, &snapshot, |renderer, maps| {
            renderer.draw(&snapshot, maps, None, None);
        });
        assert_eq!(
            renderer.pixmap.data(),
            without.as_slice(),
            "draw and draw_with_spellcheck(&[]) agree"
        );
    }

    #[test]
    fn underlines_only_the_covered_columns() {
        let mut renderer = default_renderer();
        let snapshot = text_snapshot("abcd");
        let spans = vec![crate::spellcheck::Misspelling {
            row: 0,
            col: 1,
            len: 2,
        }];
        render_spellcheck(&mut renderer, &snapshot, &spans);

        assert_eq!(underline_pixel(&renderer, 0), renderer.theme.background);
        assert_eq!(
            underline_pixel(&renderer, 1),
            renderer.theme.spellcheck_underline
        );
        assert_eq!(
            underline_pixel(&renderer, 2),
            renderer.theme.spellcheck_underline
        );
        assert_eq!(underline_pixel(&renderer, 3), renderer.theme.background);
    }

    #[test]
    fn skips_spans_outside_the_row() {
        let mut renderer = default_renderer();
        let snapshot = text_snapshot("abcd");
        render_spellcheck(&mut renderer, &snapshot, &[]);
        let baseline = renderer.pixmap.data().to_vec();

        let spans = vec![
            crate::spellcheck::Misspelling {
                row: 0,
                col: 99,
                len: 4,
            },
            crate::spellcheck::Misspelling {
                row: 5,
                col: 0,
                len: 4,
            },
        ];
        render_spellcheck(&mut renderer, &snapshot, &spans);
        assert_eq!(
            renderer.pixmap.data(),
            baseline.as_slice(),
            "out-of-range spans must paint nothing"
        );
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
            input_start: None,
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
            input_start: None,
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
        renderer.draw(&snapshot, &[VisualMap::identity(4)], None, None);
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
        renderer.draw(&snapshot, &maps, None, None);
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
        renderer.draw(&snapshot, &maps, None, None);
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
        renderer.draw(&snapshot, &[map], None, None);
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
        renderer.draw(&snapshot, &[map], None, None);
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
        renderer.draw(&snapshot, &maps, Some(&selection), None);
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
    fn selection_inverts_the_glyphs_against_the_band() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 4, cell_h * 2);
        let snapshot = row_snapshot(vec![Cell {
            text: "M".into(),
            width: 1,
            style: Style::default(),
        }]);
        let mut selection = Selection::new(LogicalPos { row: 0, col: 0 });
        selection.update(LogicalPos { row: 0, col: 0 });
        let maps = [VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        renderer.draw(&snapshot, &maps, Some(&selection), None);
        renderer.sync();

        let width = renderer.pixmap.width() as usize;
        let band = packed(renderer.theme.selection);
        let left = renderer.padding();
        let (mut banded, mut darkest) = (0, 255u8);
        for y in renderer.padding()..renderer.padding() + cell_h {
            for x in left..left + cell_w {
                let pixel = renderer.pixels()[y as usize * width + x as usize];
                if pixel == band {
                    banded += 1;
                } else {
                    // Only the glyph can differ from the band inside the cell.
                    darkest = darkest.min((pixel & 0xFF) as u8);
                }
            }
        }
        assert!(banded > 0, "the selection must fill the cell with the band");
        // WT inverts the glyph against the band instead of tinting it, so the
        // ink inside a selected cell is the band's contrast colour, not the
        // cell's `#CCCCCC` foreground.
        assert!(
            darkest < 60,
            "selected glyph ink must be dark on the white band, darkest {darkest}"
        );
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
            input_start: None,
            history: 0,
        };
        let maps: Vec<VisualMap> = (0..2).map(|_| VisualMap::identity(3)).collect();
        let mut selection = Selection::block(LogicalPos { row: 0, col: 0 });
        selection.update(LogicalPos { row: 1, col: 1 });
        renderer.draw(&snapshot, &maps, Some(&selection), None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(
            &single_cell_snapshot(cell),
            &[VisualMap::identity(1)],
            None,
            None,
        );
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
        renderer.draw(
            &single_cell_snapshot(cell),
            &[VisualMap::identity(1)],
            None,
            None,
        );
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
        renderer.draw(
            &single_cell_snapshot(cell),
            &[VisualMap::identity(1)],
            None,
            None,
        );
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
        renderer.draw(
            &single_cell_snapshot(cell),
            &[VisualMap::identity(1)],
            None,
            None,
        );
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
        renderer.draw(
            &single_cell_snapshot(cell),
            &[VisualMap::identity(1)],
            None,
            None,
        );
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
        renderer.draw(&snapshot, &maps, None, None);
        renderer.sync();

        let geom = crate::scrollbar::geometry(
            renderer.pixmap.width(),
            renderer.padding,
            renderer.fonts.cell_h,
            crate::scrollbar::BarState {
                rows: snapshot.rows,
                history: snapshot.history,
                offset: snapshot.display_offset,
                hovered: false,
            },
            renderer.scale_factor(),
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
        renderer.draw(&snapshot, &maps, None, None);
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
        renderer.draw(&snapshot, &maps, None, None);
        renderer.sync();

        let rest = crate::scrollbar::geometry(
            renderer.pixmap.width(),
            renderer.padding,
            renderer.fonts.cell_h,
            crate::scrollbar::BarState {
                rows: snapshot.rows,
                history: snapshot.history,
                offset: snapshot.display_offset,
                hovered: false,
            },
            renderer.scale_factor(),
        )
        .expect("history should show a scrollbar");
        let hovered = crate::scrollbar::geometry(
            renderer.pixmap.width(),
            renderer.padding,
            renderer.fonts.cell_h,
            crate::scrollbar::BarState {
                rows: snapshot.rows,
                history: snapshot.history,
                offset: snapshot.display_offset,
                hovered: true,
            },
            renderer.scale_factor(),
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
            renderer.draw(&snapshot, &[VisualMap::identity(1)], None, None);
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
            input_start: None,
            history: 0,
        };
        let maps = vec![VisualMap::identity(cols); rows];

        let ui_px = crate::fonts::UI_SIZE_PX;
        let layout = TabBarLayout::new(width as f32, TAB_STRIP_HEIGHT, &["one", "two"]);
        let menu = {
            use crate::ui::menu::{MenuAction, MenuItem, MenuLayout};
            let items = vec![
                MenuItem::fixed(
                    MenuAction::Settings,
                    "Settings",
                    "",
                    crate::fonts::GLYPH_SETTINGS,
                ),
                MenuItem::fixed(MenuAction::About, "About", "", crate::fonts::GLYPH_HELP),
            ];
            MenuLayout::new(layout.chevron, items, width as f32, height as f32)
        };

        // Warm the glyph atlas so the measurement covers drawing, not discovery.
        renderer.draw(&snapshot, &maps, None, None);
        draw_chrome(&mut renderer, &layout, ui_px, 0, None);
        renderer.draw_menu(&menu, &[]);

        const FRAMES: usize = 40;
        let mut render_samples = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let start = Instant::now();
            renderer.draw(&snapshot, &maps, None, None);
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
            renderer.draw(&snapshot, &maps, None, None);
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
        renderer.draw(&snapshot, &maps, None, None);
        draw_chrome(&mut renderer, &layout, ui_px, 0, None);
        renderer.draw_menu(&menu, &[]);
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
