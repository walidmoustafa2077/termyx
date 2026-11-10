//! Tab strip geometry. Pure coordinates: the renderer and the hit test share
//! one description of where every control sits.

/// Height of the strip in logical pixels; the terminal grid starts below it.
pub const TAB_STRIP_HEIGHT: f32 = 32.0;
/// Widest a single tab grows before the title is clipped.
pub const MAX_TAB_WIDTH: f32 = 220.0;
/// Narrowest a tab shrinks to while the strip still has room.
pub const MIN_TAB_WIDTH: f32 = 80.0;
/// Width of the `+` and `⌄` buttons.
pub const BUTTON_WIDTH: f32 = 32.0;
/// Width of one of the three window controls at the strip's right end,
/// matching the Windows caption buttons.
pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;
/// Logical px a tab spends on chrome besides the title itself: the left
/// padding, the icon, the gap after the icon, the gap before the close box,
/// the close box, and the right padding — `12 + 16 + 6 + 6 + 16 + 12`.
pub const TITLE_SIZE: f32 = 68.0;
/// Left inset from the tab edge to the app icon.
pub(crate) const TAB_PADDING: f32 = 12.0;
/// Side of the square app icon every tab stamps at its left.
pub(crate) const TAB_ICON_SIZE: f32 = 16.0;
/// Gap between the icon, the title, and the close box.
pub(crate) const TAB_ICON_GAP: f32 = 6.0;
const CLOSE_SIZE: f32 = 16.0;
/// Em size of the `✕` ink, smaller than a title so the cross does not fill its
/// box edge to edge.
pub(crate) const CLOSE_GLYPH_SIZE: f32 = 10.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabRect {
    pub index: usize,
    pub rect: Rect,
    pub close: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    Plus,
    Chevron,
    /// The three window controls, left to right at the strip's right end.
    Minimize,
    Maximize,
    CloseWindow,
}

/// What the pointer is over in the strip band, for painting hover.
///
/// A tab body (or its close button) is one target; the two buttons and the
/// three window controls are their own, so a press on any can highlight
/// without claiming a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripHover {
    Tab(usize),
    Plus,
    Chevron,
    Minimize,
    Maximize,
    CloseWindow,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabBarLayout {
    pub width: f32,
    /// Physical height of the strip, the one `App` reserves above the grid.
    pub strip_h: f32,
    pub tabs: Vec<TabRect>,
    pub titles: Vec<String>,
    pub plus: Rect,
    pub chevron: Rect,
    /// Window controls pinned to the right edge, left to right.
    pub minimize: Rect,
    pub maximize: Rect,
    pub close_window: Rect,
}

