//! Tabs over terminal sessions.
//!
//! [`TabSlots`] is the bookkeeping every tab operation needs: which slot is
//! active, which id it holds, and the title it shows. It is split from
//! [`TabSet`] so the rules can be tested without spawning shells.

use std::ops::{Index, IndexMut};

use crate::config::ShellConfig;
use crate::event::UserEvent;
use crate::pty::PtySession;
use crate::term::Terminal;

/// Identifies a session across the event loop; never reused within a run.
pub type SessionId = u64;

/// Title shown when a session has not set one, or cleared it.
pub const FALLBACK_TITLE: &str = "Termyx";

/// The form a title is stored in: trimmed, with blank titles cleared.
fn clean_title(title: &str) -> Option<String> {
    let title = title.trim();
    (!title.is_empty()).then(|| title.to_string())
}

#[derive(Debug, Default)]
pub struct TabSlots {
    ids: Vec<SessionId>,
    titles: Vec<Option<String>>,
    active: Option<usize>,
}

impl TabSlots {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, id: SessionId) {
        self.ids.push(id);
        self.titles.push(None);
        self.active = Some(self.ids.len() - 1);
    }

    // Counting tabs is the keyboard shortcuts' need (Task 10), so it compiles
    // for the tests only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn select(&mut self, index: usize) {
        if index < self.ids.len() {
            self.active = Some(index);
        }
    }

    /// Removes the active slot and activates the neighbour that takes its
    /// place: the one to the right, else the one to the left.
    pub fn remove_active(&mut self) -> Option<SessionId> {
        let index = self.active?;
        let id = self.ids.remove(index);
        self.titles.remove(index);
        self.active = if self.ids.is_empty() {
            None
        } else {
            Some(index.min(self.ids.len() - 1))
        };
        Some(id)
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<SessionId> {
        self.ids.clone()
    }

    /// Sets the title the slot with `id` shows, clearing it when `title` is
    /// blank. Returns whether the stored label actually changed, so a repeat
    /// of the same title costs no repaint.
    pub fn set_title(&mut self, id: SessionId, title: &str) -> bool {
        let Some(index) = self.ids.iter().position(|slot| *slot == id) else {
            return false;
        };
        let next = clean_title(title);
        let changed = self.titles[index] != next;
        self.titles[index] = next;
        changed
    }

    /// The labels of every slot, with the fallback for untitled ones.
    pub fn titles(&self) -> Vec<&str> {
        self.titles
            .iter()
            .map(|title| title.as_deref().unwrap_or(FALLBACK_TITLE))
            .collect()
    }
}

/// One shell session: its parser and its child.
///
/// Its title is not stored here: [`TabSlots`] is the single store, and the tab
/// strip reads it through [`TabSet::titles`].
pub struct Session {
    pub id: SessionId,
    pub terminal: Terminal,
    pub pty: PtySession,
}

impl Session {
    pub fn new(id: SessionId, terminal: Terminal, pty: PtySession) -> Self {
        Self { id, terminal, pty }
    }
}

/// The live sessions plus the title bookkeeping from [`TabSlots`].
pub struct TabSet {
    slots: TabSlots,
    sessions: Vec<Session>,
    next_id: SessionId,
}

impl Default for TabSet {
    fn default() -> Self {
        Self::new()
    }
}

impl TabSet {
    pub fn new() -> Self {
        Self {
            slots: TabSlots::new(),
            sessions: Vec::new(),
            next_id: 1,
        }
    }

    pub fn next_id(&mut self) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn push(&mut self, session: Session) -> usize {
        self.slots.push(session.id);
        self.sessions.push(session);
        self.sessions.len() - 1
    }

    // Counting tabs is the keyboard shortcuts' need (Task 10), so it compiles
    // for the tests only.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn active_index(&self) -> Option<usize> {
        self.slots.active_index()
    }

    pub fn select(&mut self, index: usize) {
        self.slots.select(index);
    }

    pub fn active(&self) -> Option<&Session> {
        self.slots.active_index().map(|index| &self.sessions[index])
    }

    pub fn active_mut(&mut self) -> Option<&mut Session> {
        let index = self.slots.active_index()?;
        Some(&mut self.sessions[index])
    }

    pub fn remove_active(&mut self) -> Option<Session> {
        let index = self.slots.active_index()?;
        let id = self.slots.remove_active()?;
        let session = self.sessions.remove(index);
        debug_assert_eq!(session.id, id);
        Some(session)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
    }

    pub fn titles(&self) -> Vec<&str> {
        self.slots.titles()
    }

    pub fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    pub fn set_title(&mut self, id: SessionId, title: &str) -> bool {
        self.slots.set_title(id, title)
    }
}

impl Index<usize> for TabSet {
    type Output = Session;

    fn index(&self, index: usize) -> &Session {
        &self.sessions[index]
    }
}

impl IndexMut<usize> for TabSet {
    fn index_mut(&mut self, index: usize) -> &mut Session {
        &mut self.sessions[index]
    }
}

