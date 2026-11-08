//! Window, surface, and event wiring.

use std::num::NonZeroU32;
use std::sync::Arc;

use alacritty_terminal::term::TermMode;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::event::UserEvent;
use crate::fonts::FontSet;
use crate::input::{self, KeyInput, Mods};
use crate::pty::PtySession;
use crate::render::Renderer;
use crate::selection::Selection;
use crate::term::{LogicalPos, Terminal};
use crate::theme::Theme;
use crate::visual::VisualMap;

/// Upper bound on a single paste, in Unicode scalar values. A larger clipboard
/// would freeze the UI thread while writing to a shell that may not drain it.
const MAX_PASTE_CHARS: usize = 1_048_576;

pub struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    renderer: Option<Renderer>,
    terminal: Option<Terminal>,
    pty: Option<PtySession>,
    clipboard: Option<arboard::Clipboard>,
    modifiers: ModifiersState,
    selection: Option<Selection>,
    mouse_down: bool,
    // Sub-row pixel wheel deltas carried between events so touchpads that emit
    // fractions of a cell per event still scroll.
    wheel_remainder: f32,
    cursor_pos: (f64, f64),
    cols: usize,
    rows: usize,
}

impl App {
    pub fn new(config: Config, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            config,
            proxy,
            window: None,
            surface: None,
            renderer: None,
            terminal: None,
            pty: None,
            clipboard: None,
            modifiers: ModifiersState::empty(),
            selection: None,
            mouse_down: false,
            wheel_remainder: 0.0,
            cursor_pos: (0.0, 0.0),
            cols: 80,
            rows: 24,
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let attributes = Window::default_attributes()
            .with_title("Termyx")
            .with_inner_size(LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ));
        let window = Arc::new(event_loop.create_window(attributes)?);
        // winit starts with IME disabled on Windows; committed composition text
        // arrives as `WindowEvent::Ime` and is forwarded to the PTY.
        window.set_ime_allowed(true);

        let context = Context::new(window.clone())
            .map_err(|err| anyhow::anyhow!("softbuffer context: {err}"))?;
        let mut surface = Surface::new(&context, window.clone())
            .map_err(|err| anyhow::anyhow!("softbuffer surface: {err}"))?;

