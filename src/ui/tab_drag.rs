//! A tab dragged out of the strip by the mouse.
//!
//! The press that grabs a tab selects it straight away, so a drag only ever
//! reorders the tabs the window already shows: a release before the pointer
//! leaves the threshold is still a plain click.

use crate::session::SessionId;

/// A left press that landed on a tab body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabDrag {
    /// The grabbed tab. Ids survive a reorder, so this keeps identifying the
    /// tab however often the strip is rebuilt under the pointer.
    pub session: SessionId,
    /// Where the press landed, in physical pixels. The threshold is measured
    /// from here, not from the last motion, so a slow start is still a click.
    pub origin: (f64, f64),
    /// How far into the tab body the press landed, in physical pixels.
    /// Carrying the card by this offset keeps it under the same point of the
    /// tab the user grabbed, in both axes.
    pub grab: (f64, f64),
    /// Set once the pointer has left `origin` by more than the threshold.
    pub active: bool,
}

impl TabDrag {
    pub fn new(session: SessionId, origin: (f64, f64), grab: (f64, f64)) -> Self {
        Self {
            session,
            origin,
            grab,
            active: false,
        }
    }

    /// Records the pointer and reports whether the gesture has become a drag.
    ///
    /// Once active it stays active: drifting back over the origin must not turn
    /// a drag into the click that moves the shell caret.
    pub fn step(&mut self, at: (f64, f64), threshold: f64) -> bool {
        if !self.active {
            let (dx, dy) = (at.0 - self.origin.0, at.1 - self.origin.1);
            self.active = dx.abs() > threshold || dy.abs() > threshold;
        }
        self.active
    }
}

/// One window a dragged tab could land in, in physical screen pixels.
#[derive(Debug, Clone)]
pub struct DropZone<Id> {
    /// The window this zone describes.
    pub id: Id,
    /// The client area's top-left corner on screen.
    pub origin: (i32, i32),
    /// The client area's size.
    pub size: (u32, u32),
    /// Painted height of the tab strip at the top of the client area.
    pub strip_h: f32,
    /// Tab midpoints down the strip, in screen pixels.
    pub centers: Vec<f32>,
}

impl<Id> DropZone<Id> {
    /// Whether `pointer` is inside this window's client area.
    pub fn holds(&self, pointer: (i32, i32)) -> bool {
        let (x, y) = pointer;
        let (left, top) = self.origin;
        x >= left && y >= top && x < left + self.size.0 as i32 && y < top + self.size.1 as i32
    }

    /// The slot a tab dropped here takes: the number of tab midpoints left of
    /// the pointer, which `TabSet` clamps like any other insertion index.
    pub fn drop_index(&self, x: i32) -> usize {
        self.centers
            .iter()
            .filter(|center| **center < x as f32)
            .count()
    }
}

/// What a released tab does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropTarget<Id> {
    /// Back into the strip the drag started from; the live preview has already
    /// put it where the pointer left it.
    Source,
    /// Another window's strip, at this slot.
    Window(Id, usize),
    /// A window where the pointer was released, because it is over no Termyx
    /// window at all.
    Detach,
    /// Over another window but off its strip, where a tab cannot land.
    Cancel,
}

/// Decides what a release at `pointer` means for a tab dragged out of `source`.
///
/// Only a strip accepts a tab: releasing over another window's grid must not
/// bury that window under a new one, so that case cancels. A release over no
/// window is a deliberate drop on the desktop and detaches.
pub fn resolve_drop<Id: Copy + PartialEq>(
    pointer: (i32, i32),
    source: Id,
    zones: &[DropZone<Id>],
) -> DropTarget<Id> {
    let Some(zone) = zones.iter().find(|zone| zone.holds(pointer)) else {
        return DropTarget::Detach;
    };
    if zone.id == source {
        return DropTarget::Source;
    }
    if (pointer.1 - zone.origin.1) as f32 >= zone.strip_h {
        return DropTarget::Cancel;
    }
    DropTarget::Window(zone.id, zone.drop_index(pointer.0))
}

/// The slot a dragged tab takes when `raw` tab midpoints lie left of the
/// pointer.
///
/// `raw` counts the dragged tab too, and that tab is about to leave the place
/// it currently occupies, so its own slot has to come back out: with the
/// pointer just past one neighbour `raw` is two, and the tab moves one place,
/// not two. The result is clamped to the last slot.
pub fn drop_slot(from: usize, raw: usize, len: usize) -> usize {
    raw.saturating_sub(usize::from(raw > from))
        .min(len.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::{drop_slot, resolve_drop, DropTarget, DropZone, TabDrag};

    fn zone(id: u64, x: i32, y: i32, w: u32, h: u32) -> DropZone<u64> {
        DropZone {
            id,
            origin: (x, y),
            size: (w, h),
            strip_h: 40.0,
            centers: vec![x as f32 + 50.0, x as f32 + 150.0],
        }
    }

    #[test]
    fn a_one_slot_drag_moves_one_slot() {
        // Dragging the first of three just past its neighbour counts two
        // midpoints; the tab belongs in the middle slot, not at the end.
        assert_eq!(drop_slot(0, 2, 3), 1);
        // Past both neighbours it belongs last, and the clamp holds it there.
        assert_eq!(drop_slot(0, 3, 3), 2);
        assert_eq!(drop_slot(0, 9, 3), 2);
        // Dragging left, only the tabs it passed are counted.
        assert_eq!(drop_slot(2, 0, 3), 0);
    }

    #[test]
    fn a_release_back_in_the_source_keeps_the_previewed_order() {
        let zones = [zone(1, 0, 0, 800, 600)];
        assert_eq!(
            resolve_drop((20, 20), 1, &zones),
            DropTarget::Source,
            "the live reorder already moved the tab"
        );
    }

    #[test]
    fn a_release_over_another_strip_names_its_slot() {
        let zones = [zone(1, 0, 0, 800, 600), zone(2, 1000, 0, 800, 600)];
        assert_eq!(
            resolve_drop((1000, 20), 1, &zones),
            DropTarget::Window(2, 0)
        );
        assert_eq!(
            resolve_drop((1101, 20), 1, &zones),
            DropTarget::Window(2, 1)
        );
        assert_eq!(
            resolve_drop((1200, 39), 1, &zones),
            DropTarget::Window(2, 2)
        );
    }

    #[test]
    fn a_release_over_another_grid_cancels() {
        let zones = [zone(1, 0, 0, 800, 600), zone(2, 1000, 0, 800, 600)];
        assert_eq!(resolve_drop((1100, 200), 1, &zones), DropTarget::Cancel);
    }

    #[test]
    fn a_release_outside_every_window_detaches() {
        let zones = [zone(1, 0, 0, 800, 600)];
        assert_eq!(resolve_drop((5000, 5000), 1, &zones), DropTarget::Detach);
    }

    #[test]
    fn a_press_that_stays_put_is_still_a_click() {
        let mut drag = TabDrag::new(1, (100.0, 20.0), (0.0, 0.0));
        assert!(!drag.step((102.0, 22.0), 4.0));
        assert!(!drag.active);
    }

    #[test]
    fn crossing_the_threshold_starts_a_drag_that_sticks() {
        let mut drag = TabDrag::new(1, (100.0, 20.0), (0.0, 0.0));
        assert!(drag.step((100.0, 30.0), 4.0));
        assert!(drag.active);
        // Wandering back over the origin does not undo the drag.
        assert!(drag.step((100.0, 20.0), 4.0));
        assert!(drag.active);
    }
}
