//! Painting for the find bar: the floating panel that hosts the search query,
//! the match count, and the close button.

use super::{ui_text, Renderer};
use crate::find::FindState;
use crate::ui::find_bar::{FindBar, FIND_BAR_HEIGHT};
use crate::ui::tab_bar::TAB_TITLE_SIZE;

impl Renderer {
    /// Paints the find bar over the grid, near the window's top-right corner.
    ///
    /// `bar` is already scaled to physical pixels; its height carries the scale
    /// factor, which the painter re-derives to size its radius, text and glyphs.
    pub fn draw_find_bar(&mut self, bar: &FindBar, find: &FindState) {
        let rect = bar.rect;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let k = rect.h / FIND_BAR_HEIGHT;
        let radius = 4.0 * k;

        self.fill_round_rect(rect, radius, self.theme.menu_background);
        // A one-pixel border, the same recessed tone the dropdown's separators
        // use, so the panel reads as a flyout rather than a hole in the grid.
        let border = self.theme.menu_separator;
        self.fill_rect(rect.x, rect.y, rect.w, 1.0, border);
        self.fill_rect(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, border);
        self.fill_rect(rect.x, rect.y, 1.0, rect.h, border);
        self.fill_rect(rect.x + rect.w - 1.0, rect.y, 1.0, rect.h, border);

        let px = TAB_TITLE_SIZE * k;
        let ui = crate::fonts::UI_FONT_ID;

        // The match count on the right, or "No results" once a non-empty query
        // finds nothing. Empty while the query is empty.
        let count = if find.matches.is_empty() {
            if find.query.is_empty() {
                String::new()
            } else {
                "No results".to_string()
            }
        } else {
            format!("{}/{}", find.current + 1, find.matches.len())
        };
        let count_width = if count.is_empty() {
            0.0
        } else {
            self.fonts().width_in(ui, &count, px)
        };
        let count_gap = if count_width > 0.0 { 8.0 * k } else { 0.0 };

        // The query, or the "Find" placeholder in the shortcut grey while empty.
        let (text, color) = if find.query.is_empty() {
            ("Find", self.theme.menu_shortcut)
        } else {
            (find.query.as_str(), self.theme.foreground)
        };
        let query_max = (bar.field.w - count_width - count_gap).max(1.0);
        ui_text::draw_text_ellipsis_in(
            self,
            text,
            bar.field.x,
            bar.field.y + (bar.field.h - px) * 0.5,
            ui_text::TextStyle {
                face: ui,
                px,
                color,
            },
            query_max,
        );

        if count_width > 0.0 {
            ui_text::draw_text_ink_centred(
                self,
                &count,
                bar.field.x + bar.field.w - count_width,
                bar.field.y + bar.field.h * 0.5,
                ui_text::TextStyle {
                    face: ui,
                    px,
                    color: self.theme.menu_shortcut,
                },
                count_width,
            );
        }

        ui_text::draw_text_centred(
            self,
            crate::fonts::GLYPH_CLOSE,
            bar.close,
            crate::fonts::CONTROL_ICON_PX * k,
            self.theme.menu_shortcut,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::Match;
    use crate::ui::find_bar::FindBar;

    #[test]
    fn the_panel_paints_its_background_and_border() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let bar = FindBar::new(600.0, 0.0, 1.0);
        let find = FindState::default();
        renderer.draw_find_bar(&bar, &find);
        // The top edge paints the border tone, and a point between the left
        // border and the field's left inset stays on the panel fill.
        assert_eq!(
            pixel_at(&renderer, bar.rect.x + bar.rect.w / 2.0, bar.rect.y),
            renderer.theme.menu_separator,
            "the top edge paints the border tone"
        );
        assert_eq!(
            pixel_at(&renderer, bar.rect.x + 4.0, bar.rect.y + bar.rect.h / 2.0),
            renderer.theme.menu_background,
            "the panel interior stays on the panel tone"
        );
    }

    #[test]
    fn a_query_paints_the_match_count() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let bar = FindBar::new(600.0, 0.0, 1.0);
        let mut find = FindState {
            query: "beta".to_string(),
            ..FindState::default()
        };
        find.set_matches(vec![
            Match {
                line: 0,
                start: 0,
                end: 4,
            },
            Match {
                line: 1,
                start: 0,
                end: 4,
            },
            Match {
                line: 2,
                start: 0,
                end: 4,
            },
        ]);
        renderer.draw_find_bar(&bar, &find);
        // The field now carries foreground ink (the query), not the placeholder
        // grey, so the field's left edge differs from an empty bar's.
        let with_query = pixel_at(
            &renderer,
            bar.field.x + 2.0,
            bar.field.y + bar.field.h / 2.0,
        );
        let mut empty_renderer = crate::render::tests::default_renderer();
        empty_renderer.resize(600, 400);
        empty_renderer.draw_find_bar(&bar, &FindState::default());
        let placeholder = pixel_at(
            &empty_renderer,
            bar.field.x + 2.0,
            bar.field.y + bar.field.h / 2.0,
        );
        assert_ne!(
            with_query, placeholder,
            "a typed query must paint foreground ink, not the placeholder"
        );
    }

    fn pixel_at(renderer: &Renderer, x: f32, y: f32) -> crate::theme::Rgb {
        let width = renderer.pixmap.width() as usize;
        let index = (y as usize * width + x as usize) * 4;
        let data = renderer.pixmap.data();
        crate::theme::Rgb::new(data[index], data[index + 1], data[index + 2])
    }
}
