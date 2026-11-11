//! Painting for the `⌄` dropdown.

use super::{ui_text, Renderer};
use crate::icon::TabIcon;
use crate::ui::menu::{
    MenuIcon, MenuLayout, MENU_HOVER_INSET, MENU_HOVER_RADIUS, MENU_ICON_INSET, MENU_ICON_SIZE,
    MENU_LABEL_INSET, MENU_RADIUS, MENU_RIGHT_INSET, MENU_ROW_HEIGHT, MENU_SEPARATOR_INSET,
    MENU_TEXT_PX,
};
use crate::ui::tab_bar::Rect;

impl Renderer {
    /// Paints the dropdown described by `menu` over whatever is already on the
    /// surface.
    ///
    /// `icons` holds one entry per item, resolved by the window from its icon
    /// cache. The panel cannot ask the shell API itself without blocking the
    /// paint, and Windows Terminal shows every profile's own icon in its
    /// flyout, so the bitmap has to come from the owner of that cache.
    pub fn draw_menu(&mut self, menu: &MenuLayout, icons: &[Option<TabIcon>]) {
        if menu.rect.w <= 0.0 || menu.rect.h <= 0.0 {
            return;
        }
        // The panel was laid out at the strip's scale; its row height carries
        // that factor, so the painter can scale its insets without asking the
        // caller for a second one.
        let k = menu.rows.first().map_or(1.0, |row| row.h / MENU_ROW_HEIGHT);
        let text_px = MENU_TEXT_PX * k;
        let foreground = self.theme.foreground;
        let hint = self.theme.menu_shortcut;

        let rect = menu.rect;
        self.fill_round_rect(rect, MENU_RADIUS * k, self.theme.menu_background);
        for separator in &menu.separators {
            self.fill_rect(
                rect.x + MENU_SEPARATOR_INSET * k,
                *separator,
                (rect.w - 2.0 * MENU_SEPARATOR_INSET * k).max(0.0),
                1.0,
                self.theme.menu_separator,
            );
        }

        let ui = crate::fonts::UI_FONT_ID;
        for (index, (row, item)) in menu.rows.iter().zip(&menu.items).enumerate() {
            if menu.hovered == Some(index) {
                // A rounded pill inset from the row, not a full-width wash: the
                // measured flyout keeps the panel's edge visible around it.
                let pill = Rect {
                    x: row.x + MENU_HOVER_INSET * k,
                    y: row.y,
                    w: (row.w - 2.0 * MENU_HOVER_INSET * k).max(1.0),
                    h: row.h,
                };
                self.fill_round_rect(pill, MENU_HOVER_RADIUS * k, self.theme.menu_hover);
            }

            let icon_size = MENU_ICON_SIZE * k;
            let icon_x = row.x + MENU_ICON_INSET * k;
            let centre = row.y + row.h / 2.0;
            match &item.icon {
                MenuIcon::Program(_) => {
                    if let Some(Some(icon)) = icons.get(index) {
                        super::blit_tab_icon(
                            &mut self.pixmap,
                            icon,
                            icon_x,
                            centre - icon_size / 2.0,
                            icon_size,
                        );
                    }
                }
                MenuIcon::Glyph(glyph) => {
                    let box_rect = Rect {
                        x: icon_x,
                        y: centre - icon_size / 2.0,
                        w: icon_size,
                        h: icon_size,
                    };
                    ui_text::draw_text_centred(self, glyph, box_rect, icon_size, foreground);
                }
            }

            // Measure the hint first: its width decides how much room the label
            // may use, so a long shortcut cannot be painted over.
            let shortcut_width = if item.shortcut.is_empty() {
                None
            } else {
                Some(self.fonts().width_in(ui, &item.shortcut, text_px))
            };
            let label_style = ui_text::TextStyle {
                // Windows Terminal draws the profile `+` would open semibold.
                face: if item.default {
                    crate::fonts::SEMIBOLD_FONT_ID
                } else {
                    ui
                },
                px: text_px,
                color: foreground,
            };
            ui_text::draw_text_ellipsis_ink_centred(
                self,
                &item.label,
                row.x + MENU_LABEL_INSET * k,
                centre,
                label_style,
                label_width(row.w, shortcut_width, k),
            );
            let Some(width) = shortcut_width else {
                continue;
            };
            ui_text::draw_text_ink_centred(
                self,
                &item.shortcut,
                row.x + row.w - width - MENU_RIGHT_INSET * k,
                centre,
                ui_text::TextStyle {
                    face: ui,
                    px: text_px,
                    color: hint,
                },
                width + MENU_RIGHT_INSET * k,
            );
        }
    }
}

