//! Tab strip geometry. Pure coordinates: the renderer and the hit test share
//! one description of where every control sits.

/// Height of the strip in logical pixels; the terminal grid starts below it.
pub const TAB_STRIP_HEIGHT: f32 = 32.0;
/// Widest a single tab grows before the title is clipped.
pub const MAX_TAB_WIDTH: f32 = 220.0;
/// Narrowest a tab shrinks to while the strip still has room.
pub const MIN_TAB_WIDTH: f32 = 90.0;
/// Width of the `+` and `⌄` buttons.
pub const BUTTON_WIDTH: f32 = 32.0;
const CLOSE_SIZE: f32 = 16.0;
const TAB_PADDING: f32 = 12.0;
// Room beside the title: left inset for the glyph, the close button, padding.
// Wide enough that a normal title reaches `MAX_TAB_WIDTH` and stays capped there.
const TITLE_SIZE: f32 = 120.0;

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabBarLayout {
    pub width: f32,
    pub tabs: Vec<TabRect>,
    pub titles: Vec<String>,
    pub plus: Rect,
    pub chevron: Rect,
}

impl TabBarLayout {
    /// `width_of` measures a title at the UI font size, so a long title both
    /// widens its tab (up to [`MAX_TAB_WIDTH`]) and gets clipped when the strip
    /// runs out of room.
    pub fn new(width: f32, titles: &[&str], width_of: impl Fn(&str) -> f32) -> Self {
        let available = (width - 2.0 * BUTTON_WIDTH).max(MIN_TAB_WIDTH);
        let wanted: Vec<f32> = titles
            .iter()
            .map(|title| (width_of(title) + TITLE_SIZE).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH))
            .collect();
        let total: f32 = wanted.iter().sum();
        let scale = if total > available && total > 0.0 {
            available / total
        } else {
            1.0
        };

        let mut tabs = Vec::with_capacity(titles.len());
        let mut x = 0.0;
        for (index, wanted) in wanted.iter().enumerate() {
            let w = (wanted * scale).max(MIN_TAB_WIDTH.min(available));
            let rect = Rect {
                x,
                y: 0.0,
                w,
                h: TAB_STRIP_HEIGHT,
            };
            let close = Rect {
                x: rect.x + rect.w - TAB_PADDING - CLOSE_SIZE,
                y: (TAB_STRIP_HEIGHT - CLOSE_SIZE) / 2.0,
                w: CLOSE_SIZE,
                h: CLOSE_SIZE,
            };
            tabs.push(TabRect { index, rect, close });
            x += w;
        }

        let plus = Rect {
            x,
            y: 0.0,
            w: BUTTON_WIDTH,
            h: TAB_STRIP_HEIGHT,
        };
        let chevron = Rect {
            x: plus.x + BUTTON_WIDTH,
            y: 0.0,
            w: BUTTON_WIDTH,
            h: TAB_STRIP_HEIGHT,
        };
        Self {
            width,
            tabs,
            titles: titles.iter().map(|title| (*title).to_string()).collect(),
            plus,
            chevron,
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<TabHit> {
        if y < 0.0 || y >= TAB_STRIP_HEIGHT {
            return None;
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
        let layout = TabBarLayout::new(1200.0, &["one", "two", "three"], fixed);
        assert_eq!(layout.tabs.len(), 3);
        assert!((layout.tabs[0].rect.x - 0.0).abs() < f32::EPSILON);
        assert!(layout.tabs[1].rect.x > layout.tabs[0].rect.x);
        assert!(layout.tabs[2].rect.x > layout.tabs[1].rect.x);
        assert!((layout.tabs[0].rect.w - MAX_TAB_WIDTH).abs() < f32::EPSILON);
    }

    #[test]
    fn plus_and_chevron_sit_after_the_last_tab() {
        let layout = TabBarLayout::new(1200.0, &["one"], fixed);
        let tab_end = layout.tabs[0].rect.x + layout.tabs[0].rect.w;
        assert!(layout.plus.x >= tab_end);
        assert!(layout.chevron.x > layout.plus.x);
    }

    #[test]
    fn hit_test_finds_tab_close_plus_and_chevron() {
        let layout = TabBarLayout::new(1200.0, &["one", "two"], fixed);
        let first = layout.tabs[0].rect;
        assert_eq!(
            layout.hit(first.x + 4.0, first.y + 4.0),
            Some(TabHit::Tab(0))
        );
        let close = layout.tabs[0].close;
        assert_eq!(
            layout.hit(close.x + close.w / 2.0, close.y + close.h / 2.0),
            Some(TabHit::Close(0))
        );
        assert_eq!(
            layout.hit(layout.plus.x + 2.0, layout.plus.y + 2.0),
            Some(TabHit::Plus)
        );
        assert_eq!(
            layout.hit(layout.chevron.x + 2.0, layout.chevron.y + 2.0),
            Some(TabHit::Chevron)
        );
    }

    #[test]
    fn the_strip_is_a_single_row() {
        let layout = TabBarLayout::new(1200.0, &["one"], fixed);
        assert_eq!(layout.tabs[0].rect.y, 0.0);
        assert_eq!(layout.tabs[0].rect.h, TAB_STRIP_HEIGHT);
        assert_eq!(layout.hit(600.0, TAB_STRIP_HEIGHT + 1.0), None);
    }
}
