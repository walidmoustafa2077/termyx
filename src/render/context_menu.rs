//! Painting for the tab's right-click menu.

use super::{ui_text, Renderer};
use crate::fonts::UI_FONT_ID;
use crate::theme::Rgb;
use crate::ui::context_menu::{ContextItem, ContextMenu, ContextPanel};
use crate::ui::menu::{
    MENU_HOVER_INSET, MENU_HOVER_RADIUS, MENU_ICON_INSET, MENU_ICON_SIZE, MENU_LABEL_INSET,
    MENU_RADIUS, MENU_RIGHT_INSET, MENU_SEPARATOR_INSET, MENU_TEXT_PX,
};
use crate::ui::tab_bar::Rect;

// The window's right-click handler calls this; the tests pin the paint here.
impl Renderer {
    /// Paints the chain of panels `menu` describes over whatever is already on
    /// the surface.
    ///
    /// The model was laid out at the menu's own scale, so the painter sizes
    /// its insets from `menu.scale` without asking for a second factor. A
    /// short window's dropped rows are absent from the layout, so nothing
    /// paints where `hit` cannot land.
    pub fn draw_context_menu(&mut self, menu: &ContextMenu) {
        for panel in &menu.panels {
            if panel.rect.w <= 0.0 || panel.rect.h <= 0.0 {
                continue;
            }
            self.fill_round_rect(
                panel.rect,
                MENU_RADIUS * menu.scale,
                self.theme.menu_background,
            );
            for y in &panel.separators {
                self.fill_rect(
                    panel.rect.x + MENU_SEPARATOR_INSET * menu.scale,
                    *y,
                    (panel.rect.w - 2.0 * MENU_SEPARATOR_INSET * menu.scale).max(0.0),
                    1.0,
                    self.theme.menu_separator,
                );
            }
            for (index, item) in panel.items.iter().enumerate() {
                let Some(row) = panel.rows.get(index) else {
                    continue;
                };
                self.paint_context_row(panel, *row, item, index, menu.scale);
            }
        }
    }

    /// Paints one row of its panel: the hover pill, the icon column, the
    /// label, the shortcut, and a submenu's chevron.
    fn paint_context_row(
        &mut self,
        panel: &ContextPanel,
        row: Rect,
        item: &ContextItem,
        index: usize,
        k: f32,
    ) {
        let centre = row.y + row.h / 2.0;
        // One dim ink covers everything a dead row shows: the reference keeps
        // its placeholders readable but greyed.
        let ink = if item.enabled {
            self.theme.foreground
        } else {
            self.theme.menu_disabled
        };

        // A rounded pill inset from the row, the dropdown's own highlight, but
        // only for a row a click can act on.
        if item.enabled && panel.hovered == Some(index) {
            let pill = Rect {
                x: row.x + MENU_HOVER_INSET * k,
                y: row.y,
                w: (row.w - 2.0 * MENU_HOVER_INSET * k).max(1.0),
                h: row.h,
            };
            self.fill_round_rect(pill, MENU_HOVER_RADIUS * k, self.theme.menu_hover);
        }

        let icon_size = MENU_ICON_SIZE * k;
        let icon = Rect {
            x: row.x + MENU_ICON_INSET * k,
            y: centre - icon_size / 2.0,
            w: icon_size,
            h: icon_size,
        };
        match item.swatch {
            // The chosen color fills the icon column's swatch.
            Some(Some(color)) => self.fill_round_rect(icon, 3.0 * k, color),
            // The Default row keeps the square visible without a fill of its
            // own: an opaque dark swatch would read as a chosen color.
            Some(None) => {
                self.fill_rect(icon.x, icon.y, icon.w, icon.h, self.theme.menu_background);
                self.stroke_rect(icon, self.theme.menu_shortcut);
            }
            // No swatch: the icon-face glyph, or nothing for a row with none.
            None => {
                if !item.glyph.is_empty() {
                    ui_text::draw_text_centred(self, item.glyph, icon, icon_size, ink);
                }
            }
        }

        // Measure first: the shortcut and a submenu chevron both borrow the
        // row's right end, so the label may only paint what they leave.
        let text_px = MENU_TEXT_PX * k;
        let arrow_width = if item.children.is_empty() {
            0.0
        } else {
            icon_size
        };
        let shortcut_width = if item.shortcut.is_empty() {
            None
        } else {
            Some(self.fonts().width_in(UI_FONT_ID, &item.shortcut, text_px))
        };
        let label_span = (row.w
            - MENU_LABEL_INSET * k
            - MENU_RIGHT_INSET * k
            - arrow_width
            - shortcut_width.unwrap_or(0.0))
        .max(1.0);
        ui_text::draw_text_ellipsis_ink_centred(
            self,
            &item.label,
            row.x + MENU_LABEL_INSET * k,
            centre,
            ui_text::TextStyle {
                face: UI_FONT_ID,
                px: text_px,
                color: ink,
            },
            label_span,
        );

        if let Some(width) = shortcut_width {
            ui_text::draw_text_ink_centred(
                self,
                &item.shortcut,
                row.x + row.w - width - MENU_RIGHT_INSET * k,
                centre,
                ui_text::TextStyle {
                    face: UI_FONT_ID,
                    px: text_px,
                    color: if item.enabled {
                        self.theme.menu_shortcut
                    } else {
                        self.theme.menu_disabled
                    },
                },
                width + MENU_RIGHT_INSET * k,
            );
        }

        if !item.children.is_empty() {
            let chevron = Rect {
                x: row.x + row.w - MENU_RIGHT_INSET * k - icon_size,
                y: centre - icon_size / 2.0,
                w: icon_size,
                h: icon_size,
            };
            ui_text::draw_text_centred(
                self,
                crate::fonts::GLYPH_CHEVRON_RIGHT,
                chevron,
                icon_size,
                ink,
            );
        }
    }

