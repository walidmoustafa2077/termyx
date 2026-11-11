//! Inline tab rename: the transient editor that sits over a tab while the
//! user types a new title.

use crate::session::SessionId;

/// The in-progress rename of the tab `id`: the text typed so far, seeded with
/// the tab's current title so Enter without edits keeps the name.
pub struct RenameState {
    pub id: SessionId,
    pub text: String,
}

impl RenameState {
    pub fn new(id: SessionId, current: &str) -> Self {
        Self {
            id,
            text: current.to_string(),
        }
    }

    /// Appends `ch` unless it is a control character, so a stray `\n` or `\t`
    /// never lands in a title.
    pub fn push(&mut self, ch: char) {
        if !ch.is_control() {
            self.text.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        self.text.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::RenameState;

    #[test]
    fn typing_appends_and_backspace_removes() {
        let mut state = RenameState::new(7, "Build");
        state.push(' ');
        state.push('x');
        state.backspace();
        assert_eq!(state.text, "Build ");
    }

    #[test]
    fn control_characters_are_ignored() {
        let mut state = RenameState::new(7, "");
        state.push('\n');
        state.push('\t');
        state.push('a');
        assert_eq!(state.text, "a");
    }
}
