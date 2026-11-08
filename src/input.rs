//! Keyboard and mouse input translation.

use alacritty_terminal::term::TermMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    BackTab,
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
    Function(u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEventKind {
    Press,
    Release,
    Motion,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub kind: MouseEventKind,
    pub button: Option<MouseButton>,
    /// Zero-based visual cell.
    pub column: usize,
    /// Zero-based screen row.
    pub row: usize,
    pub mods: Mods,
}

/// Encode a logical key press as bytes for the PTY.
///
/// Shift on text is already reflected in `KeyInput::Char` by the windowing
/// layer; for named keys it becomes an xterm modifier parameter.
pub fn encode(key: KeyInput, mods: Mods, mode: TermMode) -> Option<Vec<u8>> {
    match key {
        KeyInput::Char(ch) => encode_char(ch, mods),
        KeyInput::Enter => Some(alt_prefixed(b"\r".to_vec(), mods)),
        KeyInput::Backspace => Some(alt_prefixed(vec![0x7f], mods)),
        KeyInput::Tab => Some(alt_prefixed(b"\t".to_vec(), mods)),
        KeyInput::BackTab => Some(b"\x1b[Z".to_vec()),
        KeyInput::Escape => Some(alt_prefixed(vec![0x1b], mods)),
        KeyInput::Up => Some(encode_arrow(b'A', mods, mode)),
        KeyInput::Down => Some(encode_arrow(b'B', mods, mode)),
        KeyInput::Right => Some(encode_arrow(b'C', mods, mode)),
        KeyInput::Left => Some(encode_arrow(b'D', mods, mode)),
        KeyInput::Home => Some(encode_home_end(b'H', mods)),
        KeyInput::End => Some(encode_home_end(b'F', mods)),
        KeyInput::Insert => Some(encode_tilde(2, mods)),
        KeyInput::Delete => Some(encode_tilde(3, mods)),
        KeyInput::PageUp => Some(encode_tilde(5, mods)),
        KeyInput::PageDown => Some(encode_tilde(6, mods)),
        KeyInput::Function(number) => encode_function(number, mods),
    }
}

fn encode_char(ch: char, mods: Mods) -> Option<Vec<u8>> {
    if mods.ctrl {
        let lower = ch.to_ascii_lowercase();
        if !lower.is_ascii_lowercase() {
            return None;
        }
        let control = (lower as u8) - b'a' + 1;
        return Some(if mods.alt {
            vec![0x1b, control]
        } else {
            vec![control]
        });
    }
    let mut buf = [0u8; 4];
    Some(alt_prefixed(
        ch.encode_utf8(&mut buf).as_bytes().to_vec(),
        mods,
    ))
}

fn alt_prefixed(mut bytes: Vec<u8>, mods: Mods) -> Vec<u8> {
    if mods.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

fn encode_arrow(final_byte: u8, mods: Mods, mode: TermMode) -> Vec<u8> {
    if has_modifier(mods) {
        csi_with_modifier(modifier_parameter(mods), final_byte)
    } else {
        cursor_key(final_byte, mode)
    }
}

fn encode_home_end(final_byte: u8, mods: Mods) -> Vec<u8> {
    if has_modifier(mods) {
        csi_with_modifier(modifier_parameter(mods), final_byte)
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn encode_tilde(code: u8, mods: Mods) -> Vec<u8> {
    if has_modifier(mods) {
        format!("\x1b[{code};{}~", modifier_parameter(mods)).into_bytes()
    } else {
        format!("\x1b[{code}~").into_bytes()
    }
}

fn encode_function(number: u8, mods: Mods) -> Option<Vec<u8>> {
    match number {
        1..=4 => {
            let final_byte = b'P' + (number - 1);
            if has_modifier(mods) {
                Some(csi_with_modifier(modifier_parameter(mods), final_byte))
            } else {
                Some(vec![0x1b, b'O', final_byte])
            }
        }
        5..=12 => {
            const CODES: [u8; 8] = [15, 17, 18, 19, 20, 21, 23, 24];
            Some(encode_tilde(CODES[(number - 5) as usize], mods))
        }
        _ => None,
    }
}

fn csi_with_modifier(parameter: u8, final_byte: u8) -> Vec<u8> {
    vec![0x1b, b'[', b'1', b';', b'0' + parameter, final_byte]
}

/// xterm modifier parameter: `1 + shift + 2*alt + 4*ctrl`.
fn modifier_parameter(mods: Mods) -> u8 {
    1 + u8::from(mods.shift) + 2 * u8::from(mods.alt) + 4 * u8::from(mods.ctrl)
}

fn has_modifier(mods: Mods) -> bool {
    mods.shift || mods.alt || mods.ctrl
}

fn cursor_key(final_byte: u8, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// Encode a mouse event for the application, or `None` when it is not
/// reporting the mouse.
pub fn encode_mouse(event: MouseEvent, mode: TermMode) -> Option<Vec<u8>> {
    if !mode.intersects(TermMode::MOUSE_MODE) {
        return None;
    }

    let sgr = mode.contains(TermMode::SGR_MOUSE);
    let code = if event.kind == MouseEventKind::Release && !sgr {
        apply_mouse_mods(3, event.mods)
    } else {
        apply_mouse_mods(button_code(event), event.mods)
    };
    let x = event.column + 1;
    let y = event.row + 1;

    if sgr {
        let terminator = if event.kind == MouseEventKind::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{terminator}").into_bytes());
    }

    if mode.contains(TermMode::UTF8_MOUSE) {
        let mut bytes = vec![0x1b, b'[', b'M', 32 + code];
        push_utf8_codepoint(&mut bytes, 32 + x as u32);
        push_utf8_codepoint(&mut bytes, 32 + y as u32);
        return Some(bytes);
    }

    Some(vec![
        0x1b,
        b'[',
        b'M',
        32 + code,
        (32 + x as u32).min(255) as u8,
        (32 + y as u32).min(255) as u8,
    ])
}

fn button_code(event: MouseEvent) -> u8 {
    let base = match event.kind {
        MouseEventKind::Press | MouseEventKind::Release | MouseEventKind::Motion => {
            match event.button {
                Some(MouseButton::Left) => 0,
                Some(MouseButton::Middle) => 1,
                Some(MouseButton::Right) => 2,
                None => 3,
            }
        }
        MouseEventKind::WheelUp => 64,
        MouseEventKind::WheelDown => 65,
        MouseEventKind::WheelLeft => 66,
        MouseEventKind::WheelRight => 67,
    };
    if event.kind == MouseEventKind::Motion {
        base + 32
    } else {
        base
    }
}

fn apply_mouse_mods(code: u8, mods: Mods) -> u8 {
    code + 4 * u8::from(mods.shift) + 8 * u8::from(mods.alt) + 16 * u8::from(mods.ctrl)
}

fn push_utf8_codepoint(bytes: &mut Vec<u8>, codepoint: u32) {
    let ch = char::from_u32(codepoint).unwrap_or(char::REPLACEMENT_CHARACTER);
    let mut buf = [0u8; 4];
    bytes.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

/// Encode focus gained (`ESC [ I`) or lost (`ESC [ O`) for `FOCUS_IN_OUT`.
pub fn encode_focus(focused: bool) -> Vec<u8> {
    if focused {
        b"\x1b[I".to_vec()
    } else {
        b"\x1b[O".to_vec()
    }
}

/// Encode pasted text, optionally using xterm bracketed paste.
///
/// Clipboard text is untrusted. Inside a bracketed paste a literal terminator
/// would end the paste early and deliver the rest as typed input, so each marker
/// is replaced (not deleted) by U+FFFD before the text is wrapped: deleting the
/// bytes would let the surrounding text splice back into a live marker.
/// Non-bracketed paste stays verbatim.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }

    let text = text
        .replace("\x1b[200~", "\u{FFFD}")
        .replace("\x1b[201~", "\u{FFFD}");
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
            shift: false,
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
            shift: false,
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
            shift: false,
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

    #[test]
    fn paste_replaces_bracketed_paste_markers() {
        let bytes = paste("safe\x1b[201~rm -rf /\x1b[200~tail", true);
        assert_eq!(
            bytes,
            "\x1b[200~safe\u{FFFD}rm -rf /\u{FFFD}tail\x1b[201~".as_bytes()
        );
    }

    #[test]
    fn paste_cannot_splice_a_live_terminator() {
        for payload in [
            "a\x1b[201~b",
            "a\x1b[20\x1b[201~1~b",
            "a\x1b[200~\x1b[201~b",
        ] {
            let bytes = paste(payload, true);
            assert_eq!(
                bytes.windows(6).filter(|w| *w == b"\x1b[200~").count(),
                1,
                "exactly one opening marker for {payload:?}"
            );
            assert_eq!(
                bytes.windows(6).filter(|w| *w == b"\x1b[201~").count(),
                1,
                "exactly one closing marker for {payload:?}"
            );
            assert!(bytes.starts_with(b"\x1b[200~"), "{payload:?}");
            assert!(bytes.ends_with(b"\x1b[201~"), "{payload:?}");
        }
    }

    #[test]
    fn non_bracketed_paste_keeps_markers_verbatim() {
        assert_eq!(paste("a\x1b[201~b", false), b"a\x1b[201~b".to_vec());
    }

    #[test]
    fn shift_tab_is_backtab() {
        assert_eq!(
            encode(KeyInput::BackTab, plain(), TermMode::empty()),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn function_keys_use_standard_sequences() {
        let mode = TermMode::empty();
        assert_eq!(
            encode(KeyInput::Function(1), plain(), mode),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Function(5), plain(), mode),
            Some(b"\x1b[15~".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Function(12), plain(), mode),
            Some(b"\x1b[24~".to_vec())
        );
        assert_eq!(encode(KeyInput::Function(13), plain(), mode), None);
    }

    #[test]
    fn modified_navigation_uses_csi_parameters() {
        let mode = TermMode::empty();
        let ctrl = Mods {
            ctrl: true,
            alt: false,
            shift: false,
        };
        let shift = Mods {
            ctrl: false,
            alt: false,
            shift: true,
        };
        let alt = Mods {
            ctrl: false,
            alt: true,
            shift: false,
        };
        assert_eq!(
            encode(KeyInput::Left, ctrl, mode),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Home, shift, mode),
            Some(b"\x1b[1;2H".to_vec())
        );
        assert_eq!(
            encode(KeyInput::Function(5), alt, mode),
            Some(b"\x1b[15;3~".to_vec())
        );
    }

    #[test]
    fn ctrl_alt_char_prefixes_escape() {
        let mods = Mods {
            ctrl: true,
            alt: true,
            shift: false,
        };
        assert_eq!(
            encode(KeyInput::Char('a'), mods, TermMode::empty()),
            Some(vec![0x1b, 1])
        );
    }

    fn mouse(kind: MouseEventKind, button: Option<MouseButton>) -> MouseEvent {
        MouseEvent {
            kind,
            button,
            column: 2,
            row: 4,
            mods: plain(),
        }
    }

    #[test]
    fn sgr_mouse_press_release_and_motion() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(
            encode_mouse(mouse(MouseEventKind::Press, Some(MouseButton::Left)), mode),
            Some(b"\x1b[<0;3;5M".to_vec())
        );
        assert_eq!(
            encode_mouse(
                mouse(MouseEventKind::Release, Some(MouseButton::Left)),
                mode
            ),
            Some(b"\x1b[<0;3;5m".to_vec())
        );
        assert_eq!(
            encode_mouse(mouse(MouseEventKind::Motion, Some(MouseButton::Left)), mode),
            Some(b"\x1b[<32;3;5M".to_vec())
        );
        assert_eq!(
            encode_mouse(mouse(MouseEventKind::WheelUp, None), mode),
            Some(b"\x1b[<64;3;5M".to_vec())
        );
    }

    #[test]
    fn legacy_mouse_uses_three_bytes() {
        let mode = TermMode::MOUSE_REPORT_CLICK;
        let press = MouseEvent {
            kind: MouseEventKind::Press,
            button: Some(MouseButton::Left),
            column: 0,
            row: 0,
            mods: plain(),
        };
        assert_eq!(
            encode_mouse(press, mode),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        let release = MouseEvent {
            kind: MouseEventKind::Release,
            ..press
        };
        assert_eq!(
            encode_mouse(release, mode),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
    }

    #[test]
    fn utf8_mouse_encodes_coordinates() {
        let mode = TermMode::MOUSE_REPORT_CLICK | TermMode::UTF8_MOUSE;
        let event = MouseEvent {
            kind: MouseEventKind::Press,
            button: Some(MouseButton::Left),
            column: 100,
            row: 4,
            mods: plain(),
        };
        let bytes = encode_mouse(event, mode).unwrap();
        assert_eq!(&bytes[..3], b"\x1b[M");
        assert_eq!(bytes[3], 32);
        // 32 + 101 = 133 = U+0085, encoded as two UTF-8 bytes.
        assert_eq!(&bytes[4..6], &[0xc2, 0x85]);
        assert_eq!(bytes[6], 37);
    }

    #[test]
    fn mouse_requires_a_mouse_mode() {
        let event = mouse(MouseEventKind::Press, Some(MouseButton::Left));
        assert_eq!(encode_mouse(event, TermMode::empty()), None);
        assert_eq!(encode_mouse(event, TermMode::SGR_MOUSE), None);
    }

    #[test]
    fn focus_reporting_uses_csi_i_and_o() {
        assert_eq!(encode_focus(true), b"\x1b[I".to_vec());
        assert_eq!(encode_focus(false), b"\x1b[O".to_vec());
    }
}
