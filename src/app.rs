//! Window, surface, and event wiring.

use std::num::NonZeroU32;
use std::sync::Arc;

use alacritty_terminal::term::TermMode;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::event::UserEvent;
use crate::fonts::FontSet;
use crate::input::{self, KeyInput, Mods};
use crate::pty::PtySession;
use crate::render::Renderer;
use crate::term::Terminal;
use crate::theme::Theme;
use crate::visual::VisualMap;

pub struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    renderer: Option<Renderer>,
    terminal: Option<Terminal>,
    pty: Option<PtySession>,
    modifiers: ModifiersState,
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
            modifiers: ModifiersState::empty(),
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

        let context = Context::new(window.clone())
            .map_err(|err| anyhow::anyhow!("softbuffer context: {err}"))?;
        let mut surface = Surface::new(&context, window.clone())
            .map_err(|err| anyhow::anyhow!("softbuffer surface: {err}"))?;

        let fonts = FontSet::load(
            &self.config.font.family,
            self.config.font.size,
            self.config.font.line_height,
        )?;
        let theme = Theme::from_config(&self.config.theme);
        let mut renderer = Renderer::new(fonts, theme, self.config.window.padding);

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

    fn resize_grid(&mut self, width: u32, height: u32) {
        let (Some(renderer), Some(terminal), Some(pty)) =
            (&self.renderer, &mut self.terminal, &mut self.pty)
        else {
            return;
        };
        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
        );
        if cols == self.cols && rows == self.rows {
            return;
        }
        terminal.resize(cols, rows);
        if let Err(err) = pty.resize(cols as u16, rows as u16) {
            tracing::warn!(%err, "pty resize failed");
        }
        self.cols = cols;
        self.rows = rows;
    }

    fn render(&mut self) {
        let (Some(renderer), Some(terminal), Some(surface)) =
            (&mut self.renderer, &self.terminal, &mut self.surface)
        else {
            return;
        };
        let snapshot = terminal.snapshot();
        let maps = vec![VisualMap::identity(snapshot.cols); snapshot.rows];
        renderer.draw(&snapshot, &maps);
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
        if let Some(pty) = &mut self.pty {
            if let Err(err) = pty.write(&bytes) {
                tracing::warn!(%err, "pty write failed");
            }
        }
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
                self.resize_grid(size.width, size.height);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.on_key(event)
            }
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
}
