//! The `⌄` dropdown: geometry and the model of its rows.

use crate::ui::tab_bar::{Rect, TAB_STRIP_HEIGHT};

/// Width of the dropdown in logical pixels, at the strip's reference height.
///
/// Measured on Windows Terminal 1.24 at 100 % with the default profile set:
/// the flyout panel spans 388 px. WinUI sizes the panel to its widest row, so a
/// different profile list would move this number.
pub const MENU_WIDTH: f32 = 388.0;
/// Height of one row in logical pixels, at the strip's reference height.
///
/// Measured: eight rows at a 33 px pitch, the same rhythm the default menu
/// keeps through its group rule.
pub const MENU_ROW_HEIGHT: f32 = 33.0;
/// Inset of the panel around its rows, and the room a group rule gets.
pub const MENU_PADDING: f32 = 2.0;
/// Space between the anchor's bottom edge and the panel.
///
/// Measured: the panel's top edge sits 2 px below the strip's rule, the row
/// between them showing the drop shadow.
pub const MENU_GAP: f32 = 2.0;
/// Corner radius of the panel in logical pixels.
pub const MENU_RADIUS: f32 = 8.0;
/// Horizontal inset of a row's icon from the row's left edge.
pub const MENU_ICON_INSET: f32 = 12.0;
/// Size of the square a row's icon occupies.
pub const MENU_ICON_SIZE: f32 = 16.0;
/// Where a row's label starts, from the row's left edge.
pub const MENU_LABEL_INSET: f32 = 42.0;
/// Inset of a shortcut's right edge from the row's right edge.
pub const MENU_RIGHT_INSET: f32 = 10.0;
/// Horizontal inset of the hover pill inside its row.
pub const MENU_HOVER_INSET: f32 = 2.0;
/// Corner radius of the hover pill in logical pixels.
pub const MENU_HOVER_RADIUS: f32 = 4.0;
/// Inset of a group rule from the panel's left and right edges.
pub const MENU_SEPARATOR_INSET: f32 = 8.0;
/// Em size of a row's label and shortcut in logical pixels.
///
/// Measured: the label cap band is 10 rows tall, the same 12 px size as the tab
/// title, not the 14 px chrome size the rows were drawn at before.
pub const MENU_TEXT_PX: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Index into the configured profiles.
    Profile(usize),
    Settings,
    CommandPalette,
    /// Opens the project's support page in the browser.
    Support,
    About,
}

/// What a row shows in its icon column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuIcon {
    /// The shell program whose real icon the row shows. Carried as the program
    /// name because only the window owns the icon cache, and it resolves the
    /// bitmap after the menu's geometry exists.
    Program(String),
    /// A codepoint from the system icon face, for the rows that are commands
    /// rather than shells.
    Glyph(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// Right-aligned key hint; empty when the row has no shortcut.
    pub shortcut: String,
    pub action: MenuAction,
    pub icon: MenuIcon,
    /// Whether this is the profile `+` would open. Windows Terminal draws that
    /// row semibold, so the weight is part of the model, not a paint-time
    /// guess.
    pub default: bool,
}

impl MenuItem {
    pub fn profile(index: usize, label: &str, shortcut: &str, program: &str) -> Self {
        Self {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
            action: MenuAction::Profile(index),
            icon: MenuIcon::Program(program.to_string()),
            default: false,
        }
    }

