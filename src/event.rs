//! User events bridged from the windowing and PTY layers.

/// Events delivered from background threads to the UI event loop.
#[derive(Debug)]
pub enum UserEvent {
    /// Bytes read from the PTY. They are forwarded to the VT parser verbatim.
    PtyOutput(Vec<u8>),
    /// The child requested a new window title (`OSC 0` / `OSC 2`). An empty
    /// title restores the default title.
    Title(String),
    /// The title stack was popped, or the terminal resized while no title was
    /// set. Not an OSC 104/110 palette reset.
    ResetTitle,
    /// The child stored text on the system clipboard (`OSC 52`).
    ClipboardStore(String),
}
