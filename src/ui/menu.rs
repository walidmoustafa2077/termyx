//! The `⌄` dropdown: geometry and the model of its rows.

use crate::ui::tab_bar::{Rect, TAB_STRIP_HEIGHT};

/// Width of the dropdown in logical pixels, at the strip's reference height.
pub const MENU_WIDTH: f32 = 280.0;
/// Height of one row in logical pixels, at the strip's reference height.
pub const MENU_ROW_HEIGHT: f32 = 32.0;
/// Inset of the panel around its rows, and the room a group rule gets.
pub const MENU_PADDING: f32 = 4.0;
/// Space between the anchor's bottom edge and the panel.
pub const MENU_GAP: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Index into the configured profiles.
    Profile(usize),
    Settings,
    CommandPalette,
    About,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    /// Right-aligned key hint; empty when the row has no shortcut.
    pub shortcut: String,
    pub action: MenuAction,
}

impl MenuItem {
    pub fn profile(index: usize, label: &str, shortcut: &str) -> Self {
        Self::new(MenuAction::Profile(index), label, shortcut)
    }

    pub fn fixed(action: MenuAction, label: &str, shortcut: &str) -> Self {
        Self::new(action, label, shortcut)
    }

    fn new(action: MenuAction, label: &str, shortcut: &str) -> Self {
        Self {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
            action,
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
    /// Anchored under `anchor`, kept inside the window on both axes: the
    /// dropdown opens downward unless that would run past the bottom edge, in
    /// which case it flips above the anchor. The right edge is pulled in when
    /// the anchor sits near the window's edge, and the panel shrinks to the
    /// window when the window is narrower than it.
    ///
    /// When neither side can hold the whole menu, the panel is pinned directly
    /// under the strip — never over it, or a row would steal the chevron — and
    /// only the rows that fit entirely are laid out. Dropped rows are absent
    /// from `rows`, so `hit` cannot find them.
    ///
    /// `anchor` is the strip's chevron, so its height carries the DPI scale the
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
        let x = (anchor.x + anchor.w - width).clamp(0.0, (window_width - width).max(0.0));

        let below = anchor.y + anchor.h + gap;
        let above = anchor.y - gap - full_height;
        let y = if below + full_height <= window_height {
            below
        } else if above >= 0.0 {
            above
        } else {
            // Neither side holds the whole menu. Pin it under the strip and
            // keep only the rows that fit; the window never clips a row that
            // `hit` can still reach.
            (anchor.y + anchor.h).max(0.0)
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
            MenuItem::profile(0, "Windows PowerShell", "Ctrl+Shift+1"),
            MenuItem::profile(1, "Command Prompt", "Ctrl+Shift+2"),
            MenuItem::fixed(MenuAction::Settings, "Settings", "Ctrl+,"),
            MenuItem::fixed(MenuAction::About, "About", ""),
        ]
    }

    fn nine_items() -> Vec<MenuItem> {
        let mut items: Vec<MenuItem> = (0..6)
            .map(|index| MenuItem::profile(index, &format!("Profile {index}"), ""))
            .collect();
        items.push(MenuItem::fixed(MenuAction::Settings, "Settings", "Ctrl+,"));
        items.push(MenuItem::fixed(
            MenuAction::CommandPalette,
            "Command palette",
            "Ctrl+Shift+P",
        ));
        items.push(MenuItem::fixed(MenuAction::About, "About", ""));
        items
    }

    #[test]
    fn the_menu_hangs_below_its_anchor_and_keeps_its_top_edge() {
        let anchor = Rect {
            x: 700.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
        };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.rect.y, 32.0 + MENU_GAP);
        assert!(menu.rect.x + menu.rect.w <= 700.0 + 32.0 + 0.5);
        assert_eq!(menu.rows.len(), 4);
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
            h: 32.0,
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
            h: 32.0,
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
            h: 32.0,
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
            h: 32.0,
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
            h: 32.0,
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
        assert_eq!(menu.rect.y, 48.0 + MENU_GAP * k);
        assert_eq!(menu.rows[0].h, MENU_ROW_HEIGHT * k);
        assert_eq!(
            menu.hit(menu.rows[3].x + 4.0, menu.rows[3].y + 4.0),
            Some(3)
        );
    }

    #[test]
    fn a_short_window_keeps_the_panel_and_its_rows_inside() {
        // 200 px cannot hold a 9-row panel (300 px) below or above the strip,
        // so the panel is pinned under the strip and only whole rows are kept.
        let anchor = Rect {
            x: 0.0,
            y: 0.0,
            w: 32.0,
            h: 32.0,
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
            h: 32.0,
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
}
