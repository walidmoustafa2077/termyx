//! User events bridged from the windowing and PTY layers.

/// Messages from a session's reader thread and parser to the UI thread.
///
/// Every session-tagged message carries its [`session`](UserEvent::PtyOutput)
/// id, because the UI routes it to that tab instead of a single terminal.
#[derive(Debug)]
pub enum UserEvent {
    /// Bytes read from the pty, fed verbatim to the parser.
    PtyOutput { session: u64, bytes: Vec<u8> },
    /// The session's reader thread ended: the shell exited or the pty closed,
    /// so the tab has nothing left to show.
    Exited { session: u64 },
    /// OSC 0/2 set the window title.
    Title { session: u64, title: String },
    /// A title-stack pop, or a term options reset, left the terminal with no
    /// title of its own, so the window shows the default. An empty OSC 0/2
    /// arrives as `Title("")` instead, and this is not an OSC 104/110 palette
    /// reset.
    ResetTitle { session: u64 },
    /// OSC 52 asked to put text on the system clipboard.
    ClipboardStore(String),
    /// A background update check finished: `Ok(Some(tag))` when a newer
    /// release exists, `Ok(None)` when the running build is current.
    UpdateChecked(Result<Option<String>, String>),
    /// The update package was staged and the installer is armed; `Ok` means
    /// the app can exit and let it run.
    UpdateStaged(Result<(), String>),
}
