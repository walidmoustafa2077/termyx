//! The spellcheck suggestion menu: where its rows sit and what a click means.
//!
//! The menu hangs off the misspelled token: one row per suggestion, then the
//! two actions that are always offered. Like the rest of this directory it is
//! pure coordinates and state, so the layout and the hit test are exercised
//! without a window and the painter reads the same numbers.

use crate::ui::tab_bar::Rect;

/// Width of the panel in logical pixels, at the reference scale.
///
/// The real width would be the widest label, which only the renderer can
/// measure. A fixed width keeps the geometry testable and stops the panel
/// jumping about as the suggestions change; it is clamped to the viewport.
pub const SPELLCHECK_MENU_WIDTH: f32 = 220.0;
/// Height of one row in logical pixels.
pub const SPELLCHECK_ROW_HEIGHT: f32 = 28.0;
/// Inset of the panel around its rows.
pub const SPELLCHECK_PADDING: f32 = 2.0;
/// Space between the anchor's edge and the panel.
pub const SPELLCHECK_GAP: f32 = 2.0;
/// Corner radius of the panel in logical pixels.
pub const SPELLCHECK_RADIUS: f32 = 8.0;
/// Where a row's label starts, from the row's left edge.
pub const SPELLCHECK_LABEL_INSET: f32 = 12.0;
/// Inset of a label's right clip from the row's right edge.
pub const SPELLCHECK_RIGHT_INSET: f32 = 10.0;
/// Horizontal inset of the hover pill inside its row.
///
/// The rows are already inset by this much, so the pill is painted at the row
/// rect itself and the visible padding around it comes from the layout.
pub const SPELLCHECK_HOVER_INSET: f32 = 2.0;
/// Corner radius of the hover pill in logical pixels.
pub const SPELLCHECK_HOVER_RADIUS: f32 = 4.0;
/// Em size of a row's label in logical pixels.
pub const SPELLCHECK_TEXT_PX: f32 = 12.0;

/// Default excuse kept when a caller disables replacement without saying why.
pub const SPELLCHECK_NOT_LAST_WORD: &str = "the cursor must be at the end of the last word";

/// One row of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuItem {
    /// Replace the misspelled token with this word.
    Suggest(String),
    /// Add the token to the user's dictionary, so it stops being flagged.
    AddToDictionary,
    /// Turn the underlines off for this session.
    HideUnderlines,
}

impl MenuItem {
    /// The row's visible text.
    ///
    /// One source for the painter and any future keyboard menu, so the two
    /// cannot disagree about what a row says.
    pub fn label(&self) -> &str {
        match self {
            MenuItem::Suggest(word) => word,
            MenuItem::AddToDictionary => "Add to dictionary",
            MenuItem::HideUnderlines => "Hide underlines",
        }
    }
}

/// The menu's state: which token it belongs to, its rows, and what is hovered.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellcheckMenu {
    /// Grid row of the misspelled token.
    pub row: usize,
    /// Grid column of the misspelled token's first cell.
    pub col: usize,
    /// The token's cell count: how many DELs a replacement sends.
    ///
    /// This is not `token.len()`: [`crate::term::Cell::text`] may carry
    /// combining marks while the scanner reports one cell per grapheme.
    pub len: usize,
    /// The token itself, the text a suggestion would replace.
    pub token: String,
    /// Suggestions first, then the two actions; never empty.
    pub items: Vec<MenuItem>,
    /// Whether replacement is possible. Gates [`MenuItem::Suggest`] only: the
    /// two actions stay live even when the cursor has moved off the token.
    pub enabled: bool,
    /// Why replacement is unavailable.
    ///
    /// Informational only: the painter does not display it, and the window
    /// re-checks the grid before sending any replacement bytes.
    pub disabled_reason: Option<&'static str>,
    /// Row under the pointer, filled by the window and read by the painter.
    pub hovered: Option<usize>,
}

