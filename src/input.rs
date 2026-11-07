//! Keyboard and mouse input translation.

use alacritty_terminal::term::TermMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Insert,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
}

/// Encode a logical key press as bytes for the PTY.
///
/// Shift is already reflected in `KeyInput::Char` by the windowing layer.
pub fn encode(key: KeyInput, mods: Mods, mode: TermMode) -> Option<Vec<u8>> {
    let bytes = match key {
        KeyInput::Char(ch) if mods.ctrl => {
            let lower = ch.to_ascii_lowercase();
            if !lower.is_ascii_lowercase() {
                return None;
            }
            vec![(lower as u8) - b'a' + 1]
        }
        KeyInput::Char(ch) => {
            let mut buf = [0u8; 4];
            ch.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        KeyInput::Enter => b"\r".to_vec(),
        KeyInput::Backspace => vec![0x7f],
        KeyInput::Tab => b"\t".to_vec(),
        KeyInput::Escape => vec![0x1b],
        KeyInput::Up => cursor_key(b'A', mode),
        KeyInput::Down => cursor_key(b'B', mode),
        KeyInput::Right => cursor_key(b'C', mode),
        KeyInput::Left => cursor_key(b'D', mode),
        KeyInput::Home => b"\x1b[H".to_vec(),
        KeyInput::End => b"\x1b[F".to_vec(),
        KeyInput::Insert => b"\x1b[2~".to_vec(),
        KeyInput::Delete => b"\x1b[3~".to_vec(),
        KeyInput::PageUp => b"\x1b[5~".to_vec(),
        KeyInput::PageDown => b"\x1b[6~".to_vec(),
    };

    if mods.alt {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend_from_slice(&bytes);
        Some(prefixed)
    } else {
        Some(bytes)
    }
}

fn cursor_key(final_byte: u8, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// Encode pasted text, optionally using xterm bracketed paste.
#[allow(dead_code)] // Wired into the clipboard path in Task 10.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }

    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Mods {
        Mods {
            ctrl: false,
            alt: false,
        }
    }

    #[test]
    fn named_keys_use_standard_bytes() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Enter, plain(), mode), Some(b"\r".to_vec()));
        assert_eq!(encode(KeyInput::Backspace, plain(), mode), Some(vec![0x7f]));
        assert_eq!(encode(KeyInput::Tab, plain(), mode), Some(b"\t".to_vec()));
        assert_eq!(encode(KeyInput::Escape, plain(), mode), Some(vec![0x1b]));
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let normal = TermMode::empty();
        let app = TermMode::APP_CURSOR;
        assert_eq!(
            encode(KeyInput::Up, plain(), normal),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(encode(KeyInput::Up, plain(), app), Some(b"\x1bOA".to_vec()));
        assert_eq!(
            encode(KeyInput::Left, plain(), normal),
            Some(b"\x1b[D".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Left, plain(), app),
            Some(b"\x1bOD".to_vec())
        );
    }

    #[test]
    fn navigation_keys_use_csi_sequences() {
        let mode = TermMode::empty();
        assert_eq!(
            encode(KeyInput::Home, plain(), mode),
            Some(b"\x1b[H".to_vec())
        );
        assert_eq!(
            encode(KeyInput::End, plain(), mode),
            Some(b"\x1b[F".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Insert, plain(), mode),
            Some(b"\x1b[2~".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Delete, plain(), mode),
            Some(b"\x1b[3~".to_vec())
        );
        assert_eq!(
            encode(KeyInput::PageUp, plain(), mode),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            encode(KeyInput::PageDown, plain(), mode),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn ctrl_letter_becomes_a_control_byte() {
        let mode = TermMode::empty();
        let mods = Mods {
            ctrl: true,
            alt: false,
        };
        assert_eq!(encode(KeyInput::Char('c'), mods, mode), Some(vec![3]));
        assert_eq!(encode(KeyInput::Char('A'), mods, mode), Some(vec![1]));
        assert_eq!(encode(KeyInput::Char('1'), mods, mode), None);
    }

    #[test]
    fn alt_prefixes_escape() {
        let mode = TermMode::empty();
        let mods = Mods {
            ctrl: false,
            alt: true,
        };
        assert_eq!(
            encode(KeyInput::Char('x'), mods, mode),
            Some(vec![0x1b, b'x'])
        );
    }

    #[test]
    fn text_is_encoded_as_utf8() {
        let mode = TermMode::empty();
        assert_eq!(
            encode(KeyInput::Char('é'), plain(), mode),
            Some(vec![0xc3, 0xa9])
        );
        assert_eq!(
            encode(KeyInput::Char(' '), plain(), mode),
            Some(b" ".to_vec())
        );
    }

    #[test]
    fn paste_wraps_only_when_bracketed() {
        assert_eq!(paste("hi", false), b"hi".to_vec());
        assert_eq!(paste("hi", true), b"\x1b[200~hi\x1b[201~".to_vec());
    }
}