impl TabBarLayout {
    /// `strip_h` is already physical: `App` passes the height the renderer
    /// reserved above the grid. Every constant below is logical
    /// ([`TAB_STRIP_HEIGHT`] is the reference), so one factor
    /// `k = strip_h / TAB_STRIP_HEIGHT` scales the whole layout and paint and
    /// hit-testing stay aligned at any DPI.
    ///
    /// `width_of` measures a title at the UI font size, so a long title both
    /// widens its tab (up to [`MAX_TAB_WIDTH`]) and gets clipped when the strip
    /// runs out of room.
    ///
    /// The two buttons and the three window controls are reserved before any
    /// tab is placed, and each tab may claim at most an equal share of what is
    /// left. A crowded strip therefore shrinks its tabs (below
    /// [`MIN_TAB_WIDTH`] if it must) instead of pushing a control past the
    /// window edge, so `+`, `⌄` and the caption buttons stay clickable.
    pub fn new(width: f32, strip_h: f32, titles: &[&str], width_of: impl Fn(&str) -> f32) -> Self {
        let strip_h = strip_h.max(0.0);
        let k = strip_h / TAB_STRIP_HEIGHT;
        let max_tab = MAX_TAB_WIDTH * k;
        let min_tab = MIN_TAB_WIDTH * k;
        let close_size = CLOSE_SIZE * k;
        let tab_padding = TAB_PADDING * k;
        let title_size = TITLE_SIZE * k;
        // A window narrower than the button pair shrinks both together, so the
        // strip always ends exactly at `width` and neither button is clipped.
        let buttons = (2.0 * BUTTON_WIDTH * k).min(width.max(0.0));
        let button_width = buttons / 2.0;
        // The caption controls keep their Windows width unless the whole strip
        // is narrower than them; in that case their reservation still leaves
        // zero tab space instead of letting tabs paint over them.
        let caption = (3.0 * CAPTION_BUTTON_WIDTH * k).min(width.max(0.0));
        let available = (width - buttons - caption).max(0.0);
        let cap = if titles.is_empty() {
            max_tab
        } else {
            (available / titles.len() as f32).min(max_tab)
        };
        let floor = min_tab.min(cap);

        let mut tabs = Vec::with_capacity(titles.len());
        let mut x = 0.0;
        for (index, title) in titles.iter().enumerate() {
            let w = (width_of(title) + title_size).clamp(floor, cap);
            let rect = Rect {
                x,
                y: 0.0,
                w,
                h: strip_h,
            };
            // A tab narrower than its padding plus the glyph cannot host a
            // close box inside itself: giving it one would bleed into the
            // neighbour, and `hit` tests every close box before any body, so a
            // click on that tab's edge would close the wrong shell. Drop the
            // target instead; the whole narrow tab selects.
            let close_w = if rect.w >= tab_padding + close_size {
                close_size
            } else {
                0.0
            };
            let close = Rect {
                x: rect.x + rect.w - tab_padding - close_size,
                y: (strip_h - close_size) / 2.0,
                w: close_w,
                h: close_size,
            };
            tabs.push(TabRect { index, rect, close });
            x += w;
        }

        let plus = Rect {
            x,
            y: 0.0,
            w: button_width,
            h: strip_h,
        };
        let chevron = Rect {
            x: plus.x + button_width,
            y: 0.0,
            w: button_width,
            h: strip_h,
        };
        // Pinned right to left so the close control's right edge is the
        // strip's own right edge, in the Windows caption order.
        let controls_right = width;
        let control_w = CAPTION_BUTTON_WIDTH * k;
        let close_window = Rect {
            x: controls_right - control_w,
            y: 0.0,
            w: control_w,
            h: strip_h,
        };
        let maximize = Rect {
            x: close_window.x - control_w,
            y: 0.0,
            w: control_w,
            h: strip_h,
        };
        let minimize = Rect {
            x: maximize.x - control_w,
            y: 0.0,
            w: control_w,
            h: strip_h,
        };
        Self {
            width,
            strip_h,
            tabs,
            titles: titles.iter().map(|title| (*title).to_string()).collect(),
            plus,
            chevron,
            minimize,
            maximize,
            close_window,
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<TabHit> {
        if y < 0.0 || y >= self.strip_h {
            return None;
        }
        // The controls own the strip's right end before any tab is considered:
        // the reservation keeps tabs away, and this order keeps the rule
        // explicit if a future layout ever crowded them.
        if self.minimize.contains(x, y) {
            return Some(TabHit::Minimize);
        }
        if self.maximize.contains(x, y) {
            return Some(TabHit::Maximize);
        }
        if self.close_window.contains(x, y) {
            return Some(TabHit::CloseWindow);
        }
        if let Some(tab) = self.tabs.iter().find(|tab| tab.close.contains(x, y)) {
            return Some(TabHit::Close(tab.index));
        }
        if let Some(tab) = self.tabs.iter().find(|tab| tab.rect.contains(x, y)) {
            return Some(TabHit::Tab(tab.index));
        }
        if self.plus.contains(x, y) {
            return Some(TabHit::Plus);
        }
        if self.chevron.contains(x, y) {
            return Some(TabHit::Chevron);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(_: &str) -> f32 {
        100.0
    }

    #[test]
    fn tabs_are_laid_out_left_to_right_and_are_capped() {
        let layout = TabBarLayout::new(1200.0, 32.0, &["one", "two", "three"], |_| 200.0);
        assert_eq!(layout.tabs.len(), 3);
        assert!((layout.tabs[0].rect.x - 0.0).abs() < f32::EPSILON);
        assert!(layout.tabs[1].rect.x > layout.tabs[0].rect.x);
        assert!(layout.tabs[2].rect.x > layout.tabs[1].rect.x);
        assert!((layout.tabs[0].rect.w - MAX_TAB_WIDTH).abs() < f32::EPSILON);

        // A title narrower than the cap hugs its own text plus the chrome.
        let short = TabBarLayout::new(1200.0, 32.0, &["one"], |_| 30.0);
        assert_eq!(short.tabs[0].rect.w, 30.0 + TITLE_SIZE);
    }

    #[test]
    fn a_short_title_hugs_its_text() {
        let layout = TabBarLayout::new(1200.0, 32.0, &["one"], |_| 30.0);
        assert_eq!(layout.tabs[0].rect.w, 30.0 + TITLE_SIZE);
    }

    #[test]
    fn a_narrow_tab_still_leaves_room_for_every_chrome_piece() {
        // Both are layout constants, so the invariant is checked at compile
        // time rather than re-evaluated by every test run.
        const { assert!(MIN_TAB_WIDTH >= TITLE_SIZE) };
    }

    #[test]
    fn plus_and_chevron_sit_after_the_last_tab() {
        let layout = TabBarLayout::new(1200.0, 32.0, &["one"], fixed);
        let tab_end = layout.tabs[0].rect.x + layout.tabs[0].rect.w;
        assert!(layout.plus.x >= tab_end);
        assert!(layout.chevron.x > layout.plus.x);
    }

    #[test]
    fn hit_test_finds_tab_close_plus_and_chevron() {
        let layout = TabBarLayout::new(1200.0, 32.0, &["one", "two"], fixed);
        let first = layout.tabs[0].rect;
        // the top RESIZE_BORDER px is the window's resize border
        assert_eq!(
            layout.hit(first.x + 4.0, first.y + 8.0),
            Some(TabHit::Tab(0))
        );
        let close = layout.tabs[0].close;
        assert_eq!(
            layout.hit(close.x + close.w / 2.0, close.y + close.h / 2.0),
            Some(TabHit::Close(0))
        );
        // 8 px down clears the window's 6 px top resize border (Task 1's
        // `WM_NCHITTEST` subclass), so these probe the click, not the frame.
        assert_eq!(
            layout.hit(layout.plus.x + 2.0, layout.plus.y + 8.0),
            Some(TabHit::Plus)
        );
        assert_eq!(
            layout.hit(layout.chevron.x + 2.0, layout.chevron.y + 8.0),
            Some(TabHit::Chevron)
        );
    }

    #[test]
    fn the_caption_buttons_are_pinned_to_the_right_edge() {
        let layout = TabBarLayout::new(400.0, 32.0, &["one"], |_| 40.0);
        assert_eq!(layout.close_window.x + layout.close_window.w, layout.width);
        assert_eq!(layout.close_window.x, layout.maximize.x + layout.maximize.w);
        assert_eq!(layout.maximize.x, layout.minimize.x + layout.minimize.w);
        assert_eq!(layout.minimize.w, CAPTION_BUTTON_WIDTH);
        // Every control is a full-height strip cell, in the Windows order.
        for rect in [layout.minimize, layout.maximize, layout.close_window] {
            assert_eq!(rect.y, 0.0);
            assert_eq!(rect.h, layout.strip_h);
        }
    }

    #[test]
    fn tabs_stop_before_the_caption_buttons() {
        // Ten tabs whose fair share is far below `MIN_TAB_WIDTH`: with the
        // 138 px caption reservation the tab row ends at `width - 202`, so the
        // `+`/`⌄` pair and the last tab stay clear of the controls. Without the
        // reservation the row would reach `width - 64` and paint under them.
        let titles: Vec<&str> = (0..10).map(|_| "one").collect();
        let layout = TabBarLayout::new(1200.0, 32.0, &titles, |_| 100.0);
        let last = layout.tabs.last().expect("a tab");
        assert!(layout.plus.x + layout.plus.w <= layout.minimize.x);
        assert!(layout.chevron.x + layout.chevron.w <= layout.minimize.x);
        assert!(last.rect.x + last.rect.w <= layout.minimize.x);
    }

    #[test]
    fn the_caption_buttons_stay_reachable_on_a_crowded_strip() {
        // 30 tabs of 32 px each fill exactly the 960 px left after the 138 px
        // caption and 64 px button reservations at this width, so the painted
        // boundary is exact and not just ordered.
        let titles: Vec<&str> = (0..30).map(|_| "a long shell title").collect();
        let layout = TabBarLayout::new(1162.0, 32.0, &titles, |_| 120.0);
        assert_eq!(
            layout.chevron.x + layout.chevron.w,
            layout.minimize.x,
            "the reserved `+`/`⌄` pair must end exactly where the controls begin"
        );
        assert!(layout.plus.x + layout.plus.w <= layout.minimize.x);
        let last = layout.tabs.last().expect("a tab");
        assert!(last.rect.x + last.rect.w <= layout.minimize.x);

        let controls = [
            (layout.minimize, TabHit::Minimize),
            (layout.maximize, TabHit::Maximize),
            (layout.close_window, TabHit::CloseWindow),
        ];
        for (rect, expected) in controls {
            assert!(rect.x >= 0.0 && rect.x + rect.w <= layout.width);
            // 8 px down clears the 6 px top resize border, which starts a
            // resize rather than a click; the button itself is full height.
            assert_eq!(layout.hit(rect.x + 8.0, rect.y + 8.0), Some(expected));
        }
    }

    #[test]
    fn the_buttons_stay_inside_a_strip_crowded_with_tabs() {
        // 20 tabs on a 1200 px strip: the fair share is far below
        // `MIN_TAB_WIDTH`, and the two buttons must still be reachable.
        let titles: Vec<String> = (0..20).map(|index| format!("tab {index}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let layout = TabBarLayout::new(1200.0, 32.0, &refs, fixed);
        assert_eq!(layout.tabs.len(), 20);
        assert!(
            layout.tabs[0].rect.w < MIN_TAB_WIDTH,
            "a crowded strip shrinks tabs below the minimum instead of overflowing"
        );
        assert!(layout.plus.x + layout.plus.w <= layout.width);
        assert!(layout.chevron.x + layout.chevron.w <= layout.width);
        // the top RESIZE_BORDER px is the window's resize border
        assert_eq!(
            layout.hit(layout.plus.x + 4.0, 10.0),
            Some(TabHit::Plus),
            "the plus button must still be clickable"
        );
        // the top RESIZE_BORDER px is the window's resize border
        assert_eq!(
            layout.hit(layout.chevron.x + 4.0, 10.0),
            Some(TabHit::Chevron),
            "the chevron button must still be clickable"
        );
    }

    #[test]
    fn a_tab_too_narrow_for_a_close_box_has_no_close_target() {
        // 60 tabs on a 600 px strip leave about 9 px each, well under the
        // padding plus the glyph, so no tab can own a close box.
        let titles: Vec<String> = (0..60).map(|index| format!("t{index}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let layout = TabBarLayout::new(600.0, 32.0, &refs, fixed);
        assert!(layout.tabs.iter().all(|tab| tab.close.w == 0.0));

        // The right edge of the second tab is that tab, not a close on it or
        // on its neighbour.
        let second = layout.tabs[1].rect;
        assert_eq!(
            layout.hit(second.x + second.w - 1.0, 10.0),
            Some(TabHit::Tab(1))
        );
    }

    #[test]
    fn the_gap_before_the_caption_buttons_is_empty_space() {
        let layout = TabBarLayout::new(400.0, 32.0, &["one"], |_| 40.0);
        // The gap after the `⌄` — not after the `+`, which is still inside the
        // chevron at this width — is the empty band that doubles as the window's
        // drag handle.
        let x = layout.chevron.x + layout.chevron.w + 4.0;
        assert!(x < layout.minimize.x, "the test needs empty space to probe");
        assert_eq!(layout.hit(x, 16.0), None);
    }

    #[test]
    fn the_strip_is_a_single_row_of_the_requested_height() {
        // The strip height is physical and comes from the caller; a scaled
        // display passes 40 here and both the rects and the hit test follow.
        let layout = TabBarLayout::new(1200.0, 40.0, &["one"], fixed);
        let k = 40.0 / TAB_STRIP_HEIGHT;
        assert_eq!(layout.strip_h, 40.0);
        assert_eq!(layout.tabs[0].rect.y, 0.0);
        assert_eq!(layout.tabs[0].rect.h, 40.0);
        assert_eq!(layout.plus.h, 40.0);
        assert_eq!(layout.plus.w, BUTTON_WIDTH * k);
        assert_eq!(layout.tabs[0].close.w, CLOSE_SIZE * k);
        assert_eq!(layout.hit(10.0, 39.0), Some(TabHit::Tab(0)));
        assert_eq!(layout.hit(10.0, 40.0), None);
    }

    #[test]
    fn geometry_scales_with_the_strip_height() {
        let k = 48.0 / TAB_STRIP_HEIGHT;
        let layout = TabBarLayout::new(1200.0, 48.0, &["one"], fixed);
        let tab = &layout.tabs[0];
        assert_eq!(layout.plus.w, BUTTON_WIDTH * k);
        assert_eq!(layout.chevron.w, BUTTON_WIDTH * k);
        assert_eq!(tab.close.w, CLOSE_SIZE * k);
        assert_eq!(tab.close.h, CLOSE_SIZE * k);
        assert_eq!(
            tab.close.x,
            tab.rect.x + tab.rect.w - TAB_PADDING * k - CLOSE_SIZE * k
        );
        assert_eq!(tab.close.y, (48.0 - CLOSE_SIZE * k) / 2.0);

        // The title allowance scales with the em size, so the same label wants
        // a wider tab in a taller strip.
        let short = TabBarLayout::new(1200.0, 32.0, &["x"], fixed);
        let tall = TabBarLayout::new(1200.0, 48.0, &["x"], fixed);
        assert!(tall.tabs[0].rect.w > short.tabs[0].rect.w);
    }
}
