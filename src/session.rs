//! Tabs over terminal sessions.
//!
//! [`TabSlots`] is the bookkeeping every tab operation needs: which slot is
//! active and which id it holds. It is split from [`TabSet`] so the rules can
//! be tested without spawning shells.

use std::ops::{Index, IndexMut};

use crate::config::ShellConfig;
use crate::event::UserEvent;
use crate::pty::PtySession;
use crate::term::Terminal;

/// Identifies a session across the event loop; never reused within a run.
pub type SessionId = u64;

/// Title shown when a session has not set one, or cleared it.
pub const FALLBACK_TITLE: &str = "Termyx";

/// The form an OSC title is stored in: trimmed, with blank and path-like
/// titles cleared.
fn clean_title(title: &str) -> Option<String> {
    let title = title.trim();
    title_is_usable(title).then(|| title.to_string())
}

/// Whether an OSC 0/2 title can label a tab.
///
/// ConPTY gives the child no title of its own, so a shell that never sets one
/// leaves the executable path in the title. Windows Terminal hides that path
/// behind the profile name; we drop the title and keep the profile name.
pub fn title_is_usable(title: &str) -> bool {
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    // `C:\...`, `\\?\C:\...` and `\\server\share\...` are program paths, not names.
    let path_like = title.starts_with(r"\\")
        || (title.len() >= 3
            && title.as_bytes()[1] == b':'
            && (title.as_bytes()[2] == b'\\' || title.as_bytes()[2] == b'/'));
    !path_like
}

#[derive(Debug, Default)]
pub struct TabSlots {
    ids: Vec<SessionId>,
    active: Option<usize>,
}

impl TabSlots {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, id: SessionId) {
        self.ids.push(id);
        self.active = Some(self.ids.len() - 1);
    }

    // Only the tests count slots; production counts through `TabSet::len`.
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

    /// Removes the slot at `index`. The active slot stays active unless it was
    /// the one removed, in which case the neighbour that takes its place
    /// becomes active.
    pub fn remove(&mut self, index: usize) -> Option<SessionId> {
        if index >= self.ids.len() {
            return None;
        }
        let id = self.ids.remove(index);
        match self.active {
            Some(active) if active == index => {
                self.active = if self.ids.is_empty() {
                    None
                } else {
                    Some(index.min(self.ids.len() - 1))
                };
            }
            Some(active) if active > index => self.active = Some(active - 1),
            _ => {}
        }
        Some(id)
    }

    /// Removes the active slot. Only the tests close without an index;
    /// production closes by index so a background tab cannot steal the view.
    #[cfg(test)]
    pub fn remove_active(&mut self) -> Option<SessionId> {
        let index = self.active?;
        self.remove(index)
    }

    #[cfg(test)]
    pub fn ids(&self) -> Vec<SessionId> {
        self.ids.clone()
    }
}

/// One tab: its parser and, for shell tabs, its child.
///
/// Its title lives here so a rename and the shell's own OSC title update the
/// same store the strip reads through [`TabSet::titles`]. Its icon is here for
/// the same reason: the program it started is a property of this session, not
/// of the strip.
///
/// In-app pages (Settings, palette, About) use the same grid so rendering,
/// scrolling, and selection stay one path, but they have no child process and
/// therefore no `pty`.
pub struct Session {
    pub id: SessionId,
    pub terminal: Terminal,
    pub pty: Option<PtySession>,
    /// The program's icon, once the app has looked one up.
    pub icon: Option<crate::icon::TabIcon>,
    /// Set when this tab is an in-app page rather than a shell.
    pub page: Option<crate::page::Page>,
    /// The label the strip shows, or `None` for [`FALLBACK_TITLE`].
    pub title: Option<String>,
    /// A user rename wins over later OSC titles until the rename is cleared.
    pub title_locked: bool,
    /// The tab color chosen from the context menu.
    pub color: Option<crate::theme::Rgb>,
    /// Index into `profiles(&config)` for a shell tab; `None` for a page tab.
    pub profile: Option<usize>,
}

