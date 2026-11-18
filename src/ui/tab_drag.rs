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
    /// Set once the pointer has left `origin` by more than the threshold.
    pub active: bool,
}

impl TabDrag {
    pub fn new(session: SessionId, origin: (f64, f64)) -> Self {
        Self {
            session,
            origin,
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

#[cfg(test)]
mod tests {
    use super::TabDrag;

    #[test]
    fn a_press_that_stays_put_is_still_a_click() {
        let mut drag = TabDrag::new(1, (100.0, 20.0));
        assert!(!drag.step((102.0, 22.0), 4.0));
        assert!(!drag.active);
    }

    #[test]
    fn crossing_the_threshold_starts_a_drag_that_sticks() {
        let mut drag = TabDrag::new(1, (100.0, 20.0));
        assert!(drag.step((100.0, 30.0), 4.0));
        assert!(drag.active);
        // Wandering back over the origin does not undo the drag.
        assert!(drag.step((100.0, 20.0), 4.0));
        assert!(drag.active);
    }
}
