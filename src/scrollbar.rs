//! Scrollbar geometry and drag-to-offset mapping.

/// Width of the scrollbar thumb in pixels at rest.
pub const WIDTH: f32 = 6.0;
/// Width of the scrollbar thumb in pixels while hovered or dragged.
pub const HOVER_WIDTH: f32 = 12.0;
/// Space reserved at the right edge of the grid for the scrollbar. The layout
/// always leaves this much room so text never sits under the hovered thumb.
pub const GUTTER: f32 = HOVER_WIDTH + EDGE_GAP;
const MIN_THUMB_H: f32 = 12.0;
const EDGE_GAP: f32 = 2.0;

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

/// Thumb geometry for the given viewport and scrollback state.
/// Returns `None` when there is no history or the viewport is too narrow.
/// `hovered` widens the thumb leftward so its right edge stays pinned to the
/// window edge. At the live bottom the thumb rests at the bottom of the track.
pub fn geometry(
    viewport_width: u32,
    padding: u32,
    cell_h: u32,
    rows: usize,
    history: usize,
    offset: usize,
    hovered: bool,
) -> Option<ScrollbarGeometry> {
    if history == 0 || rows == 0 || cell_h == 0 {
        return None;
    }

    let width = if hovered { HOVER_WIDTH } else { WIDTH };
    let x = viewport_width as f32 - width - EDGE_GAP;
    if x < padding as f32 {
        return None;
    }

    let track_y = padding as f32;
    let track_h = rows as f32 * cell_h as f32;
    let total = (history + rows) as f32;
    let thumb_h = (track_h * rows as f32 / total).clamp(MIN_THUMB_H.min(track_h), track_h);
    let scroll = offset.min(history) as f32 / history as f32;
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

    #[test]
    fn thumb_sits_at_bottom_at_live_bottom() {
        let geom = geometry(1000, 4, 20, 10, 20, 0, false).expect("scrollbar at the live bottom");
        assert_eq!(geom.thumb_y + geom.thumb_h, geom.track_y + geom.track_h);
    }

    #[test]
    fn thumb_sits_at_bottom_when_scrolled_one_row_above_live() {
        let geom = geometry(1000, 4, 20, 10, 20, 1, false).expect("scrollbar");
        assert!(geom.thumb_y > geom.track_y);
    }

    #[test]
    fn thumb_sits_at_top_when_scrolled_to_history() {
        let geom = geometry(1000, 4, 20, 10, 100, 100, false).expect("scrollbar");
        assert_eq!(geom.thumb_y, geom.track_y);
    }

    #[test]
    fn offset_round_trips_for_top_middle_and_bottom() {
        let geom = geometry(1000, 4, 20, 10, 100, 50, false).expect("scrollbar");
        assert_eq!(offset_for_thumb_top(geom.track_y, &geom, 100), 100);
        let middle = geom.track_y + (geom.track_h - geom.thumb_h) / 2.0;
        assert_eq!(offset_for_thumb_top(middle, &geom, 100), 50);
        let bottom = geom.track_y + geom.track_h - geom.thumb_h;
        assert_eq!(offset_for_thumb_top(bottom, &geom, 100), 0);
    }

    #[test]
    fn no_scrollbar_without_history_or_space() {
        assert!(geometry(1000, 4, 20, 10, 0, 0, false).is_none());
        assert!(geometry(4, 4, 20, 10, 20, 0, false).is_none());
    }

    #[test]
    fn thumb_has_a_minimum_height() {
        let geom = geometry(1000, 4, 20, 10, 1000, 500, false).expect("scrollbar");
        assert_eq!(geom.thumb_h, 12.0);
    }

    #[test]
    fn hovered_thumb_is_wider_and_keeps_its_right_edge() {
        let rest = geometry(1000, 4, 20, 10, 100, 50, false).expect("scrollbar");
        let hovered = geometry(1000, 4, 20, 10, 100, 50, true).expect("scrollbar");
        assert!(hovered.width > rest.width);
        assert_eq!(hovered.x + hovered.width, rest.x + rest.width);
        assert_eq!(hovered.thumb_y, rest.thumb_y);
        assert_eq!(hovered.thumb_h, rest.thumb_h);
    }
}
