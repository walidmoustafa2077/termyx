//! Painting for the `⌄` dropdown.

use super::{ui_text, Renderer};
use crate::ui::menu::{MenuLayout, MENU_ROW_HEIGHT};

impl Renderer {
    /// Paints the dropdown described by `menu` over whatever is already on the
    /// surface. `ui_px` is the physical chrome em size, the same one the tab
    /// strip uses, so menu text and strip text cannot drift apart.
    pub fn draw_menu(&mut self, menu: &MenuLayout, ui_px: f32) {
        if menu.rect.w <= 0.0 || menu.rect.h <= 0.0 {
            return;
        }
        // The panel was laid out at the strip's scale; its row height carries
        // that factor, so the painter can scale its insets without asking the
        // caller for a second one.
        let k = menu.rows.first().map_or(1.0, |row| row.h / MENU_ROW_HEIGHT);
        let foreground = self.theme.foreground;
        let muted = self.theme.indexed(8);

        let rect = menu.rect;
        self.fill_rect(rect.x, rect.y, rect.w, rect.h, self.theme.menu_background);
        for separator in &menu.separators {
            self.fill_rect(
                rect.x + 8.0 * k,
                *separator,
                (rect.w - 16.0 * k).max(0.0),
                1.0,
                self.theme.tab_border,
            );
        }

        for (index, (row, item)) in menu.rows.iter().zip(&menu.items).enumerate() {
            if menu.hovered == Some(index) {
                self.fill_rect(row.x, row.y, row.w, row.h, self.theme.menu_hover);
            }
            ui_text::draw_text(
                self,
                &item.label,
                row.x + 26.0 * k,
                row.y + 9.0 * k,
                ui_px,
                foreground,
                (row.w - 100.0 * k).max(0.0),
            );
            if item.shortcut.is_empty() {
                continue;
            }
            let width = self.fonts().ui_width(&item.shortcut, ui_px);
            ui_text::draw_text(
                self,
                &item.shortcut,
                row.x + row.w - width - 4.0 * k,
                row.y + 9.0 * k,
                ui_px,
                muted,
                width + 4.0 * k,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::menu::{MenuAction, MenuItem, MenuLayout};
    use crate::ui::tab_bar::Rect;

    fn menu() -> MenuLayout {
        let anchor = Rect {
            x: 140.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
        };
        MenuLayout::new(
            anchor,
            vec![
                MenuItem::profile(0, "Windows PowerShell", "Ctrl+Shift+1"),
                MenuItem::profile(1, "Command Prompt", "Ctrl+Shift+2"),
                MenuItem::fixed(MenuAction::Settings, "Settings", "Ctrl+,"),
            ],
            600.0,
            400.0,
        )
    }

    #[test]
    fn the_panel_paints_its_background_behind_every_row() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let menu = menu();
        renderer.draw_menu(&menu, crate::fonts::UI_SIZE_PX);
        let background = renderer.theme.menu_background;
        // The inset is clear of every row, so only the panel shows there.
        assert_eq!(
            pixel_at(&renderer, menu.rect.x + 1.0, menu.rect.y + 1.0),
            background
        );
        assert_eq!(
            pixel_at(
                &renderer,
                menu.rect.x + menu.rect.w - 2.0,
                menu.rect.y + menu.rect.h - 2.0
            ),
            background
        );
    }

    #[test]
    fn the_hovered_row_is_filled_and_its_neighbour_is_not() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let mut menu = menu();
        menu.hovered = Some(0);
        renderer.draw_menu(&menu, crate::fonts::UI_SIZE_PX);
        let hover = renderer.theme.menu_hover;
        let background = renderer.theme.menu_background;
        let hovered = menu.rows[0];
        let plain = menu.rows[1];
        assert_eq!(
            pixel_at(&renderer, hovered.x + 4.0, hovered.y + hovered.h / 2.0),
            hover,
            "the row under the pointer is filled"
        );
        assert_eq!(
            pixel_at(&renderer, plain.x + 4.0, plain.y + plain.h / 2.0),
            background,
            "a row the pointer is not on keeps the panel colour"
        );
    }

    #[test]
    fn a_row_paints_its_label_and_its_shortcut_on_the_right() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let menu = menu();
        renderer.draw_menu(&menu, crate::fonts::UI_SIZE_PX);
        let row = menu.rows[0];
        // The label is left-aligned and the shortcut right-aligned, so both
        // halves of the row carry ink when the glyphs are drawn.
        assert!(
            ink_in(&renderer, left_half(row)) > 0,
            "the label must paint in the row"
        );
        assert!(
            ink_in(&renderer, right_half(row)) > 0,
            "the shortcut must paint on the right"
        );
        assert_eq!(
            ink_in(
                &renderer,
                Rect {
                    x: row.x + row.w - 2.0,
                    y: row.y,
                    w: 2.0,
                    h: row.h,
                }
            ),
            0,
            "the shortcut keeps clear of the row's right edge"
        );
    }

    #[test]
    fn the_rule_between_the_groups_paints_a_line() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let menu = menu();
        renderer.draw_menu(&menu, crate::fonts::UI_SIZE_PX);
        assert_eq!(menu.separators.len(), 1);
        let rule = menu.separators[0];
        assert_eq!(
            pixel_at(&renderer, menu.rect.x + 40.0, rule),
            renderer.theme.tab_border
        );
    }

    fn left_half(row: Rect) -> Rect {
        Rect {
            x: row.x,
            w: row.w / 2.0,
            ..row
        }
    }

    fn right_half(row: Rect) -> Rect {
        Rect {
            x: row.x + row.w / 2.0,
            w: row.w / 2.0,
            ..row
        }
    }

    /// Pixels in `rect` that are neither the panel nor the hover fill: glyph
    /// antialiasing makes every ink pixel a blend of the two.
    fn ink_in(renderer: &Renderer, rect: Rect) -> usize {
        let background = renderer.theme.menu_background;
        let hover = renderer.theme.menu_hover;
        (rect.y as u32..(rect.y + rect.h) as u32)
            .flat_map(|y| (rect.x as u32..(rect.x + rect.w) as u32).map(move |x| (x, y)))
            .filter(|(x, y)| {
                let pixel = pixel_at(renderer, *x as f32, *y as f32);
                pixel != background && pixel != hover
            })
            .count()
    }

    fn pixel_at(renderer: &Renderer, x: f32, y: f32) -> crate::theme::Rgb {
        let width = renderer.pixmap.width() as usize;
        let index = (y as usize * width + x as usize) * 4;
        let data = renderer.pixmap.data();
        crate::theme::Rgb::new(data[index], data[index + 1], data[index + 2])
    }
}