    pub fn fixed(action: MenuAction, label: &str, shortcut: &str, glyph: &'static str) -> Self {
        Self {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
            action,
            icon: MenuIcon::Glyph(glyph),
            default: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    pub rect: Rect,
    /// One rect per item, top to bottom; empty room is left for a group rule.
    pub rows: Vec<Rect>,
    pub items: Vec<MenuItem>,
    /// y of each rule between groups.
    pub separators: Vec<f32>,
    /// Row under the pointer, filled by the painter and updated by the window.
    /// It is state the painter reads, not part of the geometry, so it is a
    /// field rather than a constructor argument.
    pub hovered: Option<usize>,
}

impl MenuLayout {
    /// Anchored at `anchor`'s top-left corner like a context menu: the panel's
    /// left edge sits on the click and opens downward unless that would run
    /// past the bottom edge, in which case it flips above the point. The left
    /// edge is pulled in near the window's right edge, and the panel shrinks to
    /// the window when the window is narrower than it.
    ///
    /// A press inside the tab strip would otherwise cover the chevron, so such
    /// a point is pushed to the strip's bottom edge first. When neither side
    /// can hold the whole menu, the panel is pinned there and only the rows
    /// that fit entirely are laid out. Dropped rows are absent from `rows`, so
    /// `hit` cannot find them.
    ///
    /// `anchor` is the pointer point; its height carries the DPI scale the
    /// strip was laid out at. Deriving the factor from it keeps the dropdown in
    /// step with the scaled chrome without a fifth argument.
    pub fn new(anchor: Rect, items: Vec<MenuItem>, window_width: f32, window_height: f32) -> Self {
        let k = anchor.h / TAB_STRIP_HEIGHT;
        let width = (MENU_WIDTH * k).min(window_width.max(0.0));
        let row_height = MENU_ROW_HEIGHT * k;
        let padding = MENU_PADDING * k;
        let gap = MENU_GAP * k;
        // A rule separates the profiles from the window commands. Its room is
        // reserved only when such a group follows, so a profile-only menu has
        // no empty band.
        let rule_gap = if items
            .iter()
            .any(|item| !matches!(item.action, MenuAction::Profile(_)))
        {
            padding
        } else {
            0.0
        };
        let full_height = items.len() as f32 * row_height + 2.0 * padding + rule_gap;
        let x = anchor.x.clamp(0.0, (window_width - width).max(0.0));
        // A click on the strip lands on the chevron; opening over it would put
        // a row under the pointer and steal the toggle.
        let floor = if anchor.y < anchor.h {
            anchor.y + anchor.h
        } else {
            0.0
        };
        let top = anchor.y.max(floor);

        let y = if top + full_height <= window_height {
            top
        } else if top - gap - full_height >= floor {
            top - gap - full_height
        } else {
            // Neither side holds the whole menu. Pin it to the floor and keep
            // only the rows that fit; the window never clips a row that `hit`
            // can still reach.
            floor
        };

        let mut rows = Vec::with_capacity(items.len());
        let mut separators = Vec::new();
        let mut row_y = y + padding;
        let mut seen_command = false;
        for item in &items {
            let needs_rule = !matches!(item.action, MenuAction::Profile(_)) && !seen_command;
            let top = if needs_rule { row_y + rule_gap } else { row_y };
            // A row is kept whole: one that would hang over the window's bottom
            // edge is dropped, along with every row after it.
            if top + row_height + padding > window_height {
                break;
            }
            if needs_rule {
                separators.push(row_y);
                row_y = top;
                seen_command = true;
            }
            rows.push(Rect {
                x: x + padding,
                y: row_y,
                w: width - 2.0 * padding,
                h: row_height,
            });
            row_y += row_height;
        }
        let height = (row_y + padding - y).clamp(0.0, (window_height - y).max(0.0));

        Self {
            rect: Rect {
                x,
                y,
                w: width,
                h: height,
            },
            rows,
            items,
            separators,
            hovered: None,
        }
    }

    /// Index of the row at `(x, y)`, or `None` on the panel's inset or in the
    /// gap a rule lives in. Row order matches `items`.
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if !self.rect.contains(x, y) {
            return None;
        }
        self.rows
            .iter()
            .position(|row| x >= row.x && x < row.x + row.w && y >= row.y && y < row.y + row.h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::profile(0, "Windows PowerShell", "Ctrl+Shift+1", "powershell"),
            MenuItem::profile(1, "Command Prompt", "Ctrl+Shift+2", "cmd"),
            MenuItem::fixed(
                MenuAction::Settings,
                "Settings",
                "Ctrl+,",
                crate::fonts::GLYPH_SETTINGS,
            ),
            MenuItem::fixed(MenuAction::About, "About", "", crate::fonts::GLYPH_HELP),
        ]
    }

    fn nine_items() -> Vec<MenuItem> {
        let mut items: Vec<MenuItem> = (0..6)
            .map(|index| MenuItem::profile(index, &format!("Profile {index}"), "", "powershell"))
            .collect();
        items.push(MenuItem::fixed(
            MenuAction::Settings,
            "Settings",
            "Ctrl+,",
            crate::fonts::GLYPH_SETTINGS,
        ));
        items.push(MenuItem::fixed(
            MenuAction::CommandPalette,
            "Command palette",
            "Ctrl+Shift+P",
            crate::fonts::GLYPH_COMMAND_PALETTE,
        ));
        items.push(MenuItem::fixed(
            MenuAction::About,
            "About",
            "",
            crate::fonts::GLYPH_HELP,
        ));
        items
    }

    #[test]
    fn the_menu_opens_at_the_click_point() {
        let anchor = Rect {
            x: 300.0,
            y: 200.0,
            w: 0.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.rect.x, 300.0, "the left edge sits on the click");
        assert_eq!(menu.rect.y, 200.0, "the top edge sits on the click");
        assert_eq!(menu.rows.len(), 4);
    }

    #[test]
    fn a_click_near_the_right_edge_pulls_the_menu_in() {
        let anchor = Rect {
            x: 700.0,
            y: 200.0,
            w: 0.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.rect.x, 800.0 - MENU_WIDTH);
        assert_eq!(menu.rect.y, 200.0);
    }

    #[test]
    fn a_click_on_the_strip_opens_below_it() {
        // The chevron press must not put a row under the pointer.
        let anchor = Rect {
            x: 700.0,
            y: 0.0,
            w: 0.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.rect.y, TAB_STRIP_HEIGHT);
        assert_eq!(menu.hit(700.0, 4.0), None);
    }

    #[test]
    fn a_menu_wider_than_the_window_is_shoved_inside() {
        // The window is narrower than the panel, so only the shrink branch can
        // keep it on screen.
        let window_width = 200.0;
        let anchor = Rect {
            x: 180.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), window_width, 600.0);
        assert_eq!(menu.rect.x, 0.0, "the panel is pulled to the left edge");
        assert!(
            menu.rect.w <= window_width,
            "the panel must shrink to the window width"
        );
    }