/// Spawns the shell for a new session and builds its parser.
///
/// The id tags both ends of the session: the parser's event proxy and the PTY
/// reader thread.
pub fn spawn_session(
    id: SessionId,
    shell: &ShellConfig,
    cols: usize,
    rows: usize,
    scrollback: usize,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
) -> anyhow::Result<Session> {
    let terminal = Terminal::new_with_events(cols, rows, scrollback, Some((proxy.clone(), id)));
    let pty = PtySession::spawn(shell, cols as u16, rows as u16, proxy, id)?;
    Ok(Session::new(id, terminal, pty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_active_slot_selects_a_neighbour() {
        let mut slots = TabSlots::new();
        slots.push(1);
        slots.push(2);
        slots.push(3);
        assert_eq!(slots.len(), 3);
        assert_eq!(slots.active_index(), Some(2));

        slots.select(1);
        assert_eq!(slots.remove_active(), Some(2));
        assert_eq!(slots.active_index(), Some(1));

        slots.select(0);
        assert_eq!(slots.remove_active(), Some(1));
        assert_eq!(slots.active_index(), Some(0));
        assert_eq!(slots.remove_active(), Some(3));
        assert_eq!(slots.active_index(), None);
        assert!(slots.is_empty());
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        let mut slots = TabSlots::new();
        slots.push(7);
        slots.select(5);
        assert_eq!(slots.active_index(), Some(0));
    }

    #[test]
    fn titles_fall_back_to_the_window_name() {
        let mut slots = TabSlots::new();
        slots.push(1);
        assert_eq!(slots.titles(), vec!["Termyx"]);
        slots.set_title(1, "build");
        assert_eq!(slots.titles(), vec!["build"]);
        slots.set_title(1, "  ");
        assert_eq!(slots.titles(), vec!["Termyx"]);
    }

    #[test]
    fn a_new_slot_becomes_active_and_ids_never_repeat() {
        let mut slots = TabSlots::new();
        slots.push(1);
        slots.push(2);
        assert_eq!(slots.active_index(), Some(1));
        slots.select(0);
        slots.push(3);
        assert_eq!(slots.active_index(), Some(2));
        assert_eq!(slots.ids(), vec![1, 2, 3]);
    }

    /// An inert shell that stays open until the session is dropped, so the
    /// tab tests never depend on a command finishing.
    fn idle_shell() -> ShellConfig {
        #[cfg(windows)]
        let program = "cmd.exe";
        #[cfg(not(windows))]
        let program = "/bin/sh";
        ShellConfig {
            program: program.into(),
            args: Vec::new(),
        }
    }

    fn test_session(id: SessionId) -> Session {
        let pty =
            PtySession::spawn_with(&idle_shell(), 80, 24, |_| {}).expect("spawn the test shell");
        Session::new(id, Terminal::new(80, 24, 100), pty)
    }

    #[test]
    fn an_empty_tab_set_has_no_active_session() {
        let mut set = TabSet::default();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
        assert_eq!(set.active_index(), None);
        assert!(set.active().is_none());
        assert!(set.active_mut().is_none());
        assert_eq!(set.iter_mut().count(), 0);
        assert_eq!(set.titles(), Vec::<&str>::new());
        assert!(set.session_mut(1).is_none());
        set.set_title(1, "build");
        set.select(0);
        assert!(set.remove_active().is_none());
        assert_eq!(set.next_id(), 1);
    }

    #[test]
    fn a_pushed_session_becomes_active_and_keeps_its_id() {
        let mut set = TabSet::new();
        let first = set.next_id();
        let second = set.next_id();
        assert_eq!(set.push(test_session(first)), 0);
        assert_eq!(set.push(test_session(second)), 1);

        assert_eq!(set.len(), 2);
        assert_eq!(set.active_index(), Some(1));
        assert_eq!(set.active().map(|session| session.id), Some(second));
        assert_eq!(set[1].id, second);
        let first_session = &mut set[0];
        assert_eq!(first_session.id, first);
        assert_eq!(
            set.iter_mut().map(|session| session.id).collect::<Vec<_>>(),
            vec![first, second]
        );

        set.set_title(second, "build");
        assert_eq!(set.titles(), vec!["Termyx", "build"]);
        assert_eq!(set.active_mut().map(|session| session.id), Some(second));

        set.select(0);
        let removed = set.remove_active().expect("an active session");
        assert_eq!(removed.id, first);
        assert_eq!(set.titles(), vec!["build"]);
        assert_eq!(set.active_index(), Some(0));
        assert_eq!(
            set.session_mut(second).map(|session| session.id),
            Some(second)
        );
    }

    #[test]
    fn a_session_owns_a_live_parser_and_child() {
        let mut session = test_session(1);
        assert!(!session.terminal.feed(b""));
        session.pty.resize(80, 24).expect("resize the child");
    }

    #[test]
    fn spawn_session_has_the_signature_the_app_calls() {
        // Pins the public signature until the app has a call site to break.
        let _ = spawn_session
            as fn(
                SessionId,
                &ShellConfig,
                usize,
                usize,
                usize,
                winit::event_loop::EventLoopProxy<UserEvent>,
            ) -> anyhow::Result<Session>;
    }
}
