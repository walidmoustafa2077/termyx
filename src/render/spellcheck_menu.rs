//! Painting for the spellcheck suggestion menu.
//!
//! The geometry comes from `ui::spellcheck_menu`; this module only turns those
//! rectangles into pixels, so the painter and the hit test cannot disagree.

// Task 6 wires this module into the app; the allow disappears with it.
#![allow(dead_code)]

use super::ui_text;
use super::Renderer;
use crate::fonts::UI_FONT_ID;
use crate::ui::spellcheck_menu::{
    MenuItem, SpellcheckMenu, SPELLCHECK_HOVER_RADIUS, SPELLCHECK_LABEL_INSET, SPELLCHECK_RADIUS,
    SPELLCHECK_RIGHT_INSET, SPELLCHECK_TEXT_PX,
};
use crate::ui::tab_bar::Rect;

// The window's click handler calls this; the tests pin the paint here.
impl Renderer {
    /// Paints `menu` over whatever is already on the surface.
    ///
    /// The caller passes the anchor cell's rectangle and the viewport it must
    /// stay inside, both in physical pixels; the painter lays the panel out at
    /// its own scale, so the window never carries geometry it does not use.
    pub fn draw_spellcheck_menu(&mut self, menu: &SpellcheckMenu, anchor: Rect, viewport: Rect) {
        let k = self.scale_factor;
        let layout = menu.layout(anchor, viewport, k);
        if layout.panel.w <= 0.0 || layout.panel.h <= 0.0 {
            return;
        }
        self.fill_round_rect(
            layout.panel,
            SPELLCHECK_RADIUS * k,
            self.theme.menu_background,
        );
        for (index, item) in menu.items.iter().enumerate() {
            let Some(row) = layout.rows.get(index).copied() else {
                continue;
            };
            // The rows are already inset, so the pill fills the row rect and
            // the panel's padding supplies the breathing room around it.
            let actionable = menu.enabled || !matches!(item, MenuItem::Suggest(_));
            if actionable && menu.hovered == Some(index) {
                self.fill_round_rect(row, SPELLCHECK_HOVER_RADIUS * k, self.theme.menu_hover);
            }
            // One ink per row: a suggestion the cursor has left reads dimmed,
            // while the two actions stay at full contrast.
            let ink = if actionable {
                self.theme.foreground
            } else {
                self.theme.menu_disabled
            };
            ui_text::draw_text_ellipsis_ink_centred(
                self,
                item.label(),
                row.x + SPELLCHECK_LABEL_INSET * k,
                row.y + row.h / 2.0,
                ui_text::TextStyle {
                    face: UI_FONT_ID,
                    px: SPELLCHECK_TEXT_PX * k,
                    color: ink,
                },
                row.w - (SPELLCHECK_LABEL_INSET + SPELLCHECK_RIGHT_INSET) * k,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Rgb;

    fn menu_renderer() -> Renderer {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(800, 600);
        renderer
    }

    fn viewport() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        }
    }

    fn anchor() -> Rect {
        Rect {
            x: 40.0,
            y: 40.0,
            w: 60.0,
            h: 18.0,
        }
    }

    fn menu() -> SpellcheckMenu {
        SpellcheckMenu::new(
            2,
            4,
            "teh".to_string(),
            vec!["the".to_string(), "tea".to_string()],
            true,
            None,
        )
    }

    /// The pixmap pixel at a physical coordinate. Mirrors the helper in
    /// `render::tests`, which is private to that module.
    fn pixel_at(renderer: &Renderer, x: f32, y: f32) -> Rgb {
        let width = renderer.pixmap.width() as usize;
        let index = (y as usize * width + x as usize) * 4;
        let data = renderer.pixmap.data();
        Rgb::new(data[index], data[index + 1], data[index + 2])
    }

    /// The brightest red channel in `rect`: every ink this menu paints is a
    /// neutral grey, so one channel reads the whole tone. The panel's own fill
    /// is neutral too, so a peak above it can only be label ink.
    fn peak_red(renderer: &Renderer, rect: Rect) -> u8 {
        (rect.y as u32..(rect.y + rect.h) as u32)
            .flat_map(|y| (rect.x as u32..(rect.x + rect.w) as u32).map(move |x| (x, y)))
            .map(|(x, y)| pixel_at(renderer, x as f32, y as f32).r)
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn paints_the_panel_over_the_background() {
        let mut renderer = menu_renderer();
        let menu = menu();
        renderer.draw_spellcheck_menu(&menu, anchor(), viewport());
        let layout = menu.layout(anchor(), viewport(), renderer.scale_factor);
        // The probe sits in the panel's right inset, clear of the rounded
        // corners and of every row's label.
        let probe = pixel_at(
            &renderer,
            layout.panel.x + layout.panel.w - 3.0,
            layout.panel.y + layout.panel.h / 2.0,
        );
        assert_ne!(
            probe, renderer.theme.background,
            "the panel must cover the terminal background"
        );
        assert_eq!(
            probe, renderer.theme.menu_background,
            "the panel is filled with the menu background"
        );
    }

    #[test]
    fn paints_no_panel_for_an_empty_surface() {
        // A zero-width viewport leaves nothing to paint; the painter must not
        // scribble outside the panel's own rect.
        let mut renderer = menu_renderer();
        let menu = menu();
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
        };
        renderer.draw_spellcheck_menu(&menu, anchor(), viewport);
        assert_eq!(
            pixel_at(&renderer, 10.0, 10.0),
            renderer.theme.background,
            "nothing paints when the viewport has no room"
        );
    }

    #[test]
    fn highlights_only_the_hovered_row() {
        let mut renderer = menu_renderer();
        let mut menu = menu();
        menu.hovered = Some(1);
        renderer.draw_spellcheck_menu(&menu, anchor(), viewport());
        let layout = menu.layout(anchor(), viewport(), renderer.scale_factor);
        let x = layout.panel.x + layout.panel.w - 6.0;
        let hovered = pixel_at(&renderer, x, layout.rows[1].y + layout.rows[1].h / 2.0);
        let plain = pixel_at(&renderer, x, layout.rows[0].y + layout.rows[0].h / 2.0);
        assert_eq!(
            hovered, renderer.theme.menu_hover,
            "the row under the pointer carries the hover pill"
        );
        assert_ne!(hovered, plain, "the neighbouring row must stay unpilled");
    }

    #[test]
    fn dims_suggestions_when_replacement_is_disabled() {
        let mut renderer = menu_renderer();
        // No reason is given on purpose: `new` supplies the cursor hint, which
        // is also the common case this paint has to look right in.
        let menu = SpellcheckMenu::new(
            2,
            4,
            "teh".to_string(),
            vec!["the".to_string()],
            false,
            None,
        );
        renderer.draw_spellcheck_menu(&menu, anchor(), viewport());
        let layout = menu.layout(anchor(), viewport(), renderer.scale_factor);

        // Antialiasing blends each glyph pixel between the panel fill and the
        // row's ink, so no pixel is guaranteed to sit exactly on
        // `menu_disabled`. The peak proves which ink the row was drawn with:
        // the suggestion tops out at the disabled tone, the action above it.
        let suggestion = peak_red(&renderer, layout.rows[0]);
        let action = peak_red(&renderer, layout.rows[1]);
        assert!(
            suggestion > renderer.theme.menu_background.r,
            "the disabled suggestion still paints its label"
        );
        assert!(
            suggestion <= renderer.theme.menu_disabled.r,
            "a suggestion the cursor has left is dimmed (peak {suggestion})"
        );
        assert!(
            action > renderer.theme.menu_disabled.r,
            "an action row keeps the foreground ink (peak {action})"
        );
    }

    #[test]
    fn paints_every_row_of_a_suggestion_free_menu() {
        // Zero suggestions is a normal case: the two actions must still paint
        // and still be told apart from the panel.
        let mut renderer = menu_renderer();
        let menu = SpellcheckMenu::new(0, 0, "teh".to_string(), vec![], true, None);
        renderer.draw_spellcheck_menu(&menu, anchor(), viewport());
        let layout = menu.layout(anchor(), viewport(), renderer.scale_factor);
        assert_eq!(layout.rows.len(), 2);
        for row in &layout.rows {
            assert!(
                peak_red(&renderer, *row) > renderer.theme.menu_background.r,
                "every row paints its label"
            );
        }
    }
}