    #[test]
    fn a_menu_past_the_bottom_edge_opens_upward() {
        let anchor = Rect {
            x: 0.0,
            y: 560.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert!(
            menu.rect.y < anchor.y,
            "the menu must flip above the anchor"
        );
        assert!(
            menu.rect.y + menu.rect.h <= 600.0,
            "the menu must not leave the bottom edge"
        );
    }

    #[test]
    fn hit_test_maps_rows_in_order() {
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(
            menu.rows.len(),
            menu.items.len(),
            "every item gets exactly one row"
        );
        assert_eq!(
            menu.items[2].action,
            MenuAction::Settings,
            "row 2 belongs to the third item"
        );
        let row = menu.rows[2];
        assert_eq!(
            menu.hit(row.x + 4.0, row.y + 4.0),
            Some(2),
            "the third row is Settings"
        );
        assert_eq!(menu.hit(menu.rect.x + 4.0, menu.rect.y - 8.0), None);
    }

    #[test]
    fn hit_test_skips_the_separator_gap() {
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        let rule = menu.separators[0];
        assert_eq!(
            menu.hit(menu.rect.x + 4.0, rule + 1.0),
            None,
            "a point between two rows is no row at all"
        );
    }

    #[test]
    fn there_is_a_separator_before_the_fixed_rows() {
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.separators.len(), 1);
        assert!(menu.separators[0] > menu.rows[1].y);
        assert!(menu.separators[0] < menu.rows[2].y);
    }

    #[test]
    fn the_menu_scales_with_the_strip_height() {
        // The anchor is the strip's chevron, so a scaled strip must give a
        // scaled dropdown: same geometry, same proportions as the tab strip.
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 48.0,
            h: 48.0,
        };
        let menu = MenuLayout::new(anchor, items(), 1200.0, 800.0);
        let k = 48.0 / TAB_STRIP_HEIGHT;
        assert_eq!(menu.rect.w, MENU_WIDTH * k);
        assert_eq!(menu.rect.y, 48.0);
        assert_eq!(menu.rows[0].h, MENU_ROW_HEIGHT * k);
        assert_eq!(
            menu.hit(menu.rows[3].x + 4.0, menu.rows[3].y + 4.0),
            Some(3)
        );
    }

    #[test]
    fn a_short_window_keeps_the_panel_and_its_rows_inside() {
        // 200 px cannot hold a 9-row panel below or above the strip, so the
        // panel is pinned under the strip and only whole rows are kept.
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, nine_items(), 800.0, 200.0);
        assert!(
            menu.rect.y + menu.rect.h <= 200.0,
            "the panel must not leave the window's bottom edge"
        );
        assert!(
            menu.rows.len() < menu.items.len(),
            "rows that would hang over the edge are dropped"
        );
        for (index, row) in menu.rows.iter().enumerate() {
            assert!(
                row.y + row.h <= 200.0,
                "row {index} must fit entirely in the window"
            );
            assert!(
                row.y >= anchor.y + anchor.h,
                "row {index} must stay below the strip"
            );
            assert_eq!(
                menu.hit(row.x + 4.0, row.y + 4.0),
                Some(index),
                "row {index} must still be reachable"
            );
        }
        assert_eq!(
            menu.hit(menu.rect.x + 4.0, 199.0),
            None,
            "a point past the last row is no row at all"
        );
    }

    #[test]
    fn a_middling_window_keeps_row_zero_below_the_strip() {
        // 330 px still cannot hold the whole panel at once, but the pinned
        // panel now covers most of the window. Row 0 must begin under the
        // strip so a chevron press cannot land on a row.
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let menu = MenuLayout::new(anchor, nine_items(), 800.0, 330.0);
        assert!(
            menu.rect.y >= anchor.y + anchor.h,
            "the panel must not cover the strip"
        );
        assert!(
            menu.rows[0].y >= anchor.y + anchor.h,
            "the first row must stay below the strip"
        );
        assert_eq!(
            menu.hit(anchor.x + 4.0, anchor.y + 4.0),
            None,
            "a chevron press must not resolve to a row"
        );
    }

    #[test]
    fn the_row_carries_its_icon_and_its_default_weight() {
        // The icon and the semibold weight are model data: the painter owns no
        // profile list and cannot guess either.
        let mut items = items();
        items[0].default = true;
        assert_eq!(
            items[0].icon,
            MenuIcon::Program("powershell".to_string()),
            "a profile row shows the shell's own icon"
        );
        assert_eq!(
            items[2].icon,
            MenuIcon::Glyph(crate::fonts::GLYPH_SETTINGS),
            "a command row shows an icon-face glyph"
        );
        assert!(items[0].default, "the row `+` opens is the default one");
        assert!(!items[1].default, "only one row is the default");
    }
}
