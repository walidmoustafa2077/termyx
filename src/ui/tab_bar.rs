//! Tab strip geometry. Pure coordinates: the renderer and the hit test share
//! one description of where every control sits.

/// Height of the strip in logical pixels; the terminal grid starts below it.
///
/// Windows Terminal 1.24 at 100 % paints a 39 px strip plus a 1 px rule, 40 in
/// total, and its tab text is 14 px. This constant is the 100 % reference for
/// the whole layout scale `k = strip_h / TAB_STRIP_HEIGHT`: changing it does
/// not scale the tab width, the caption width, or the font, it only makes the
/// band itself taller.
pub const TAB_STRIP_HEIGHT: f32 = 40.0;
/// Strip px WT leaves above a tab.
///
/// The tab is 32 px tall and floats on the 40 px band, so 8 px of strip shows
/// above it and the tab reads as a rounded card rather than a full-height cell.
pub const TAB_TOP_INSET: f32 = 8.0;
/// Widest a single tab grows; the equal share is capped here.
pub const MAX_TAB_WIDTH: f32 = 240.0;
/// Strip px Windows Terminal leaves before the first tab.
///
/// WT's first tab outline sits at x=7 with its body starting at x=8, so the
/// strip keeps an ~8 px left margin. Without it the active tab's rounded left
/// corner and outline are clipped by the window edge.
pub const TAB_LEFT_PAD: f32 = 8.0;
/// Gap in strip px between the last tab and the `+` button.
///
/// WT's `+` sits a few px right of the last tab, and the only separator on that
/// run of strip is the one between `+` and `⌄` (measured: the separator is the
/// last tab end + 37 at one, two and three 240 px tabs, with 32 px buttons).
pub const TAB_BUTTON_GAP: f32 = 4.0;
/// The width below which the strip counts as crowded, pinned by the tests.
///
/// The layout applies no floor — Windows Terminal enforces only the
/// [`MAX_TAB_WIDTH`] cap, and tabs shrink further when they must — so this is
/// a reference value for the crowding assertions, not a limit the painter
/// reads. Test-only so an unused production constant cannot drift.
#[cfg(test)]
pub const MIN_TAB_WIDTH: f32 = 80.0;
/// Width of the `+` and `⌄` buttons.
pub const BUTTON_WIDTH: f32 = 32.0;
/// Width of the empty drag band kept between the `⌄` and the caption controls.
///
/// Tabs are sized from the space left after the buttons and the controls. At a
/// crowded width that share can absorb the entire band, leaving no pixel to
/// press for moving the window; reserving this gutter keeps the title-bar
/// gesture reachable no matter how many tabs are open.
pub const DRAG_GUTTER: f32 = 24.0;
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
    /// The full-band cell, `y = 0`, `h = strip_h`, that `hit` claims so a
    /// click in the 8 px of strip above a tab still selects that tab.
    pub rect: Rect,
    /// The painted tab body, inset by [`TAB_TOP_INSET`] so it floats on the
    /// strip; everything the tab draws is positioned relative to this.
    pub body: Rect,
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
    /// The two buttons and the three window controls are reserved before any
    /// tab is placed, and every tab then claims the same share of the width
    /// left after the drag gutter, the gap before `+` and the strip's left
    /// margin, capped at [`MAX_TAB_WIDTH`]. That is the equal division Windows
    /// Terminal uses: a title never drives the width, so one tab on a wide
    /// window takes the full 240 px.
    ///
    /// A crowded strip shrinks every tab below the 80 px reference width
    /// instead of pushing a control past the window edge or swallowing the
    /// empty band that moves the window, so `+`, `⌄` and the caption buttons
    /// stay clickable and the window stays draggable.
    pub fn new(width: f32, strip_h: f32, titles: &[&str]) -> Self {
        let strip_h = strip_h.max(0.0);
        let k = strip_h / TAB_STRIP_HEIGHT;
        let max_tab = MAX_TAB_WIDTH * k;
        let close_size = CLOSE_SIZE * k;
        let tab_padding = TAB_PADDING * k;
        // WT keeps a small strip margin before the first tab so its rounded
        // outline is not clipped, and a small gap before the `+` button.
        let pad = TAB_LEFT_PAD * k;
        let gap = TAB_BUTTON_GAP * k;
        // A window narrower than the button pair shrinks both together, so the
        // strip always ends exactly at `width` and neither button is clipped.
        let buttons = (2.0 * BUTTON_WIDTH * k).min(width.max(0.0));
        let button_width = buttons / 2.0;
        // The caption controls keep their Windows width unless the whole strip
        // is narrower than them; in that case their reservation still leaves
        // zero tab space instead of letting tabs paint over them.
        let caption = (3.0 * CAPTION_BUTTON_WIDTH * k).min(width.max(0.0));
        // Reserve the drag gutter before sizing tabs, not after: the tabs take
        // whatever is left, so subtracting it here is what keeps the empty band
        // visible even when every tab is at its cap.
        let gutter = (DRAG_GUTTER * k).min((width - buttons - caption).max(0.0));
        let available = (width - buttons - caption - gutter - gap - pad).max(0.0);
        // Equal division, capped. No floor: when the share drops below the
        // 80 px reference width the strip is crowded and the smaller share is
        // taken as it is, rather than overflowing the controls.
        let tab_width = if titles.is_empty() {
            max_tab
        } else {
            (available / titles.len() as f32).min(max_tab)
        };

        let mut tabs = Vec::with_capacity(titles.len());
        let mut x = pad;
        for index in 0..titles.len() {
            let rect = Rect {
                x,
                y: 0.0,
                w: tab_width,
                h: strip_h,
            };
            // WT floats a 32 px tab on its 40 px band: 8 logical px of strip
            // stay above it. The body, not `rect`, is what the painter fills
            // and strokes, and the close box centres on it.
            let body = Rect {
                x: rect.x,
                y: TAB_TOP_INSET * k,
                w: rect.w,
                h: (strip_h - TAB_TOP_INSET * k).max(0.0),
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
                // Centred on the tab body, not the strip: WT's `✕` sits in
                // the middle of the visible card.
                y: body.y + (body.h - close_size) / 2.0,
                w: close_w,
                h: close_size,
            };
            tabs.push(TabRect {
                index,
                rect,
                body,
                close,
            });
            x += tab_width;
        }

        // The buttons follow the last tab instead of hugging the right edge.
        // The clamp keeps both left of the caption controls on a crowded strip,
        // where the share would otherwise push them under the controls.
        let plus_x = (x + gap).max(pad).min((width - caption - buttons).max(pad));
        let plus = Rect {
            x: plus_x,
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

    #[test]
    fn tabs_are_laid_out_left_to_right_and_are_capped() {
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one", "two", "three"]);
        assert_eq!(layout.tabs.len(), 3);
        assert_eq!(layout.tabs[0].rect.x, TAB_LEFT_PAD);
        assert!(layout.tabs[1].rect.x > layout.tabs[0].rect.x);
        assert!(layout.tabs[2].rect.x > layout.tabs[1].rect.x);
        assert!((layout.tabs[0].rect.w - MAX_TAB_WIDTH).abs() < f32::EPSILON);
    }

    #[test]
    fn a_single_tab_on_a_wide_window_is_the_reference_width() {
        // Equal division, not title-driven: one tab on a 1200 px strip takes
        // the full 240 px cap even though `one` is a very short title.
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        assert_eq!(layout.tabs[0].rect.w, MAX_TAB_WIDTH);
    }

    #[test]
    fn every_tab_gets_the_same_share_until_the_cap() {
        // Three tabs on 1200 px: `(1200 - 64 - 138 - 24 - 4 - 8) / 3` is above
        // the cap, so all three are exactly the cap wide.
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one", "two", "three"]);
        for tab in &layout.tabs {
            assert_eq!(tab.rect.w, MAX_TAB_WIDTH);
        }

        // Six tabs divide the same room, so every tab is narrower and they
        // share one width regardless of their titles.
        let layout = TabBarLayout::new(
            1200.0,
            TAB_STRIP_HEIGHT,
            &["a", "a much longer title", "c", "d", "e", "f"],
        );
        let first = layout.tabs[0].rect.w;
        assert!(first < MAX_TAB_WIDTH);
        assert!(layout.tabs.iter().all(|tab| tab.rect.w == first));
    }

    #[test]
    fn a_narrow_tab_still_leaves_room_for_every_chrome_piece() {
        // Both are layout constants, so the invariant is checked at compile
        // time rather than re-evaluated by every test run.
        const { assert!(MIN_TAB_WIDTH >= TITLE_SIZE) };
    }

    #[test]
    fn plus_and_chevron_sit_after_the_last_tab() {
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        let tab_end = layout.tabs[0].rect.x + layout.tabs[0].rect.w;
        assert!(layout.plus.x >= tab_end);
        assert!(layout.chevron.x > layout.plus.x);
    }

    #[test]
    fn the_first_tab_gets_the_strip_left_margin() {
        // WT's first tab outline sits at x=7 and its body starts at x=8, so
        // ours starts at the 8 px pad and still takes the full 240 px cap.
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        assert_eq!(layout.tabs[0].rect.x, 8.0);
        assert_eq!(layout.tabs[0].body.x, 8.0);
        assert_eq!(layout.tabs[0].rect.w, 240.0);
        // The `+` follows the tab after a 4 px gap; `⌄` starts one button later.
        assert_eq!(layout.plus.x, 252.0);
        assert_eq!(layout.chevron.x, 284.0);
    }

    #[test]
    fn the_button_block_tracks_the_last_tab_on_two_and_three_tabs() {
        // Measured WT: the `+`/`⌄` boundary is the last tab end + 37 at one,
        // two and three 240 px tabs; with our 4 px gap it lands at end + 36.
        // Either way the block moves with the tabs rather than being pinned
        // to the right edge.
        let two = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one", "two"]);
        assert_eq!(two.chevron.x, 524.0);
        let three = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one", "two", "three"]);
        assert_eq!(three.chevron.x, 764.0);
        for layout in [&two, &three] {
            assert_eq!(layout.chevron.x, layout.plus.x + BUTTON_WIDTH);
        }
    }

    #[test]
    fn eight_tabs_keep_the_buttons_clear_of_the_caption_controls() {
        let titles: Vec<&str> = (0..8).map(|_| "one").collect();
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &titles);
        // WT's three 46 px caption controls reserve 138 px; the button block
        // must end before them and every tab must remain a target.
        assert!(
            layout.chevron.x + layout.chevron.w <= 1200.0 - 3.0 * CAPTION_BUTTON_WIDTH,
            "the button block must stay left of the caption controls"
        );
        assert!(
            layout.tabs.iter().all(|tab| tab.rect.w >= 1.0),
            "every tab must keep a clickable width"
        );
    }

    #[test]
    fn the_strip_left_of_the_first_tab_is_not_a_tab() {
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        assert_eq!(layout.hit(TAB_LEFT_PAD - 1.0, 10.0), None);
        assert_eq!(layout.hit(0.0, 10.0), None);
    }

    #[test]
    fn the_button_rects_still_hit_plus_and_chevron() {
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one", "two", "three"]);
        assert_eq!(layout.hit(layout.plus.x + 2.0, 10.0), Some(TabHit::Plus));
        assert_eq!(
            layout.hit(layout.chevron.x + 2.0, 10.0),
            Some(TabHit::Chevron)
        );
    }

    #[test]
    fn hit_test_finds_tab_close_plus_and_chevron() {
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one", "two"]);
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
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
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
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &titles);
        let last = layout.tabs.last().expect("a tab");
        assert!(layout.plus.x + layout.plus.w <= layout.minimize.x);
        assert!(layout.chevron.x + layout.chevron.w <= layout.minimize.x);
        assert!(last.rect.x + last.rect.w <= layout.minimize.x);
    }

    #[test]
    fn the_caption_buttons_stay_reachable_on_a_crowded_strip() {
        // 30 tabs whose fair share is below `MIN_TAB_WIDTH`; the tab row ends
        // at the drag gutter, which itself ends exactly where the controls
        // begin, so the painted boundary is exact and not just ordered.
        let titles: Vec<&str> = (0..30).map(|_| "a long shell title").collect();
        let layout = TabBarLayout::new(1162.0, TAB_STRIP_HEIGHT, &titles);
        assert!(
            (layout.chevron.x + layout.chevron.w + DRAG_GUTTER - layout.minimize.x).abs() < 0.01,
            "the drag gutter must occupy exactly the space between `⌄` and the controls"
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
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &refs);
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
    fn a_crowded_strip_keeps_a_drag_gutter_before_the_caption_controls() {
        // The tab budget is `min(max_tab, available / n)`, so without a
        // reservation `n * max_tab >= available` lets the tabs, `+` and `⌄`
        // reach the controls and leaves no pixel to drag the window. Probe both
        // the default and the minimum window width with 30 long-titled tabs.
        let titles: Vec<String> = (0..30)
            .map(|index| format!("a long shell title {index}"))
            .collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        for width in [1200.0, 1024.0] {
            let layout = TabBarLayout::new(width, TAB_STRIP_HEIGHT, &refs);
            let gutter_left = layout.chevron.x + layout.chevron.w;
            assert!(
                gutter_left + DRAG_GUTTER <= layout.minimize.x + 0.01,
                "the drag gutter must survive at width {width}"
            );
            let midpoint = (gutter_left + layout.minimize.x) / 2.0;
            assert_eq!(
                layout.hit(midpoint, 16.0),
                None,
                "the gutter's midpoint must belong to no button at width {width}"
            );
        }
    }

    #[test]
    fn a_tab_too_narrow_for_a_close_box_has_no_close_target() {
        // 60 tabs on a 600 px strip leave about 9 px each, well under the
        // padding plus the glyph, so no tab can own a close box.
        let titles: Vec<String> = (0..60).map(|index| format!("t{index}")).collect();
        let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
        let layout = TabBarLayout::new(600.0, TAB_STRIP_HEIGHT, &refs);
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
        let layout = TabBarLayout::new(400.0, TAB_STRIP_HEIGHT, &["one"]);
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
        // display passes 44 here and both the rects and the hit test follow.
        let layout = TabBarLayout::new(1200.0, 44.0, &["one"]);
        let k = 44.0 / TAB_STRIP_HEIGHT;
        assert_eq!(layout.strip_h, 44.0);
        assert_eq!(layout.tabs[0].rect.y, 0.0);
        assert_eq!(layout.tabs[0].rect.h, 44.0);
        assert_eq!(layout.plus.h, 44.0);
        assert_eq!(layout.plus.w, BUTTON_WIDTH * k);
        assert_eq!(layout.tabs[0].close.w, CLOSE_SIZE * k);
        assert_eq!(layout.hit(10.0, 43.0), Some(TabHit::Tab(0)));
        assert_eq!(layout.hit(10.0, 44.0), None);
    }

    #[test]
    fn a_tab_floats_on_the_band_with_an_eight_pixel_inset() {
        // At 100 % the tab body is 32 px tall on the 40 px band, leaving 8 px
        // of strip above it; `rect` still claims the full band for hit testing.
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        let tab = &layout.tabs[0];
        assert_eq!(tab.rect.y, 0.0);
        assert_eq!(tab.rect.h, 40.0);
        assert_eq!(tab.body.y, 8.0);
        assert_eq!(tab.body.h, 32.0);
        assert_eq!(tab.body.w, tab.rect.w);
    }

    #[test]
    fn a_click_in_the_strip_above_a_tab_still_selects_it() {
        let layout = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["one"]);
        // 3 px down is strip above the tab body, but inside the tab's full-band
        // `rect`, so it selects the tab rather than falling through.
        assert_eq!(
            layout.hit(layout.tabs[0].rect.x + 4.0, 3.0),
            Some(TabHit::Tab(0))
        );
        // The `+` button's own band above its glyph is still the button.
        assert_eq!(layout.hit(layout.plus.x + 4.0, 3.0), Some(TabHit::Plus));
    }

    #[test]
    fn geometry_scales_with_the_strip_height() {
        let k = 48.0 / TAB_STRIP_HEIGHT;
        let layout = TabBarLayout::new(1200.0, 48.0, &["one"]);
        let tab = &layout.tabs[0];
        assert_eq!(layout.plus.w, BUTTON_WIDTH * k);
        assert_eq!(layout.chevron.w, BUTTON_WIDTH * k);
        assert_eq!(tab.close.w, CLOSE_SIZE * k);
        assert_eq!(tab.close.h, CLOSE_SIZE * k);
        assert_eq!(
            tab.close.x,
            tab.rect.x + tab.rect.w - TAB_PADDING * k - CLOSE_SIZE * k
        );
        assert_eq!(tab.body.y, TAB_TOP_INSET * k);
        assert_eq!(tab.body.h, 48.0 - TAB_TOP_INSET * k);
        // The close box centres on the body, so it stays visually in the middle
        // of the card the user sees.
        assert_eq!(
            tab.close.y,
            tab.body.y + (tab.body.h - CLOSE_SIZE * k) / 2.0
        );
        assert_eq!(
            tab.close.y + tab.close.h / 2.0,
            tab.body.y + tab.body.h / 2.0,
            "the close box's vertical centre must equal the body's centre"
        );

        // The cap scales with the strip height, so the same strip gives a
        // wider tab on a taller (higher-DPI) window.
        let short = TabBarLayout::new(1200.0, TAB_STRIP_HEIGHT, &["x"]);
        let tall = TabBarLayout::new(1200.0, 48.0, &["x"]);
        assert!(tall.tabs[0].rect.w > short.tabs[0].rect.w);
    }
}
