//! Tooltip geometry: the small panel shown when the pointer rests on a control.

use crate::ui::tab_bar::{Rect, TAB_STRIP_HEIGHT};

/// Em size of a tooltip's text, the same size the menu rows use.
pub const TOOLTIP_TEXT_PX: f32 = 12.0;
/// Inset of the text from the panel's left and right edges.
pub const TOOLTIP_PADDING_X: f32 = 9.0;
/// Inset of the text from the panel's top and bottom edges.
pub const TOOLTIP_PADDING_Y: f32 = 5.0;
/// Corner radius of the panel, WinUI's overlay radius.
pub const TOOLTIP_RADIUS: f32 = 4.0;
/// Space between the control and the panel.
pub const TOOLTIP_GAP: f32 = 4.0;
/// How long the pointer must rest on a control before its tooltip shows.
///
/// WinUI's `ToolTipService` default. Showing sooner would flash a label at
/// every pass over the strip, showing later would feel unresponsive.
pub const TOOLTIP_DELAY_MS: u64 = 400;

/// A tooltip panel: where it sits, what it says, and the scale it was laid out
/// at.
#[derive(Debug, Clone, PartialEq)]
pub struct Tooltip {
    pub rect: Rect,
    pub text: String,
    /// The strip's scale, carried from the anchor so the painter can size its
    /// radius and text without being told a second time.
    pub scale: f32,
}

impl Tooltip {
    /// Places a panel under `anchor`, centred on it, and keeps it inside the
    /// window on both axes.
    ///
    /// `text_width` and `line_height` come from the font set: the model must
    /// not measure text itself. The panel flips above the anchor only when it
    /// would otherwise leave the bottom edge; `None` when the text is empty or
    /// the panel cannot fit the window at all.
    pub fn new(
        text: &str,
        anchor: Rect,
        text_width: f32,
        line_height: f32,
        window_width: f32,
        window_height: f32,
    ) -> Option<Self> {
        if text.is_empty() {
            return None;
        }
        let k = anchor.h / TAB_STRIP_HEIGHT;
        let width = text_width + 2.0 * TOOLTIP_PADDING_X * k;
        let height = line_height + 2.0 * TOOLTIP_PADDING_Y * k;
        if width > window_width || height > window_height {
            return None;
        }
        let x = (anchor.x + (anchor.w - width) / 2.0).clamp(0.0, window_width - width);
        let below = anchor.y + anchor.h + TOOLTIP_GAP * k;
        let y = if below + height <= window_height {
            below
        } else {
            (anchor.y - TOOLTIP_GAP * k - height).max(0.0)
        };
        Some(Self {
            rect: Rect {
                x,
                y,
                w: width,
                h: height,
            },
            text: text.to_string(),
            scale: k,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anchor() -> Rect {
        Rect {
            x: 40.0,
            y: 0.0,
            w: 200.0,
            h: TAB_STRIP_HEIGHT,
        }
    }

    #[test]
    fn the_panel_sits_centred_under_its_control() {
        let anchor = anchor();
        let tip = Tooltip::new("New tab", anchor, 40.0, 14.0, 800.0, 600.0)
            .expect("a tooltip must fit a normal window");
        assert_eq!(tip.rect.w, 40.0 + 2.0 * TOOLTIP_PADDING_X);
        assert_eq!(tip.rect.h, 14.0 + 2.0 * TOOLTIP_PADDING_Y);
        assert_eq!(tip.rect.y, anchor.y + anchor.h + TOOLTIP_GAP);
        assert_eq!(
            tip.rect.x + tip.rect.w / 2.0,
            anchor.x + anchor.w / 2.0,
            "the panel is centred on the control it describes"
        );
    }

    #[test]
    fn a_panel_past_the_right_edge_is_pulled_inside() {
        let anchor = Rect {
            x: 760.0,
            y: 0.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let tip = Tooltip::new("Close", anchor, 30.0, 14.0, 800.0, 600.0)
            .expect("a tooltip must fit a normal window");
        assert_eq!(tip.rect.x + tip.rect.w, 800.0);
        assert!(tip.rect.x >= 0.0);
    }

    #[test]
    fn a_panel_past_the_bottom_edge_flips_above() {
        let anchor = Rect {
            x: 10.0,
            y: 560.0,
            w: 32.0,
            h: TAB_STRIP_HEIGHT,
        };
        let tip = Tooltip::new("Minimize", anchor, 50.0, 14.0, 800.0, 600.0)
            .expect("a tooltip must fit a normal window");
        assert!(
            tip.rect.y + tip.rect.h <= anchor.y,
            "the panel must sit above the control rather than leave the window"
        );
    }

    #[test]
    fn an_empty_label_gets_no_panel() {
        assert_eq!(Tooltip::new("", anchor(), 0.0, 14.0, 800.0, 600.0), None);
    }

    #[test]
    fn the_panel_scales_with_the_strip_height() {
        let anchor = Rect {
            x: 40.0,
            y: 0.0,
            w: 200.0,
            h: TAB_STRIP_HEIGHT * 2.0,
        };
        let tip = Tooltip::new("New tab", anchor, 80.0, 28.0, 1600.0, 1200.0)
            .expect("a tooltip must fit a normal window");
        assert_eq!(tip.scale, 2.0);
        assert_eq!(tip.rect.h, 28.0 + 4.0 * TOOLTIP_PADDING_Y);
        assert_eq!(tip.rect.y, anchor.h + 2.0 * TOOLTIP_GAP);
    }
}
