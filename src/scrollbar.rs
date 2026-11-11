//! Scrollbar geometry and drag-to-offset mapping.

/// Width of the scrollbar thumb at rest, in physical pixels at 100% scale.
pub const WIDTH: f32 = 6.0;
/// Width of the scrollbar thumb while hovered or dragged, at 100% scale.
pub const HOVER_WIDTH: f32 = 12.0;
const MIN_THUMB_H: f32 = 12.0;
const EDGE_GAP: f32 = 2.0;

/// Space reserved at the right edge of the grid for the scrollbar at `scale`.
///
/// The reservation is the hovered width plus the edge gap, so text never sits
/// under the expanded thumb. It grows with the display alongside the text, so
/// a 150% window still keeps the wide thumb clear of the last column.
pub fn gutter(scale: f32) -> f32 {
    (HOVER_WIDTH + EDGE_GAP) * scale
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarGeometry {
    pub x: f32,
    pub width: f32,
    pub track_y: f32,
    pub track_h: f32,
    pub thumb_y: f32,
    pub thumb_h: f32,
}

/// Which part of the scrollbar the pointer is over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollbarHit {
    /// The pointer is on the thumb, at this vertical offset inside it.
    Thumb(f32),
    /// The pointer is on the track above or below the thumb.
    Track,
}

/// The viewport and pointer state one thumb is painted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarState {
    pub rows: usize,
    pub history: usize,
    pub offset: usize,
    /// Whether the thumb is widened for hover or drag.
    pub hovered: bool,
}

/// Thumb geometry for the given viewport and scrollback state.
/// Returns `None` when there is no history or the viewport is too narrow.
/// `scale` is the display scale factor: the thumb's widths and minimum height
/// are 100%-DPI values multiplied by it. A hovered thumb widens leftward so its
/// right edge stays pinned to the window edge, and at the live bottom it rests
/// at the bottom of the track.
pub fn geometry(
    viewport_width: u32,
    padding: u32,
    cell_h: u32,
    state: BarState,
    scale: f32,
) -> Option<ScrollbarGeometry> {
    if state.history == 0 || state.rows == 0 || cell_h == 0 {
        return None;
    }

    let width = if state.hovered { HOVER_WIDTH } else { WIDTH } * scale;
    let x = viewport_width as f32 - width - EDGE_GAP * scale;
    if x < padding as f32 {
        return None;
    }

    let track_y = padding as f32;
    let track_h = state.rows as f32 * cell_h as f32;
    let total = (state.history + state.rows) as f32;
    let min_thumb_h = MIN_THUMB_H * scale;
    let thumb_h = (track_h * state.rows as f32 / total).clamp(min_thumb_h.min(track_h), track_h);
    let scroll = state.offset.min(state.history) as f32 / state.history as f32;
    let thumb_y = track_y + (track_h - thumb_h) * (1.0 - scroll);

    Some(ScrollbarGeometry {
        x,
        width,
        track_y,
        track_h,
        thumb_y,
        thumb_h,
    })
}

/// Scrollback offset for a thumb top position. Larger offset means scrolled further back.
pub fn offset_for_thumb_top(thumb_top: f32, geometry: &ScrollbarGeometry, history: usize) -> usize {
    if history == 0 {
        return 0;
    }
    let travel = geometry.track_h - geometry.thumb_h;
    if travel <= 0.0 {
        return 0;
    }
    let top = thumb_top.clamp(geometry.track_y, geometry.track_y + travel);
    let ratio = 1.0 - (top - geometry.track_y) / travel;
    (ratio * history as f32).round().clamp(0.0, history as f32) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Geometry for a 1000 px viewport with 4 px padding and 20 px cells over
    /// ten rows, the shape most of these tests share.
    fn scrollbar(
        history: usize,
        offset: usize,
        hovered: bool,
        scale: f32,
    ) -> Option<ScrollbarGeometry> {
        geometry(
            1000,
            4,
            20,
            BarState {
                rows: 10,
                history,
                offset,
                hovered,
            },
            scale,
        )
    }

    #[test]
    fn thumb_sits_at_bottom_at_live_bottom() {
        let geom = scrollbar(20, 0, false, 1.0).expect("scrollbar at the live bottom");
        assert_eq!(geom.thumb_y + geom.thumb_h, geom.track_y + geom.track_h);
    }

    #[test]
    fn thumb_sits_at_bottom_when_scrolled_one_row_above_live() {
        let geom = scrollbar(20, 1, false, 1.0).expect("scrollbar");
        assert!(geom.thumb_y > geom.track_y);
    }

    #[test]
    fn thumb_sits_at_top_when_scrolled_to_history() {
        let geom = scrollbar(100, 100, false, 1.0).expect("scrollbar");
        assert_eq!(geom.thumb_y, geom.track_y);
    }

    #[test]
    fn offset_round_trips_for_top_middle_and_bottom() {
        let geom = scrollbar(100, 50, false, 1.0).expect("scrollbar");
        assert_eq!(offset_for_thumb_top(geom.track_y, &geom, 100), 100);
        let middle = geom.track_y + (geom.track_h - geom.thumb_h) / 2.0;
        assert_eq!(offset_for_thumb_top(middle, &geom, 100), 50);
        let bottom = geom.track_y + geom.track_h - geom.thumb_h;
        assert_eq!(offset_for_thumb_top(bottom, &geom, 100), 0);
    }

    #[test]
    fn no_scrollbar_without_history_or_space() {
        assert!(scrollbar(0, 0, false, 1.0).is_none());
        assert!(geometry(
            4,
            4,
            20,
            BarState {
                rows: 10,
                history: 20,
                offset: 0,
                hovered: false,
            },
            1.0,
        )
        .is_none());
    }

    #[test]
    fn thumb_has_a_minimum_height() {
        let geom = scrollbar(1000, 500, false, 1.0).expect("scrollbar");
        assert_eq!(geom.thumb_h, 12.0);
    }

    #[test]
    fn hovered_thumb_is_wider_and_keeps_its_right_edge() {
        let rest = scrollbar(100, 50, false, 1.0).expect("scrollbar");
        let hovered = scrollbar(100, 50, true, 1.0).expect("scrollbar");
        assert!(hovered.width > rest.width);
        assert_eq!(hovered.x + hovered.width, rest.x + rest.width);
        assert_eq!(hovered.thumb_y, rest.thumb_y);
        assert_eq!(hovered.thumb_h, rest.thumb_h);
    }

    #[test]
    fn metrics_follow_the_display_scale() {
        // At 200% the resting thumb, its hovered width and the shortest thumb
        // all double, so the bar keeps its weight against doubled text.
        assert_eq!(gutter(1.0), 14.0);
        assert_eq!(gutter(1.5), 21.0);

        let rest = scrollbar(100, 50, false, 2.0).expect("scrollbar");
        assert_eq!(rest.width, 12.0);
        let hovered = scrollbar(100, 50, true, 2.0).expect("scrollbar");
        assert_eq!(hovered.width, 24.0);
        assert_eq!(hovered.x + hovered.width, rest.x + rest.width);

        let short = scrollbar(1000, 500, false, 2.0).expect("scrollbar");
        assert_eq!(short.thumb_h, 24.0);
    }
}