impl Session {
    pub fn new(id: SessionId, terminal: Terminal, pty: PtySession) -> Self {
        // A session does not know which profile started it, so the app fills
        // the icon and profile in once it has resolved the program.
        Self {
            id,
            terminal,
            pty: Some(pty),
            icon: None,
            page: None,
            title: None,
            title_locked: false,
            color: None,
            profile: None,
        }
    }

    /// A page tab: a grid with generated content and no child to write to.
    pub fn page_tab(id: SessionId, terminal: Terminal, page: crate::page::Page) -> Self {
        Self {
            id,
            terminal,
            pty: None,
            icon: None,
            page: Some(page),
            title: None,
            title_locked: false,
            color: None,
            profile: None,
        }
    }
}

/// The live sessions plus the slot bookkeeping.
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

    // Counting tabs is what lets the keyboard shortcuts wrap; the mouse only
    // ever needs `is_empty`.
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

    /// Removes the session at `index`. The tab on screen stays on screen unless
    /// it was the one removed, so closing a background tab never steals the
    /// view.
    pub fn remove(&mut self, index: usize) -> Option<Session> {
        let id = self.slots.remove(index)?;
        let session = self.sessions.remove(index);
        debug_assert_eq!(session.id, id);
        Some(session)
    }

    // The tests close whatever happens to be active; production always names
    // the index it closes.
    #[cfg(test)]
    pub fn remove_active(&mut self) -> Option<Session> {
        let index = self.slots.active_index()?;
        self.remove(index)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
    }

    pub fn titles(&self) -> Vec<&str> {
        self.sessions
            .iter()
            .map(|session| session.title.as_deref().unwrap_or(FALLBACK_TITLE))
            .collect()
    }

    pub fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    /// The session `id` names, if it is still open.
    // Wired to the context-menu color row in a later task; the tests pin the
    // lookup now.
    #[allow(dead_code)]
    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.iter().find(|session| session.id == id)
    }

    /// The slot index of the session `id` carries, if it is still open.
    pub fn position(&self, id: SessionId) -> Option<usize> {
        self.sessions.iter().position(|session| session.id == id)
    }

    /// Moves the tab at `from` so it sits at `to`, keeping the same session
    /// active.
    ///
    /// The session and its slot id move together because [`titles`] and the
    /// strip layout read the two vectors in parallel; reordering one alone
    /// would label every tab in between with the neighbour's title. Returns
    /// whether an index actually changed.
    ///
    /// [`titles`]: TabSet::titles
    // The context menu's move commands call this in a later task.
    #[allow(dead_code)]
    pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
        let len = self.sessions.len();
        if from >= len || to >= len || from == to {
            return false;
        }
        let active_id = self
            .slots
            .active_index()
            .map(|index| self.sessions[index].id);
        let session = self.sessions.remove(from);
        let id = self.slots.ids.remove(from);
        self.sessions.insert(to, session);
        self.slots.ids.insert(to, id);
        if let Some(active_id) = active_id {
            self.slots.active = self.slots.ids.iter().position(|id| *id == active_id);
        }
        true
    }

    /// Sets the title the session `id` shows, clearing it when `title` is
    /// blank. Returns whether the stored label actually changed, so a repeat
    /// of the same title costs no repaint.
    ///
    /// A title that is really a program path is ignored: ConPTY leaves the
    /// executable path behind when the shell sets no title of its own. A user
    /// rename locks the title, and later OSC updates leave it alone.
    pub fn set_title(&mut self, id: SessionId, title: &str) -> bool {
        let Some(session) = self.session_mut(id) else {
            return false;
        };
        if session.title_locked {
            return false;
        }
        let next = clean_title(title);
        // A path is not a name, but a blank title must still clear the label so
        // the fallback can come back.
        if next.is_none() && !title.trim().is_empty() {
            return false;
        }
        let changed = session.title != next;
        session.title = next;
        changed
    }

    /// Renames the session `id` and locks the title against OSC updates.
    /// A blank name clears the title and the lock, so the shell can name the
    /// tab again.
    // The rename UI that calls this lands in a later task; the tests pin the
    // behavior now.
    #[allow(dead_code)]
    pub fn rename(&mut self, id: SessionId, title: &str) -> bool {
        let Some(session) = self.session_mut(id) else {
            return false;
        };
        let trimmed = title.trim();
        if trimmed.is_empty() {
            let changed = session.title.is_some() || session.title_locked;
            session.title = None;
            session.title_locked = false;
            return changed;
        }
        // An explicit rename is the user's choice, so it bypasses the path-like
        // filter that protects OSC titles from becoming `C:\...`.
        let next = Some(trimmed.to_string());
        let changed = session.title != next || !session.title_locked;
        session.title = next;
        session.title_locked = true;
        changed
    }

    /// The tab color `id` carries, if any.
    // Wired to the context-menu swatches in a later task; the tests pin the
    // store now.
    #[allow(dead_code)]
    pub fn color(&self, id: SessionId) -> Option<crate::theme::Rgb> {
        self.session(id).and_then(|session| session.color)
    }

    /// Sets the tab color `id` carries, returning whether it actually changed
    /// so a repeat choice costs no repaint.
    // Wired to the context-menu swatches in a later task.
    #[allow(dead_code)]
    pub fn set_color(&mut self, id: SessionId, color: Option<crate::theme::Rgb>) -> bool {
        let Some(session) = self.session_mut(id) else {
            return false;
        };
        if session.color == color {
            return false;
        }
        session.color = color;
        true
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
    fn closing_a_background_slot_keeps_the_active_one() {
        let mut slots = TabSlots::new();
        slots.push(1);
        slots.push(2);
        slots.push(3);
        slots.select(0);

        // Closing a tab the window is not showing leaves the view alone.
        assert_eq!(slots.remove(1), Some(2));
        assert_eq!(slots.active_index(), Some(0));
        assert_eq!(slots.ids(), vec![1, 3]);

        // Removing a slot before the active one shifts it down with its id.
        slots.select(1);
        assert_eq!(slots.remove(0), Some(1));
        assert_eq!(slots.active_index(), Some(0));
        assert_eq!(slots.ids(), vec![3]);

        // An index no slot has removes nothing.
        assert_eq!(slots.remove(9), None);
        assert_eq!(slots.ids(), vec![3]);
    }

    #[test]
    fn closing_the_active_slot_activates_a_neighbour() {
        let mut slots = TabSlots::new();
        slots.push(1);
        slots.push(2);
        slots.push(3);
        slots.select(1);

        assert_eq!(slots.remove(1), Some(2));
        assert_eq!(slots.active_index(), Some(1), "the slot to the right");
        assert_eq!(slots.ids(), vec![1, 3]);

        // The last slot falls back to the one on its left, and the empty set
        // has no active slot at all.
        assert_eq!(slots.remove(1), Some(3));
        assert_eq!(slots.active_index(), Some(0));
        assert_eq!(slots.remove(0), Some(1));
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
        let mut tabs = TabSet::new();
        let id = tabs.next_id();
        tabs.push(test_session(id));
        assert_eq!(tabs.titles(), vec!["Termyx"]);
        tabs.set_title(id, "build");
        assert_eq!(tabs.titles(), vec!["build"]);
        tabs.set_title(id, "  ");
        assert_eq!(tabs.titles(), vec!["Termyx"]);
    }

    #[test]
    fn a_conpty_executable_path_is_not_a_tab_title() {
        assert!(!title_is_usable(r"C:\WINDOWS\System32\WindX"));
        assert!(!title_is_usable(r"\\?\C:\Windows\System32\cmd.exe"));
        assert!(!title_is_usable("   "));
        assert!(!title_is_usable(""));
        assert!(title_is_usable("Windows PowerShell"));
        assert!(title_is_usable("OC | Next steps"));
        assert!(title_is_usable("nvim src/app.rs"));
    }

    #[test]
    fn a_shell_path_does_not_replace_the_stored_title() {
        let mut tabs = TabSet::new();
        let id = tabs.next_id();
        tabs.push(test_session(id));
        tabs.set_title(id, "Windows PowerShell");
        assert!(!tabs.set_title(id, r"C:\WINDOWS\System32\WindX"));
        assert_eq!(
            tabs.titles(),
            vec!["Windows PowerShell"],
            "a ConPTY path must leave the profile name in place"
        );
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
        let pty = PtySession::spawn_with(&idle_shell(), 80, 24, |_| {}, || {})
            .expect("spawn the test shell");
        Session::new(id, Terminal::new(80, 24, 100), pty)
    }

    #[test]
    fn rename_locks_the_title_against_osc_updates() {
        let mut tabs = TabSet::new();
        let id = tabs.next_id();
        tabs.push(test_session(id));
        assert!(tabs.rename(id, "Build"));
        assert_eq!(tabs.titles(), vec!["Build"]);
        assert!(!tabs.set_title(id, "shell title"));
        assert_eq!(tabs.titles(), vec!["Build"]);
    }

    #[test]
    fn a_blank_rename_clears_the_lock() {
        let mut tabs = TabSet::new();
        let id = tabs.next_id();
        tabs.push(test_session(id));
        assert!(tabs.rename(id, "Build"));
        assert!(tabs.rename(id, "   "));
        assert_eq!(tabs.titles(), vec![FALLBACK_TITLE]);
        assert!(tabs.set_title(id, "shell title"));
        assert_eq!(tabs.titles(), vec!["shell title"]);
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
    fn a_session_is_found_by_id_until_it_is_removed() {
        let mut set = TabSet::new();
        let first = set.next_id();
        let second = set.next_id();
        set.push(test_session(first));
        set.push(test_session(second));

        assert_eq!(set.position(first), Some(0));
        assert_eq!(set.position(second), Some(1));
        set.remove(0);
        assert_eq!(
            set.position(first),
            None,
            "a closed session must not be found again"
        );
        assert_eq!(
            set.position(second),
            Some(0),
            "the index follows the removal"
        );
    }

    #[test]
    fn move_tab_is_a_noop_at_the_edges() {
        let mut tabs = TabSet::new();
        let a = tabs.next_id();
        tabs.push(test_session(a));
        let b = tabs.next_id();
        tabs.push(test_session(b));
        assert!(!tabs.move_tab(0, 0));
        assert!(!tabs.move_tab(0, 2));
        assert_eq!(tabs[0].id, a);
        assert_eq!(tabs[1].id, b);
    }

    #[test]
    fn move_tab_keeps_the_active_session_active() {
        let mut tabs = TabSet::new();
        let a = tabs.next_id();
        tabs.push(test_session(a));
        let b = tabs.next_id();
        tabs.push(test_session(b));
        let c = tabs.next_id();
        tabs.push(test_session(c));
        tabs.select(0);
        assert!(tabs.move_tab(0, 2));
        assert_eq!(tabs.titles().len(), 3);
        assert_eq!(tabs.active().map(|session| session.id), Some(a));
        assert_eq!(tabs[0].id, b);
        assert_eq!(tabs[1].id, c);
        assert_eq!(tabs[2].id, a);

        // A remove/insert is a true move, not a swap: moving the first tab to
        // the middle shifts the tab after it left and keeps the rest in order.
        assert!(tabs.move_tab(0, 1));
        assert_eq!(
            tabs.iter_mut()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            vec![c, b, a],
            "the id vector must move with the sessions vector"
        );
        assert_eq!(tabs.active().map(|session| session.id), Some(a));
    }

    #[test]
    fn color_is_stored_per_session() {
        let mut tabs = TabSet::new();
        let id = tabs.next_id();
        tabs.push(test_session(id));
        assert_eq!(tabs.color(id), None);
        assert!(tabs.set_color(id, Some(crate::theme::Rgb::new(1, 2, 3))));
        assert_eq!(tabs.color(id), Some(crate::theme::Rgb::new(1, 2, 3)));
        assert!(
            !tabs.set_color(id, Some(crate::theme::Rgb::new(1, 2, 3))),
            "the same color must not ask for a repaint"
        );
        assert!(tabs.set_color(id, None));
        assert_eq!(tabs.color(id), None);
        assert!(!tabs.set_color(id + 100, Some(crate::theme::Rgb::new(1, 2, 3))));
    }

    #[test]
    fn a_session_owns_a_live_parser_and_child() {
        let mut session = test_session(1);
        assert!(!session.terminal.feed(b""));
        assert!(
            session.icon.is_none(),
            "a session starts without an icon; only the app knows its program"
        );
        session
            .pty
            .as_mut()
            .expect("a shell session has a child")
            .resize(80, 24)
            .expect("resize the child");
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
