//! The find bar's geometry: a floating panel near the top-right of the window.
//!
//! Coordinates are physical pixels, scaled once by the caller so the bar's
//! size tracks the window's DPI like the rest of the chrome.

use crate::ui::tab_bar::Rect;

pub const FIND_BAR_WIDTH: f32 = 320.0;
pub const FIND_BAR_HEIGHT: f32 = 36.0;

pub struct FindBar {
    pub rect: Rect,
    pub field: Rect,
    pub close: Rect,
}

pub enum FindHit {
    Field,
    Close,
    Outside,
}

impl FindBar {
    pub fn new(window_width: f32, top: f32, scale: f32) -> Self {
        let width = FIND_BAR_WIDTH * scale;
        let height = FIND_BAR_HEIGHT * scale;
        let rect = Rect {
            x: (window_width - width - 12.0 * scale).max(0.0),
            y: top + 8.0 * scale,
            w: width.min(window_width),
            h: height,
        };
        let close = Rect {
            x: rect.x + rect.w - 28.0 * scale,
            y: rect.y + 6.0 * scale,
            w: 20.0 * scale,
            h: 24.0 * scale,
        };
        let field = Rect {
            x: rect.x + 10.0 * scale,
            y: rect.y + 6.0 * scale,
            w: rect.w - 44.0 * scale,
            h: 24.0 * scale,
        };
        Self { rect, field, close }
    }

    pub fn hit(&self, x: f32, y: f32) -> FindHit {
        if self.close.contains(x, y) {
            FindHit::Close
        } else if self.rect.contains(x, y) {
            FindHit::Field
        } else {
            FindHit::Outside
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FindBar, FindHit};

    #[test]
    fn the_close_box_is_hit_before_the_field() {
        let bar = FindBar::new(1000.0, 0.0, 1.0);
        assert!(matches!(
            bar.hit(bar.close.x + 1.0, bar.close.y + 1.0),
            FindHit::Close
        ));
    }

    #[test]
    fn the_field_is_hit_inside_the_panel() {
        let bar = FindBar::new(1000.0, 0.0, 1.0);
        assert!(matches!(
            bar.hit(bar.field.x + 1.0, bar.field.y + 1.0),
            FindHit::Field
        ));
        // The panel body outside the field (but inside the bar) also reads as
        // the field, so a click anywhere in the bar focuses it.
        assert!(matches!(
            bar.hit(bar.rect.x + 1.0, bar.rect.y + 1.0),
            FindHit::Field
        ));
    }

    #[test]
    fn a_click_outside_the_bar_is_outside() {
        let bar = FindBar::new(1000.0, 0.0, 1.0);
        assert!(matches!(bar.hit(-5.0, -5.0), FindHit::Outside));
    }

    #[test]
    fn a_narrow_window_keeps_the_bar_inside() {
        let bar = FindBar::new(200.0, 0.0, 1.0);
        assert!(bar.rect.x >= 0.0);
        assert!(bar.rect.x + bar.rect.w <= 200.0);
    }
}