impl SpellcheckMenu {
    /// Builds the menu for `token`, whose suggestions are given in the order
    /// the speller ranked them. `len` is the token's cell count.
    ///
    /// The actions are appended unconditionally, so a token with no
    /// suggestions still opens a menu with two usable rows.
    pub fn new(
        row: usize,
        col: usize,
        len: usize,
        token: String,
        suggestions: Vec<String>,
        can_replace: bool,
        disabled_reason: Option<&'static str>,
    ) -> Self {
        let mut items: Vec<MenuItem> = suggestions.into_iter().map(MenuItem::Suggest).collect();
        items.push(MenuItem::AddToDictionary);
        items.push(MenuItem::HideUnderlines);
        // A reason is only meaningful while replacement is off, and a caller
        // that forgets one still leaves the window something to say.
        let disabled_reason = if can_replace {
            disabled_reason
        } else {
            Some(disabled_reason.unwrap_or(SPELLCHECK_NOT_LAST_WORD))
        };
        Self {
            row,
            col,
            len,
            token,
            items,
            enabled: can_replace,
            disabled_reason,
            hovered: None,
        }
    }

    /// Places the panel against `anchor` inside `viewport`, at scale `k`.
    ///
    /// The panel hangs below the anchor unless it would cross the viewport's
    /// bottom edge, in which case it flips above. Either way it is pulled back
    /// inside the viewport, so a menu opened at the window's edge still shows
    /// every row.
    pub fn layout(&self, anchor: Rect, viewport: Rect, k: f32) -> SpellcheckMenuLayout {
        let width = (SPELLCHECK_MENU_WIDTH * k).min(viewport.w.max(0.0));
        let row_height = SPELLCHECK_ROW_HEIGHT * k;
        let padding = SPELLCHECK_PADDING * k;
        let gap = SPELLCHECK_GAP * k;
        let height = self.items.len() as f32 * row_height + 2.0 * padding;

        let below = anchor.y + anchor.h + gap;
        let y = if below + height <= viewport.y + viewport.h {
            below
        } else {
            anchor.y - height - gap
        };

        let panel = Rect {
            x: clamp_origin(anchor.x, viewport.x, viewport.w, width),
            y: clamp_origin(y, viewport.y, viewport.h, height),
            w: width,
            h: height,
        };

        let inset = SPELLCHECK_HOVER_INSET * k;
        let rows = (0..self.items.len())
            .map(|index| Rect {
                x: panel.x + inset,
                y: panel.y + padding + index as f32 * row_height,
                w: (panel.w - 2.0 * inset).max(0.0),
                h: row_height,
            })
            .collect();

        SpellcheckMenuLayout { panel, rows }
    }

    /// Index of the row at `(x, y)`, or `None` outside the rows.
    ///
    /// The rows are inset inside the panel, so a point on the panel's padding
    /// is no row at all, exactly as it looks.
    pub fn hit(&self, layout: &SpellcheckMenuLayout, x: f32, y: f32) -> Option<usize> {
        layout.rows.iter().position(|row| row.contains(x, y))
    }

    /// Points the hover at whatever row contains `(x, y)`.
    pub fn set_hover(&mut self, layout: &SpellcheckMenuLayout, x: f32, y: f32) {
        self.hovered = self.hit(layout, x, y);
    }

    /// The item a click on `index` runs, or `None` for a suggestion while
    /// replacement is disabled.
    ///
    /// The actions are never gated: adding the word or hiding the underlines
    /// does not depend on the cursor still sitting on the token.
    pub fn select(&self, index: usize) -> Option<MenuItem> {
        let item = self.items.get(index)?;
        match item {
            MenuItem::Suggest(_) if !self.enabled => None,
            _ => Some(item.clone()),
        }
    }
}

/// The rectangles the painter and the hit test share.
#[derive(Debug, Clone, PartialEq)]
pub struct SpellcheckMenuLayout {
    /// The panel's outline.
    pub panel: Rect,
    /// One row per item, top to bottom, inset inside the panel.
    pub rows: Vec<Rect>,
}