    /// Outlines `rect` with a 1 px border: the renderer has no stroke
    /// primitive, so the edge is four fills.
    fn stroke_rect(&mut self, rect: Rect, color: Rgb) {
        self.fill_rect(rect.x, rect.y, rect.w, 1.0, color);
        self.fill_rect(rect.x, rect.y + rect.h - 1.0, rect.w, 1.0, color);
        self.fill_rect(rect.x, rect.y, 1.0, rect.h, color);
        self.fill_rect(rect.x + rect.w - 1.0, rect.y, 1.0, rect.h, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::{GLYPH_MOVE_TAB, GLYPH_SPLIT};
    use crate::ui::context_menu::{ContextItem, ContextMenu, TabCommand};
    use crate::ui::menu::{MENU_ICON_INSET, MENU_ICON_SIZE, MENU_RIGHT_INSET};

    /// The tab card the menu hangs from, at the strip's reference size.
    fn tab() -> Rect {
        Rect {
            x: 100.0,
            y: 8.0,
            w: 180.0,
            h: 32.0,
        }
    }

    fn menu_renderer() -> Renderer {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(800, 600);
        renderer
    }

    #[test]
    fn a_disabled_row_uses_the_disabled_ink() {
        // Antialiasing blends every glyph pixel with the panel behind it, so
        // the disabled ink shows as the enabled ink's coverage map scaled to
        // a dimmer tone, not as a flat `menu_disabled` swatch. Peaking one
        // channel over the row compares the two maps: the disabled row must
        // top out below the enabled one while still painting ink.
        let mut item = ContextItem::action("Split tab", GLYPH_SPLIT, TabCommand::Split);
        let enabled = ContextMenu::new(tab(), vec![item.clone()], 800.0, 600.0, 1.0);
        item.enabled = false;
        let disabled = ContextMenu::new(tab(), vec![item], 800.0, 600.0, 1.0);

        let mut enabled_renderer = menu_renderer();
        enabled_renderer.draw_context_menu(&enabled);
        let mut disabled_renderer = menu_renderer();
        disabled_renderer.draw_context_menu(&disabled);

        let row = disabled.panels[0].rows[0];
        assert!(
            ink_in(&disabled_renderer, row) > 0,
            "a disabled row still paints its label"
        );
        assert!(
            peak_ink(&disabled_renderer, row) < peak_ink(&enabled_renderer, row),
            "the disabled row's ink must be dimmer than the enabled one"
        );
    }

    #[test]
    fn a_submenu_parent_draws_a_right_chevron() {
        let menu = ContextMenu::new(
            tab(),
            vec![
                ContextItem::action("Split tab", GLYPH_SPLIT, TabCommand::Split),
                ContextItem::submenu(
                    "Move tab",
                    GLYPH_MOVE_TAB,
                    vec![ContextItem::action(
                        "Move right",
                        "R",
                        TabCommand::MoveRight,
                    )],
                ),
            ],
            800.0,
            600.0,
            1.0,
        );
        let mut renderer = menu_renderer();
        renderer.draw_context_menu(&menu);
        let k = menu.scale;
        let column = |row: Rect| Rect {
            // The chevron sits inside the right inset and clear of the panel's
            // rounded corners.
            x: row.x + row.w - MENU_RIGHT_INSET * k - MENU_ICON_SIZE * k + 2.0,
            y: row.y + 2.0,
            w: MENU_ICON_SIZE * k - 2.0,
            h: row.h - 4.0,
        };
        assert!(
            ink_in(&renderer, column(menu.panels[0].rows[1])) > 0,
            "a submenu parent shows the chevron on its right end"
        );
        assert_eq!(
            ink_in(&renderer, column(menu.panels[0].rows[0])),
            0,
            "a plain row keeps the row's right end empty"
        );
    }

    #[test]
    fn a_color_row_fills_its_swatch_at_the_icon_inset() {
        let color = Rgb::new(0, 122, 204);
        let menu = ContextMenu::new(
            tab(),
            vec![ContextItem::swatch("#007ACC", Some(color))],
            800.0,
            600.0,
            1.0,
        );
        let mut renderer = menu_renderer();
        renderer.draw_context_menu(&menu);
        let swatch = swatch_rect(menu.panels[0].rows[0], menu.scale);
        assert_eq!(
            pixel_at(
                &renderer,
                swatch.x + swatch.w / 2.0,
                swatch.y + swatch.h / 2.0
            ),
            color,
            "the chosen color fills the swatch's centre"
        );
    }

    #[test]
    fn the_default_swatch_is_a_bordered_empty_square() {
        let menu = ContextMenu::new(
            tab(),
            vec![ContextItem::swatch("Default", None)],
            800.0,
            600.0,
            1.0,
        );
        let mut renderer = menu_renderer();
        renderer.draw_context_menu(&menu);
        let swatch = swatch_rect(menu.panels[0].rows[0], menu.scale);
        // The centre keeps the panel's colour: an empty swatch must not read
        // as a filled dark swatch a user could mistake for a chosen color.
        assert_eq!(
            pixel_at(
                &renderer,
                swatch.x + swatch.w / 2.0,
                swatch.y + swatch.h / 2.0
            ),
            renderer.theme.menu_background,
            "the Default swatch's centre carries no fill of its own"
        );
        assert_eq!(
            pixel_at(&renderer, swatch.x, swatch.y + swatch.h / 2.0),
            renderer.theme.menu_shortcut,
            "the Default swatch is outlined, not filled"
        );
    }

    /// The swatch square inside a row's icon column.
    fn swatch_rect(row: Rect, k: f32) -> Rect {
        Rect {
            x: row.x + MENU_ICON_INSET * k,
            y: row.y + (row.h - MENU_ICON_SIZE * k) / 2.0,
            w: MENU_ICON_SIZE * k,
            h: MENU_ICON_SIZE * k,
        }
    }

    /// Ink pixels in `rect`: everything that is neither the panel nor a hover
    /// pill, counting glyph antialiasing.
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

    /// The brightest red channel in `rect`: every ink the menu paints is a
    /// neutral grey, so one channel reads the whole tone.
    fn peak_ink(renderer: &Renderer, rect: Rect) -> u8 {
        (rect.y as u32..(rect.y + rect.h) as u32)
            .flat_map(|y| (rect.x as u32..(rect.x + rect.w) as u32).map(move |x| (x, y)))
            .map(|(x, y)| pixel_at(renderer, x as f32, y as f32).r)
            .max()
            .unwrap_or(0)
    }

    fn pixel_at(renderer: &Renderer, x: f32, y: f32) -> Rgb {
        let width = renderer.pixmap.width() as usize;
        let index = (y as usize * width + x as usize) * 4;
        let data = renderer.pixmap.data();
        Rgb::new(data[index], data[index + 1], data[index + 2])
    }
}
