//! The tab's right-click menu: panel geometry, submenu chains, and the
//! commands its rows run.
//!
//! Like the `⌄` dropdown, the model owns coordinates only. [`ContextMenu::new`]
//! hangs the root panel under the tab card, [`ContextMenu::open_submenu`]
//! chains child panels to the right, and [`ContextMenu::hit`] and
//! [`ContextMenu::press`] answer pointer questions with [`ContextHit`] and
//! [`ContextPress`]. The painter draws what the panels carry; the app
//! performs the [`TabCommand`] a press's [`ContextPress::Run`] returns.

use crate::theme::Rgb;
use crate::ui::menu::{MENU_GAP, MENU_PADDING, MENU_ROW_HEIGHT};
use crate::ui::tab_bar::Rect;

/// Width of the context menu in logical pixels; submenus reuse it so a chain
/// of panels steps right at a steady rhythm.
pub const CONTEXT_MENU_WIDTH: f32 = 320.0;

/// What performing one of the context menu's rows asks the app to do.
// The right-click handler consumes this in a later task; the tests pin the
// set now.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCommand {
    /// Colors the tab; the inner `None` is the Default swatch.
    ChangeColor(Option<Rgb>),
    Rename,
    Duplicate,
    Split,
    MoveToNewWindow,
    MoveLeft,
    MoveRight,
    ExportText,
    Find,
    CloseOtherTabs,
    CloseTabsToRight,
    CloseTab,
}

/// One row of a panel: what it shows, the command it runs, and the submenu
/// it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    pub label: String,
    /// Right-aligned key hint; empty when the row has no shortcut.
    pub shortcut: String,
    /// Icon-face codepoint, or empty when the row draws no glyph: a swatch
    /// row shows its swatch in the icon column instead.
    pub glyph: &'static str,
    /// The command the row runs; `None` for a submenu parent.
    pub command: Option<TabCommand>,
    pub enabled: bool,
    /// Outer `Some` means draw an icon swatch; inner `None` is the empty
    /// "Default" swatch. `None` means the row has no swatch.
    pub swatch: Option<Option<Rgb>>,
    pub separator_before: bool,
    /// The rows the submenu shows, empty for a plain row.
    pub children: Vec<ContextItem>,
}

impl ContextItem {
    /// A plain row that runs `command`.
    pub fn action(label: &str, glyph: &'static str, command: TabCommand) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            glyph,
            command: Some(command),
            enabled: true,
            swatch: None,
            separator_before: false,
            children: Vec::new(),
        }
    }

    /// A submenu parent: opening it shows `children`; it runs no command.
    pub fn submenu(label: &str, glyph: &'static str, children: Vec<ContextItem>) -> Self {
        let mut item = Self::action(label, glyph, TabCommand::Rename);
        item.command = None;
        item.children = children;
        item
    }

    /// A swatch row: the icon column shows `color`, or the bordered empty
    /// square when it is `None`, and the row applies that color to the tab.
    pub fn swatch(label: &str, color: Option<Rgb>) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            glyph: "",
            command: Some(TabCommand::ChangeColor(color)),
            enabled: true,
            swatch: Some(color),
            separator_before: false,
            children: Vec::new(),
        }
    }

    /// The row shows but does nothing: the placeholder the reference keeps
    /// visible until its feature exists.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Gives the row a group rule in front of it.
    pub fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
}

/// One panel of the chain: its own inset rect, one row rect per laid-out
/// item, and the y of every group rule the panel owns.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextPanel {
    pub rect: Rect,
    /// One rect per item, top to bottom. A short window keeps the rows that
    /// fit entirely, so `rows` and `items` stay in step and `hit` can never
    /// find a dropped row.
    pub rows: Vec<Rect>,
    pub items: Vec<ContextItem>,
    /// y of each group rule, recorded at the top of the padding gap the row
    /// that asked for it reserved, where the dropdown records its own.
    pub separators: Vec<f32>,
    /// Row under the pointer, filled by the window and read by the painter.
    pub hovered: Option<usize>,
}

