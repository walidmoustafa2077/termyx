//! User events bridged from the windowing and PTY layers.

/// Events delivered from background threads to the UI event loop.
#[derive(Debug)]
pub enum UserEvent {
    /// Bytes read from the PTY. They are forwarded to the VT parser verbatim.
    PtyOutput(Vec<u8>),
}
