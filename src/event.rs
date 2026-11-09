//! User events bridged from the windowing and PTY layers.

/// Messages from a session's reader thread and parser to the UI thread.
///
/// Every session-tagged message carries its [`session`](UserEvent::PtyOutput)
/// id, because the UI routes it to that tab instead of a single terminal.
#[derive(Debug)]
pub enum UserEvent {
    /// Bytes read from the pty, fed verbatim to the parser.
    PtyOutput { session: u64, bytes: Vec<u8> },
    /// OSC 0/2 set the window title.
    Title { session: u64, title: String },
    /// OSC 0/2 with an empty argument restores the default title.
    ResetTitle { session: u64 },
    /// OSC 52 asked to put text on the system clipboard.
    ClipboardStore(String),
}