/// The chain of panels the menu shows: the root under the tab card, plus one
/// panel per open submenu.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    pub panels: Vec<ContextPanel>,
    /// The strip's scale, clamped into `0.5..=4.0`; the painter sizes its
    /// ink from it without being told a second time.
    pub scale: f32,
    /// The window the chain was clamped into, carried so submenu placement
    /// uses the same bounds the root did.
    pub window_width: f32,
    pub window_height: f32,
}

/// Where a point lands in the chain: on a row, on a panel's padding, or
/// nowhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextHit {
    Row { panel: usize, row: usize },
    Panel(usize),
    Outside,
}

/// What a press on the chain asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPress {
    Run(TabCommand),
    OpenSubmenu(usize),
    KeepOpen,
    Dismiss,
}

/// Lays one panel out at `x` from `items`, starting at `desired_y` and
/// clamping into the window; rows that would hang past its bottom edge are
/// dropped from the end.
fn carve(
    x: f32,
    desired_y: f32,
    items: Vec<ContextItem>,
    window_width: f32,
    window_height: f32,
    k: f32,
) -> ContextPanel {
    let padding = MENU_PADDING * k;
    let row_height = MENU_ROW_HEIGHT * k;
    let width = (CONTEXT_MENU_WIDTH * k).min(window_width.max(0.0));

    // Each row's offset below the panel's top padding, with a rule's room
    // reserved in front of the rows that ask for one. The first row gets no
    // rule: the panel's own padding is its group start.
    let mut offsets = Vec::with_capacity(items.len());
    let mut cursor = 0.0_f32;
    for (index, item) in items.iter().enumerate() {
        if item.separator_before && index != 0 {
            cursor += padding;
        }
        offsets.push(cursor);
        cursor += row_height;
    }
    let height_with = |count: usize| {
        if count == 0 {
            0.0
        } else {
            offsets[count - 1] + row_height + 2.0 * padding
        }
    };

    // Whole rows only: a row that would hang over the edge is dropped, along
    // with every row after it.
    let mut count = items.len();
    while count > 0 && height_with(count) > window_height {
        count -= 1;
    }
    let height = height_with(count);

    // The panel stays under the tab card whenever the window can hold it; a
    // clamped panel pins to the window's bottom edge, never above the strip.
    let y = if desired_y + height <= window_height {
        desired_y
    } else {
        (window_height - height).max(0.0)
    };

    let mut items = items;
    items.truncate(count);
    let mut rows = Vec::with_capacity(items.len());
    let mut separators = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if item.separator_before && index != 0 {
            separators.push(y + offsets[index]);
        }
        rows.push(Rect {
            x: x + padding,
            y: y + padding + offsets[index],
            w: width - 2.0 * padding,
            h: row_height,
        });
    }

    ContextPanel {
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

// The whole chain is wired to the right-click handler in a later task; the
// tests pin the geometry now.
#[allow(dead_code)]
impl ContextMenu {
    /// Opens the menu for the tab card `anchor`: the root panel hangs from
    /// its bottom edge by [`MENU_GAP`], clamped inside the window on both
    /// axes.
    ///
    /// `scale` is the strip's factor; it clamps to `0.5..=4.0` so a tiny
    /// strip cannot shrink the rows out of reach and a huge one cannot blow
    /// the panel past the window. A short window keeps only the rows that
    /// fit entirely; the menu never flips above the strip, because it is
    /// anchored to the tab card.
    pub fn new(
        anchor: Rect,
        items: Vec<ContextItem>,
        window_width: f32,
        window_height: f32,
        scale: f32,
    ) -> Self {
        let k = scale.clamp(0.5, 4.0);
        let width = (CONTEXT_MENU_WIDTH * k).min(window_width.max(0.0));
        let x = anchor.x.clamp(0.0, (window_width - width).max(0.0));
        let below = anchor.y + anchor.h + MENU_GAP * k;
        Self {
            panels: vec![carve(x, below, items, window_width, window_height, k)],
            scale: k,
            window_width,
            window_height,
        }
    }

    /// Replaces every panel after `panel` with the child chain the row
    /// `row` opens, placed to the parent panel's right by [`MENU_GAP`].
    ///
    /// A child that would overflow the window's right edge opens to the
    /// parent's left instead, and one pinned at the left edge may still hang
    /// past the right if it is wider than the window; the painter truncates
    /// such text. The child's first row lines up with the parent row, and
    /// the panel never leaves the window's bottom edge.
    pub fn open_submenu(&mut self, panel: usize, row: usize) {
        let (parent, children) = {
            let host = &self.panels[panel];
            (host.rect, host.items[row].children.clone())
        };
        let k = self.scale;
        let gap = MENU_GAP * k;
        let width = (CONTEXT_MENU_WIDTH * k).min(self.window_width.max(0.0));
        let x = if parent.x + parent.w + gap + width <= self.window_width {
            parent.x + parent.w + gap
        } else {
            (parent.x - width - gap).max(0.0)
        };
        let desired_y = self.panels[panel].rows[row].y - MENU_PADDING * k;
        let child = carve(
            x,
            desired_y,
            children,
            self.window_width,
            self.window_height,
            k,
        );
        self.panels.truncate(panel + 1);
        self.panels.push(child);
    }

    /// Which panel of the chain the point lands on, checking child panels
    /// first: they sit closer to the pointer than the parent's rows do.
    pub fn hit(&self, x: f32, y: f32) -> ContextHit {
        for (index, panel) in self.panels.iter().enumerate().rev() {
            if !panel.rect.contains(x, y) {
                continue;
            }
            if let Some(row) = panel.rows.iter().position(|row| row.contains(x, y)) {
                return ContextHit::Row { panel: index, row };
            }
            return ContextHit::Panel(index);
        }
        ContextHit::Outside
    }

    /// Keeps the open chain of panels in step with the pointer at `(x, y)`.
    ///
    /// Hovering a submenu row opens its child panel, and hovering a plain
    /// row closes the chain that row's parent no longer needs. A disabled
    /// submenu parent leaves the chain as it is: it neither opens nor
    /// closes. Hovering a panel's padding or leaving the chain entirely only
    /// clears the highlight, never the panels — the pointer may just be
    /// crossing the gap into a child.
    pub fn hover(&mut self, x: f32, y: f32) {
        match self.hit(x, y) {
            ContextHit::Row { panel, row } => {
                let (opens, plain) = {
                    let item = &self.panels[panel].items[row];
                    (
                        item.enabled && !item.children.is_empty(),
                        item.children.is_empty(),
                    )
                };
                if opens {
                    self.open_submenu(panel, row);
                } else if plain {
                    self.panels.truncate(panel + 1);
                }
                let depth = panel;
                self.panels[depth].hovered = Some(row);
                for (index, panel) in self.panels.iter_mut().enumerate() {
                    if index != depth {
                        panel.hovered = None;
                    }
                }
            }
            ContextHit::Panel(_) | ContextHit::Outside => self.clear_hover(),
        }
    }

    /// Answers a press at `(x, y)` without mutating the menu: the window
    /// closes the chain on [`ContextPress::Dismiss`], chains the submenu on
    /// [`ContextPress::OpenSubmenu`], and performs
    /// [`ContextPress::Run`]'s command.
    pub fn press(&self, x: f32, y: f32) -> ContextPress {
        match self.hit(x, y) {
            ContextHit::Outside => ContextPress::Dismiss,
            ContextHit::Panel(_) => ContextPress::KeepOpen,
            ContextHit::Row { panel, row } => {
                let item = &self.panels[panel].items[row];
                if !item.enabled {
                    ContextPress::KeepOpen
                } else if !item.children.is_empty() {
                    ContextPress::OpenSubmenu(row)
                } else if let Some(command) = item.command {
                    ContextPress::Run(command)
                } else {
                    ContextPress::KeepOpen
                }
            }
        }
    }

    /// Drops every row highlight, leaving the panels open: the pointer left
    /// all of them, the way `CursorLeft` ends a pass over the chain.
    pub fn clear_hover(&mut self) {
        for panel in &mut self.panels {
            panel.hovered = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tab_bar::Rect;

    fn tab() -> Rect {
        Rect {
            x: 100.0,
            y: 8.0,
            w: 180.0,
            h: 32.0,
        }
    }

    fn items() -> Vec<ContextItem> {
        vec![
            ContextItem::action("Rename tab", "R", TabCommand::Rename),
            ContextItem::submenu(
                "Move tab",
                "M",
                vec![
                    ContextItem::action("Move right", "R", TabCommand::MoveRight),
                    ContextItem::action("Move left", "L", TabCommand::MoveLeft),
                ],
            ),
            ContextItem::action("Close tab", "X", TabCommand::CloseTab).separator_before(),
        ]
    }

    #[test]
    fn opens_below_the_tab_and_clamps_inside_the_window() {
        let menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        assert!(menu.panels[0].rect.y >= tab().y + tab().h);
        assert!(menu.panels[0].rect.x >= 0.0);
        assert!(menu.panels[0].rect.x + menu.panels[0].rect.w <= 800.0);
    }

    #[test]
    fn a_narrow_tab_near_the_right_pulls_the_panel_left() {
        let mut tab = tab();
        tab.x = 790.0;
        let menu = ContextMenu::new(tab, items(), 800.0, 600.0, 1.0);
        assert_eq!(menu.panels[0].rect.x + menu.panels[0].rect.w, 800.0);
    }

    #[test]
    fn clicking_a_submenu_parent_opens_it_to_the_right() {
        let mut menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        let row = menu.panels[0].rows[1];
        assert_eq!(
            menu.press(row.x + 5.0, row.y + 5.0),
            ContextPress::OpenSubmenu(1)
        );
        menu.open_submenu(0, 1);
        assert_eq!(menu.panels.len(), 2);
        assert_eq!(
            menu.panels[1].rect.x,
            menu.panels[0].rect.x + menu.panels[0].rect.w + 2.0
        );
    }

    #[test]
    fn a_disabled_row_never_runs() {
        let mut items = items();
        items[0].enabled = false;
        let menu = ContextMenu::new(tab(), items, 800.0, 600.0, 1.0);
        let row = menu.panels[0].rows[0];
        assert_eq!(menu.press(row.x + 5.0, row.y + 5.0), ContextPress::KeepOpen);
    }

    #[test]
    fn clicks_outside_dismiss() {
        let menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        assert_eq!(menu.press(0.0, 0.0), ContextPress::Dismiss);
    }

    #[test]
    fn separator_space_is_not_a_row() {
        let menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        let separator = menu.panels[0].separators[0];
        assert!(matches!(
            menu.hit(menu.panels[0].rect.x + 5.0, separator),
            ContextHit::Panel(0)
        ));
    }

    #[test]
    fn a_short_window_keeps_only_the_rows_that_fit() {
        // 90 px cannot hold three rows and their rule, so the trailing rows
        // and their rule vanish and the panel still ends at the bottom edge.
        let menu = ContextMenu::new(tab(), items(), 800.0, 90.0, 1.0);
        assert_eq!(menu.panels[0].rows.len(), 2);
        assert_eq!(menu.panels[0].items.len(), 2);
        assert!(
            menu.panels[0].separators.is_empty(),
            "a rule in front of a dropped row is dropped with it"
        );
        assert!(menu.panels[0].rect.y + menu.panels[0].rect.h <= 90.0);
        for row in &menu.panels[0].rows {
            assert!(row.y + row.h <= 90.0);
        }
    }

    #[test]
    fn hovering_steps_the_row_highlight_and_the_chain() {
        let mut menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        let parent = menu.panels[0].rows[1];
        menu.hover(parent.x + 5.0, parent.y + 5.0);
        assert_eq!(
            menu.panels.len(),
            2,
            "hovering a submenu row opens its child"
        );
        assert_eq!(menu.panels[0].hovered, Some(1));
        let child = menu.panels[1].rows[0];
        menu.hover(child.x + 5.0, child.y + 5.0);
        assert_eq!(menu.panels[1].hovered, Some(0));
        assert_eq!(
            menu.panels[0].hovered, None,
            "the child's row takes the highlight"
        );
        let plain = menu.panels[0].rows[0];
        menu.hover(plain.x + 5.0, plain.y + 5.0);
        assert_eq!(
            menu.panels.len(),
            1,
            "a row without a submenu closes the child chain"
        );
        assert_eq!(menu.panels[0].hovered, Some(0));
        menu.clear_hover();
        assert_eq!(menu.panels[0].hovered, None);
    }
}