/// Pulls `value` into `[origin, origin + span - size]`.
///
/// A panel wider than the viewport would invert that range, so it is pinned to
/// the origin instead of being clamped into a negative interval.
fn clamp_origin(value: f32, origin: f32, span: f32, size: f32) -> f32 {
    let max = origin + span - size;
    if max <= origin {
        origin
    } else {
        value.clamp(origin, max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        }
    }

    fn anchor(x: f32, y: f32) -> Rect {
        Rect {
            x,
            y,
            w: 40.0,
            h: 18.0,
        }
    }

    fn menu() -> SpellcheckMenu {
        SpellcheckMenu::new(
            3,
            4,
            3,
            "teh".to_string(),
            vec!["the".to_string(), "tea".to_string()],
            true,
            None,
        )
    }

    #[test]
    fn lists_suggestions_then_actions() {
        let menu = SpellcheckMenu::new(
            3,
            4,
            3,
            "teh".to_string(),
            vec!["the".to_string()],
            true,
            None,
        );
        assert_eq!(menu.items.len(), 3);
        assert!(matches!(menu.items[0], MenuItem::Suggest(ref s) if s == "the"));
        assert_eq!(menu.items[1], MenuItem::AddToDictionary);
        assert_eq!(menu.items[2], MenuItem::HideUnderlines);
        assert!(menu.enabled, "replacement is possible at the last token");
        assert_eq!(menu.token, "teh");
        assert_eq!(menu.len, 3);
        assert_eq!((menu.row, menu.col), (3, 4));
    }

    #[test]
    fn labels_every_row() {
        assert_eq!(MenuItem::Suggest("the".to_string()).label(), "the");
        assert_eq!(MenuItem::AddToDictionary.label(), "Add to dictionary");
        assert_eq!(MenuItem::HideUnderlines.label(), "Hide underlines");
    }

    #[test]
    fn disables_replacement_when_not_last_token() {
        let menu = SpellcheckMenu::new(
            0,
            0,
            3,
            "teh".to_string(),
            vec!["the".to_string()],
            false,
            Some("not the last word"),
        );
        assert!(!menu.enabled);
        assert_eq!(menu.disabled_reason, Some("not the last word"));
        assert_eq!(menu.select(0), None, "a suggestion is not clickable");
        assert_eq!(menu.select(1), Some(MenuItem::AddToDictionary));
        assert_eq!(menu.select(2), Some(MenuItem::HideUnderlines));
    }

    #[test]
    fn a_missing_reason_falls_back_to_the_cursor_hint() {
        let menu = SpellcheckMenu::new(0, 0, 3, "teh".to_string(), vec![], false, None);
        assert_eq!(menu.disabled_reason, Some(SPELLCHECK_NOT_LAST_WORD));
    }

    #[test]
    fn keeps_actions_when_there_are_no_suggestions() {
        let menu = SpellcheckMenu::new(0, 0, 3, "teh".to_string(), vec![], true, None);
        assert_eq!(menu.items.len(), 2);
        assert_eq!(menu.items[0], MenuItem::AddToDictionary);
        assert_eq!(menu.items[1], MenuItem::HideUnderlines);
    }

    #[test]
    fn places_the_panel_below_the_anchor() {
        let anchor = anchor(10.0, 30.0);
        let layout = menu().layout(anchor, viewport(), 1.0);
        assert_eq!(
            layout.panel.x, 10.0,
            "the panel is left-aligned to the anchor"
        );
        assert_eq!(
            layout.panel.y,
            anchor.y + anchor.h + SPELLCHECK_GAP,
            "the panel hangs below the anchor"
        );
        assert!(layout.panel.y > anchor.y + anchor.h);
    }

    #[test]
    fn flips_above_near_the_bottom_edge() {
        let anchor = anchor(10.0, 560.0);
        let layout = menu().layout(anchor, viewport(), 1.0);
        assert!(
            layout.panel.y < anchor.y,
            "a panel that cannot open below flips above"
        );
        assert_eq!(
            layout.panel.y,
            anchor.y - layout.panel.h - SPELLCHECK_GAP,
            "the flipped panel sits one gap above the anchor's top edge"
        );
        assert!(
            layout.panel.y + layout.panel.h <= viewport().y + viewport().h,
            "the flipped panel still fits the viewport"
        );
    }

    #[test]
    fn clamps_the_panel_inside_the_viewport() {
        let anchor = anchor(780.0, 100.0);
        let layout = menu().layout(anchor, viewport(), 1.0);
        assert!(
            layout.panel.x + layout.panel.w <= viewport().x + viewport().w,
            "the panel must not leave the right edge"
        );
        assert_eq!(layout.panel.x, 800.0 - SPELLCHECK_MENU_WIDTH);
    }

    #[test]
    fn shrinks_to_a_viewport_narrower_than_the_panel() {
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            w: 120.0,
            h: 600.0,
        };
        let layout = menu().layout(anchor(100.0, 10.0), viewport, 1.0);
        assert_eq!(layout.panel.w, 120.0, "the panel shrinks to the viewport");
        assert_eq!(
            layout.panel.x, 0.0,
            "a negative range is pinned to the origin"
        );
    }

    #[test]
    fn pins_an_oversized_panel_to_the_viewport_top() {
        // 50 px cannot hold a three-row panel (88 px), so the only safe place
        // is the viewport's origin: the clamp range would otherwise invert and
        // a bare `f32::clamp` would panic.
        let viewport = Rect {
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 50.0,
        };
        let menu = SpellcheckMenu::new(
            0,
            0,
            3,
            "teh".to_string(),
            vec!["the".to_string()],
            true,
            None,
        );
        let layout = menu.layout(anchor(10.0, 40.0), viewport, 1.0);
        assert_eq!(
            layout.panel.y, viewport.y,
            "an oversized panel pins to the top"
        );
        assert_eq!(
            layout.panel.h,
            menu.items.len() as f32 * SPELLCHECK_ROW_HEIGHT + 2.0 * SPELLCHECK_PADDING,
            "the panel keeps its full computed height"
        );
    }

    #[test]
    fn hit_tests_rows() {
        let layout = menu().layout(anchor(10.0, 30.0), viewport(), 1.0);
        let row = layout.rows[1];
        assert_eq!(
            menu().hit(&layout, row.x + row.w / 2.0, row.y + row.h / 2.0),
            Some(1),
            "a point inside the second row is row 1"
        );
        assert_eq!(
            menu().hit(&layout, layout.panel.x - 5.0, layout.panel.y - 5.0),
            None,
            "a point outside the panel is no row"
        );
    }

    #[test]
    fn set_hover_points_at_the_row_under_the_pointer() {
        let layout = menu().layout(anchor(10.0, 30.0), viewport(), 1.0);
        let mut menu = menu();
        let row = layout.rows[2];
        menu.set_hover(&layout, row.x + 4.0, row.y + 4.0);
        assert_eq!(menu.hovered, Some(2));
        menu.set_hover(&layout, layout.panel.x - 5.0, layout.panel.y - 5.0);
        assert_eq!(menu.hovered, None);
    }

    #[test]
    fn rows_tile_without_gaps() {
        let layout = menu().layout(anchor(10.0, 30.0), viewport(), 1.0);
        assert_eq!(layout.rows.len(), menu().items.len());
        assert_eq!(layout.rows[1].y, layout.rows[0].y + layout.rows[0].h);
        assert_eq!(layout.rows[2].y, layout.rows[1].y + layout.rows[1].h);
    }

    #[test]
    fn the_layout_scales_with_the_window() {
        let layout = menu().layout(anchor(10.0, 30.0), viewport(), 2.0);
        assert_eq!(layout.panel.w, SPELLCHECK_MENU_WIDTH * 2.0);
        assert_eq!(layout.rows[0].h, SPELLCHECK_ROW_HEIGHT * 2.0);
        assert_eq!(
            layout.panel.h,
            menu().items.len() as f32 * SPELLCHECK_ROW_HEIGHT * 2.0
                + 2.0 * SPELLCHECK_PADDING * 2.0
        );
    }
}