/// Width the label may paint in, leaving the shortcut column clear.
///
/// `shortcut_width` is `None` for a row without a hint, which then only needs
/// the panel's right inset. The label always keeps a positive width, even when
/// the shortcut alone would eat the whole row.
fn label_width(row_width: f32, shortcut_width: Option<f32>, k: f32) -> f32 {
    let left = MENU_LABEL_INSET * k;
    let right = match shortcut_width {
        Some(width) => width + MENU_RIGHT_INSET * k,
        None => MENU_RIGHT_INSET * k,
    };
    (row_width - left - right).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::menu::{MenuAction, MenuItem, MenuLayout};
    use crate::ui::tab_bar::TAB_STRIP_HEIGHT;

    fn menu() -> MenuLayout {
        let anchor = Rect {
            x: 140.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        MenuLayout::new(
            anchor,
            vec![
                MenuItem::profile(0, "Windows PowerShell", "Ctrl+Shift+1", "powershell"),
                MenuItem::profile(1, "Command Prompt", "Ctrl+Shift+2", "cmd"),
                MenuItem::fixed(
                    MenuAction::Settings,
                    "Settings",
                    "Ctrl+,",
                    crate::fonts::GLYPH_SETTINGS,
                ),
            ],
            600.0,
            400.0,
        )
    }

    fn no_icons(menu: &MenuLayout) -> Vec<Option<TabIcon>> {
        vec![None; menu.items.len()]
    }

    #[test]
    fn the_panel_paints_its_background_behind_every_row() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let menu = menu();
        renderer.draw_menu(&menu, &no_icons(&menu));
        let background = renderer.theme.menu_background;
        // Mid-edge samples: the panel's corners are rounded, so the inset is
        // only clear of every row at the middle of an edge.
        assert_eq!(
            pixel_at(
                &renderer,
                menu.rect.x + menu.rect.w / 2.0,
                menu.rect.y + 1.0
            ),
            background
        );
        assert_eq!(
            pixel_at(
                &renderer,
                menu.rect.x + menu.rect.w / 2.0,
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
        renderer.draw_menu(&menu, &no_icons(&menu));
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
        renderer.draw_menu(&menu, &no_icons(&menu));
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
                    // Mid-row only: the panel's rounded corner eats into the
                    // top and bottom edges of the first row.
                    y: row.y + row.h / 2.0 - 4.0,
                    w: 2.0,
                    h: 8.0,
                }
            ),
            0,
            "the shortcut keeps clear of the row's right edge"
        );
    }

    #[test]
    fn the_icon_column_shows_the_program_bitmap_and_the_command_glyph() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let menu = menu();
        let mut icons = no_icons(&menu);
        // An opaque 16 px bitmap stands in for the shell icon the window
        // resolves: the painter must blit it into the row's icon column.
        icons[0] = Some(TabIcon {
            rgba: vec![255; 16 * 16 * 4],
            size: 16,
        });
        renderer.draw_menu(&menu, &icons);
        let row = menu.rows[0];
        let k = row.h / MENU_ROW_HEIGHT;
        let column = Rect {
            x: row.x + MENU_ICON_INSET * k,
            y: row.y + 2.0,
            w: MENU_ICON_SIZE * k,
            h: row.h - 4.0,
        };
        assert!(
            ink_in(&renderer, column) > 0,
            "a profile row shows its shell's icon"
        );
        // The Settings row has no bitmap, so its icon-face glyph must paint
        // instead of leaving an empty column.
        let glyph_row = menu.rows[2];
        let glyph_column = Rect {
            x: glyph_row.x + MENU_ICON_INSET * k,
            y: glyph_row.y + 2.0,
            w: MENU_ICON_SIZE * k,
            h: glyph_row.h - 4.0,
        };
        assert!(
            ink_in(&renderer, glyph_column) > 0,
            "a command row shows its glyph"
        );
    }

    #[test]
    fn the_rule_between_the_groups_paints_a_line() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let menu = menu();
        renderer.draw_menu(&menu, &no_icons(&menu));
        assert_eq!(menu.separators.len(), 1);
        let rule = menu.separators[0];
        assert_eq!(
            pixel_at(&renderer, menu.rect.x + 40.0, rule),
            renderer.theme.menu_separator
        );
    }

    #[test]
    fn a_long_shortcut_keeps_the_label_out_of_its_column() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(600, 400);
        let anchor = Rect {
            x: 140.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let shortcut = "Ctrl+Shift+Alt+F12";
        let menu = MenuLayout::new(
            anchor,
            vec![MenuItem::fixed(
                MenuAction::Settings,
                "Windows PowerShell",
                shortcut,
                crate::fonts::GLYPH_SETTINGS,
            )],
            600.0,
            400.0,
        );
        renderer.draw_menu(&menu, &no_icons(&menu));
        let row = menu.rows[0];
        let k = row.h / MENU_ROW_HEIGHT;
        let text_px = MENU_TEXT_PX * k;
        let shortcut_width = renderer
            .fonts()
            .width_in(crate::fonts::UI_FONT_ID, shortcut, text_px);
        let label_width = label_width(row.w, Some(shortcut_width), k);
        let label_right = row.x + MENU_LABEL_INSET * k + label_width;
        let shortcut_left = row.x + row.w - shortcut_width - MENU_RIGHT_INSET * k;
        assert!(
            label_right <= shortcut_left,
            "the label's clip must end before the shortcut starts"
        );
        let gap = Rect {
            x: label_right,
            y: row.y,
            w: shortcut_left - label_right,
            h: row.h,
        };
        assert_eq!(
            ink_in(&renderer, gap),
            0,
            "a long shortcut must not be painted over by the label"
        );
        let column = Rect {
            x: shortcut_left,
            y: row.y,
            w: shortcut_width,
            h: row.h,
        };
        assert!(
            ink_in(&renderer, column) > 0,
            "the shortcut still paints in its own column"
        );
    }

    #[test]
    fn a_shortcut_wider_than_the_row_leaves_the_label_some_room() {
        assert_eq!(label_width(100.0, Some(200.0), 1.0), 1.0);
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
