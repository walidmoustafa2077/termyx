//! The tab context menu's rows: what the model shows for one tab.
//!
//! The app builds these rows once per right click from what it knows about
//! the target tab; `ui::context_menu` owns their geometry and the painter
//! their ink. The builder is pure, so the strip's rules live in tests instead
//! of in the window's event code.

use crate::fonts::{
    GLYPH_CLOSE, GLYPH_DUPLICATE, GLYPH_EXPORT, GLYPH_FIND, GLYPH_MOVE_TAB, GLYPH_RENAME,
    GLYPH_SPLIT, GLYPH_TAB_COLOR,
};
use crate::theme::Rgb;
use crate::ui::context_menu::{ContextItem, TabCommand};

/// Builds every row the right-clicked tab shows, in reference order: color,
/// rename, duplicate, split, move, export, find, and the close group.
///
/// `index` is the tab's position in the strip and `total` its size, so edge
/// tabs lose the rows that could not move or close anything they have. A
/// page tab sets `can_duplicate` to `false`, having no shell to spawn again.
/// `colors` are the swatch choices, one row each under the Default swatch.
// Built by the right-click handler; the tests pin the rows here.
pub fn rows(index: usize, total: usize, can_duplicate: bool, colors: &[Rgb]) -> Vec<ContextItem> {
    let mut duplicate =
        ContextItem::action("Duplicate tab", GLYPH_DUPLICATE, TabCommand::Duplicate);
    duplicate.enabled = can_duplicate;

    let mut move_right = ContextItem::action("Move right", "", TabCommand::MoveRight);
    move_right.enabled = index + 1 < total;
    let mut move_left = ContextItem::action("Move left", "", TabCommand::MoveLeft);
    move_left.enabled = index > 0;

    let mut close_others = ContextItem::action("Close other tabs", "", TabCommand::CloseOtherTabs);
    close_others.enabled = total > 1;
    let mut close_right =
        ContextItem::action("Close tabs to right", "", TabCommand::CloseTabsToRight);
    close_right.enabled = index + 1 < total;

    vec![
        color_submenu(colors),
        ContextItem::action("Rename tab", GLYPH_RENAME, TabCommand::Rename),
        duplicate,
        ContextItem::action("Split tab", GLYPH_SPLIT, TabCommand::Split).disabled(),
        ContextItem::submenu(
            "Move tab",
            GLYPH_MOVE_TAB,
            vec![
                // The reference keeps splitting and new-window moves visible
                // but disabled until their own plans exist.
                ContextItem::action("Move tab to new window", "", TabCommand::MoveToNewWindow)
                    .disabled(),
                move_right,
                move_left,
            ],
        ),
        ContextItem::action("Export text", GLYPH_EXPORT, TabCommand::ExportText).separator_before(),
        ContextItem::action("Find", GLYPH_FIND, TabCommand::Find),
        ContextItem::submenu("Close", GLYPH_CLOSE, vec![close_others, close_right])
            .separator_before(),
        ContextItem::action("Close tab", GLYPH_CLOSE, TabCommand::CloseTab).separator_before(),
    ]
}

/// The `Change tab color` submenu: the Default swatch first, then one swatch
/// row per `colors` entry, each carrying the [`TabCommand::ChangeColor`] the
/// app applies to the target tab.
fn color_submenu(colors: &[Rgb]) -> ContextItem {
    let mut children = Vec::with_capacity(colors.len() + 1);
    children.push(ContextItem::swatch("Default", None));
    children.extend(
        colors
            .iter()
            .map(|color| ContextItem::swatch(&color_label(*color), Some(*color))),
    );
    ContextItem::submenu("Change tab color", GLYPH_TAB_COLOR, children)
}

/// Names a swatch row after the color it fills, since the builder is handed
/// the `Rgb`s themselves, not any palette the theme can name for them.
fn color_label(color: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tab_menu;

    /// The row with `label`, searching submenus depth-first.
    fn find<'a>(rows: &'a [ContextItem], label: &str) -> &'a ContextItem {
        find_some(rows, label)
            .unwrap_or_else(|| panic!("no row labelled `{label}` in the tab menu"))
    }

    fn find_some<'a>(rows: &'a [ContextItem], label: &str) -> Option<&'a ContextItem> {
        for row in rows {
            if row.label == label {
                return Some(row);
            }
            if let Some(found) = find_some(&row.children, label) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn edge_tabs_disable_the_matching_move_rows() {
        let colors = [Rgb::new(1, 2, 3); 8];
        let left = tab_menu::rows(0, 3, true, &colors);
        let right = tab_menu::rows(2, 3, true, &colors);
        assert!(find(&left, "Move right").enabled);
        assert!(!find(&left, "Move left").enabled);
        assert!(!find(&right, "Move right").enabled);
        assert!(find(&right, "Move left").enabled);
    }

    #[test]
    fn page_tabs_cannot_duplicate_but_can_close() {
        let colors = [Rgb::new(1, 2, 3); 8];
        let rows = tab_menu::rows(0, 1, false, &colors);
        assert!(!find(&rows, "Duplicate tab").enabled);
        assert!(!find(&rows, "Close other tabs").enabled);
        assert!(find(&rows, "Close tab").enabled);
    }

    #[test]
    fn the_rows_follow_the_reference_order() {
        let colors = [Rgb::new(1, 2, 3); 8];
        let rows = tab_menu::rows(1, 3, true, &colors);
        let labels: Vec<&str> = rows.iter().map(|row| row.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "Change tab color",
                "Rename tab",
                "Duplicate tab",
                "Split tab",
                "Move tab",
                "Export text",
                "Find",
                "Close",
                "Close tab",
            ]
        );
    }

    #[test]
    fn the_placeholder_rows_ship_visible_but_disabled() {
        let colors = [Rgb::new(1, 2, 3); 8];
        let rows = tab_menu::rows(1, 3, true, &colors);
        assert!(!find(&rows, "Split tab").enabled);
        assert!(!find(&rows, "Move tab to new window").enabled);
    }

    #[test]
    fn the_color_submenu_lists_default_then_every_color() {
        let colors = [Rgb::new(16, 32, 64), Rgb::new(200, 100, 0)];
        let rows = tab_menu::rows(0, 2, true, &colors);
        let submenu = find(&rows, "Change tab color");
        assert_eq!(
            submenu.children.len(),
            3,
            "the Default swatch plus one swatch per color"
        );
        assert_eq!(
            submenu.children[0].swatch,
            Some(None),
            "Default is the bordered empty swatch"
        );
        assert_eq!(
            submenu.children[0].command,
            Some(TabCommand::ChangeColor(None))
        );
        assert_eq!(submenu.children[1].swatch, Some(Some(Rgb::new(16, 32, 64))));
        assert_eq!(submenu.children[2].label, "#C86400");
    }
}