        let fonts = FontSet::load(
            &self.config.font.family,
            &self.config.font.arabic_family,
            self.config.font.size,
            self.config.font.line_height,
        )?;
        let theme = Theme::from_config(&self.config.theme);
        let mut renderer = Renderer::new(fonts, theme, self.config.window.padding)?;

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        renderer.resize(width, height);

        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
        );
        let terminal = Terminal::new(cols, rows, self.config.scrollback.lines);
        let pty = PtySession::spawn(
            &self.config.shell,
            cols as u16,
            rows as u16,
            self.proxy.clone(),
        )?;

        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            if let Err(err) = surface.resize(w, h) {
                tracing::warn!(%err, "initial surface resize failed");
            }
        }

        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.terminal = Some(terminal);
        self.pty = Some(pty);
        self.cols = cols;
        self.rows = rows;

        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        let (Some(surface), Some(renderer)) = (&mut self.surface, &mut self.renderer) else {
            return;
        };
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        if let Err(err) = surface.resize(w, h) {
            tracing::warn!(%err, "surface resize failed");
        }
        renderer.resize(width, height);
    }

    /// Rebuilds the grid for the new surface size. Returns whether the grid
    /// actually changed and therefore reflowed.
    fn resize_grid(&mut self, width: u32, height: u32) -> bool {
        let (Some(renderer), Some(terminal), Some(pty)) =
            (&self.renderer, &mut self.terminal, &mut self.pty)
        else {
            return false;
        };
        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
        );
        if cols == self.cols && rows == self.rows {
            return false;
        }
        terminal.resize(cols, rows);
        if let Err(err) = pty.resize(cols as u16, rows as u16) {
            tracing::warn!(%err, "pty resize failed");
        }
        self.cols = cols;
        self.rows = rows;
        true
    }

    fn render(&mut self) {
        let (Some(renderer), Some(terminal), Some(surface)) =
            (&mut self.renderer, &self.terminal, &mut self.surface)
        else {
            return;
        };
        let snapshot = terminal.snapshot();
        let maps: Vec<VisualMap> = snapshot
            .lines
            .iter()
            .map(|line| VisualMap::for_row(line, &self.config.bidi))
            .collect();
        renderer.draw(&snapshot, &maps, self.selection.as_ref());
        renderer.sync();

        match surface.buffer_mut() {
            Ok(mut buffer) => {
                if buffer.len() != renderer.pixels().len() {
                    tracing::warn!(
                        surface = buffer.len(),
                        renderer = renderer.pixels().len(),
                        "surface buffer size mismatch; skipping frame"
                    );
                    return;
                }
                buffer.copy_from_slice(renderer.pixels());
                if let Err(err) = buffer.present() {
                    tracing::warn!(%err, "present failed");
                }
            }
            Err(err) => tracing::warn!(%err, "surface buffer unavailable"),
        }
    }

    fn on_key(&mut self, event: KeyEvent) {
        if self.modifiers.control_key() && self.modifiers.shift_key() {
            if let Key::Character(text) = &event.logical_key {
                match text.as_str() {
                    "c" | "C" => {
                        self.copy_selection();
                        return;
                    }
                    "v" | "V" => {
                        if !event.repeat {
                            self.paste_clipboard();
                        }
                        return;
                    }
                    _ => {}
                }
            }
        }
        let Some(key) = map_key(&event.logical_key) else {
            return;
        };
        let mode = self
            .terminal
            .as_ref()
            .map(Terminal::mode)
            .unwrap_or_else(TermMode::empty);
        let Some(bytes) = input::encode(key, mods_from(self.modifiers), mode) else {
            return;
        };
        self.write_input(&bytes);
    }

    /// Writes text to the shell through the shared PTY path.
    fn send_text(&mut self, text: &str) {
        self.write_input(text.as_bytes());
    }

    fn write_input(&mut self, bytes: &[u8]) {
        if let Some(pty) = &mut self.pty {
            if let Err(err) = pty.write(bytes) {
                tracing::warn!(%err, "pty write failed");
            }
        }
    }

    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_pos = (x, y);
        if !self.mouse_down {
            return;
        }
        let Some(position) = self.pos_at(x, y) else {
            return;
        };
        if let Some(selection) = &mut self.selection {
            selection.update(position);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        let cell_h = match &self.renderer {
            Some(renderer) => renderer.fonts().cell_h as f32,
            None => return,
        };
        let lines = wheel_lines(delta, cell_h, &mut self.wheel_remainder);
        if lines == 0 {
            return;
        }
        let Some(terminal) = &mut self.terminal else {
            return;
        };
        let before = terminal.display_offset();
        terminal.scroll_display(lines);
        if terminal.display_offset() == before {
            return;
        }
        // Selection positions are logical viewport offsets; scrolling makes them stale.
        self.selection = None;
        self.mouse_down = false;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn on_mouse(&mut self, state: ElementState) {
        match state {
            ElementState::Pressed => {
                let Some(position) = self.pos_at(self.cursor_pos.0, self.cursor_pos.1) else {
                    return;
                };
                self.selection = Some(Selection::new(position));
                self.mouse_down = true;
            }
            ElementState::Released => self.mouse_down = false,
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn pos_at(&self, x: f64, y: f64) -> Option<LogicalPos> {
        let renderer = self.renderer.as_ref()?;
        let (row, visual) = cell_at(
            x,
            y,
            renderer.padding() as f64,
            renderer.fonts().cell_w as f64,
            renderer.fonts().cell_h as f64,
            self.cols,
            self.rows,
        );
        let map = VisualMap::identity(self.cols);
        Some(LogicalPos {
            row,
            col: map.logical_at(visual),
        })
    }

    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            match arboard::Clipboard::new() {
                Ok(clipboard) => self.clipboard = Some(clipboard),
                Err(err) => {
                    tracing::warn!(%err, "clipboard unavailable");
                    return None;
                }
            }
        }
        self.clipboard.as_mut()
    }

    fn copy_selection(&mut self) {
        let Some(selection) = &self.selection else {
            return;
        };
        let Some(terminal) = &self.terminal else {
            return;
        };
        let text = selection.to_text(&terminal.snapshot(), self.config.selection.trim_trailing);
        let Some(clipboard) = self.clipboard() else {
            return;
        };
        if let Err(err) = clipboard.set_text(text) {
            tracing::warn!(%err, "copy failed");
        }
    }

    fn paste_clipboard(&mut self) {
        let Some(clipboard) = self.clipboard() else {
            return;
        };
        let text = match clipboard.get_text() {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(%err, "paste failed");
                return;
            }
        };
        let text = cap_paste(text);
        let bracketed = self
            .terminal
            .as_ref()
            .map(|terminal| terminal.mode().contains(TermMode::BRACKETED_PASTE))
            .unwrap_or(false);
        let bytes = input::paste(&text, bracketed);
        self.write_input(&bytes);
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(err) = self.create_window(event_loop) {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Termyx")
                .set_description(format!("Startup failed: {err:#}"))
                .show();
            event_loop.exit();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyOutput(bytes) => {
                if let Some(terminal) = &mut self.terminal {
                    terminal.feed(&bytes);
                }
                // Output can scroll rows under a fixed logical anchor (even at
                // the scrollback cap, where history size stops growing), so a
                // pending selection may now cover different text.
                self.selection = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if size.width == 0 || size.height == 0 {
                    return;
                }
                self.resize_surface(size.width, size.height);
                if self.resize_grid(size.width, size.height) {
                    // Reflow invalidates logical selection positions.
                    self.selection = None;
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(position.x, position.y)
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.on_mouse(state),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.on_key(event)
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.send_text(&text),
            // Composition preview (preedit) is Phase 3; the OS candidate window
            // is enough until the grid can render it.
            WindowEvent::Ime(_) => {}
            _ => {}
        }
    }
}

fn grid_size(width: u32, height: u32, cell_w: u32, cell_h: u32, padding: u32) -> (usize, usize) {
    let usable_w = width.saturating_sub(padding.saturating_mul(2));
    let usable_h = height.saturating_sub(padding.saturating_mul(2));
    let cols = (usable_w / cell_w.max(1)).max(20) as usize;
    let rows = (usable_h / cell_h.max(1)).max(5) as usize;
    (cols, rows)
}

fn map_key(key: &Key) -> Option<KeyInput> {
    match key {
        Key::Character(text) => text.chars().next().map(KeyInput::Char),
        Key::Named(NamedKey::Space) => Some(KeyInput::Char(' ')),
        Key::Named(NamedKey::Enter) => Some(KeyInput::Enter),
        Key::Named(NamedKey::Backspace) => Some(KeyInput::Backspace),
        Key::Named(NamedKey::Tab) => Some(KeyInput::Tab),
        Key::Named(NamedKey::Escape) => Some(KeyInput::Escape),
        Key::Named(NamedKey::ArrowUp) => Some(KeyInput::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KeyInput::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyInput::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyInput::Right),
        Key::Named(NamedKey::Home) => Some(KeyInput::Home),
        Key::Named(NamedKey::End) => Some(KeyInput::End),
        Key::Named(NamedKey::Delete) => Some(KeyInput::Delete),
        Key::Named(NamedKey::Insert) => Some(KeyInput::Insert),
        Key::Named(NamedKey::PageUp) => Some(KeyInput::PageUp),
        Key::Named(NamedKey::PageDown) => Some(KeyInput::PageDown),
        _ => None,
    }
}

fn mods_from(modifiers: ModifiersState) -> Mods {
    Mods {
        ctrl: modifiers.control_key(),
        alt: modifiers.alt_key(),
    }
}

fn cell_at(
    x: f64,
    y: f64,
    padding: f64,
    cell_w: f64,
    cell_h: f64,
    cols: usize,
    rows: usize,
) -> (usize, usize) {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let visual = (((x - padding) / cell_w).floor() as isize).clamp(0, cols as isize - 1) as usize;
    let row = (((y - padding) / cell_h).floor() as isize).clamp(0, rows as isize - 1) as usize;
    (row, visual)
}

fn wheel_lines(delta: MouseScrollDelta, cell_h: f32, remainder: &mut f32) -> i32 {
    *remainder += match delta {
        MouseScrollDelta::LineDelta(_, y) => y * 3.0,
        MouseScrollDelta::PixelDelta(position) => position.y as f32 / cell_h.max(1.0),
    };
    let lines = remainder.trunc();
    *remainder -= lines;
    lines as i32
}

/// Truncates a clipboard payload to [`MAX_PASTE_CHARS`] without splitting a
/// UTF-8 scalar value.
fn cap_paste(mut text: String) -> String {
    if let Some((index, _)) = text.char_indices().nth(MAX_PASTE_CHARS) {
        tracing::warn!(limit = MAX_PASTE_CHARS, "paste truncated");
        text.truncate(index);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_size_uses_padding_and_minimums() {
        assert_eq!(grid_size(1200, 800, 10, 20, 4), (119, 39));
        assert_eq!(grid_size(100, 100, 10, 20, 4), (20, 5));
    }

    #[test]
    fn map_key_recognizes_text_and_named_keys() {
        assert_eq!(
            map_key(&Key::Character("a".into())),
            Some(KeyInput::Char('a'))
        );
        assert_eq!(map_key(&Key::Named(NamedKey::Enter)), Some(KeyInput::Enter));
        assert_eq!(map_key(&Key::Named(NamedKey::F1)), None);
    }

    #[test]
    fn modifiers_map_to_ctrl_and_alt() {
        let mods = mods_from(ModifiersState::CONTROL | ModifiersState::ALT);
        assert!(mods.ctrl);
        assert!(mods.alt);
    }

    #[test]
    fn cell_at_maps_pixels_to_grid_cells() {
        assert_eq!(cell_at(4.0, 4.0, 4.0, 10.0, 20.0, 80, 24), (0, 0));
        assert_eq!(cell_at(13.0, 24.0, 4.0, 10.0, 20.0, 80, 24), (1, 0));
        assert_eq!(
            cell_at(10_000.0, 10_000.0, 4.0, 10.0, 20.0, 80, 24),
            (23, 79)
        );
    }

    #[test]
    fn wheel_lines_converts_scroll_to_rows() {
        use winit::dpi::PhysicalPosition;

        let mut remainder = 0.0;
        assert_eq!(
            wheel_lines(MouseScrollDelta::LineDelta(0.0, 1.0), 20.0, &mut remainder),
            3
        );
        assert_eq!(
            wheel_lines(MouseScrollDelta::LineDelta(0.0, -1.0), 20.0, &mut remainder),
            -3
        );
        assert_eq!(
            wheel_lines(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 40.0)),
                20.0,
                &mut remainder
            ),
            2
        );
        // A zero cell height must not divide by zero.
        assert_eq!(
            wheel_lines(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 40.0)),
                0.0,
                &mut remainder
            ),
            40
        );
    }

    #[test]
    fn wheel_lines_accumulates_fractional_pixel_deltas() {
        use winit::dpi::PhysicalPosition;

        // A quarter cell per event: three events stay below one row, the fourth
        // completes it and leaves no remainder.
        let delta = MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 5.0));
        let mut remainder = 0.0;
        for _ in 0..3 {
            assert_eq!(wheel_lines(delta, 20.0, &mut remainder), 0);
        }
        assert_eq!(wheel_lines(delta, 20.0, &mut remainder), 1);
        assert_eq!(remainder, 0.0);
    }

    #[test]
    fn wheel_lines_accumulates_fractional_line_deltas() {
        let delta = MouseScrollDelta::LineDelta(0.0, 0.5);
        let mut remainder = 0.0;
        assert_eq!(wheel_lines(delta, 20.0, &mut remainder), 1);
        assert_eq!(remainder, 0.5);
        assert_eq!(wheel_lines(delta, 20.0, &mut remainder), 2);
        assert_eq!(remainder, 0.0);
    }

    #[test]
    fn cap_paste_truncates_without_splitting_utf8() {
        let text = "é".repeat(MAX_PASTE_CHARS + 1);
        let capped = cap_paste(text);
        assert_eq!(capped.chars().count(), MAX_PASTE_CHARS);
        assert!(capped.chars().all(|ch| ch == 'é'));
    }
}
