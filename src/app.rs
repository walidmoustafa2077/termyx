//! Window, surface, and event wiring.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::config::{BidiConfig, Config};
use crate::event::UserEvent;
use crate::fonts::FontSet;
use crate::input::{self, KeyInput, Mods};
use crate::platform;
use crate::pty::PtySession;
use crate::render::Renderer;
use crate::scrollbar::{self, ScrollbarGeometry, ScrollbarHit};
use crate::selection::{line_bounds, word_bounds, Selection, SelectionMode};
use crate::term::{LogicalPos, RowSnapshot, Terminal};
use crate::theme::Theme;
use crate::visual::VisualMap;

/// Upper bound on a single paste, in Unicode scalar values. A larger clipboard
/// would freeze the UI thread while writing to a shell that may not drain it.
const MAX_PASTE_CHARS: usize = 1_048_576;

/// Upper bound on an OSC 52 payload, in bytes. The VT parser accumulates the
/// whole sequence before the UI thread writes it to the clipboard.
const MAX_OSC52_BYTES: usize = 100 * 1024;

const DEFAULT_TITLE: &str = "Termyx";

/// Longest delay between two clicks of one multi-click gesture.
const CLICK_TIMEOUT: Duration = Duration::from_millis(400);

/// Largest cursor movement between two clicks of one multi-click gesture.
const CLICK_DISTANCE: f64 = 4.0;

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
    click_count: u8,
    last_click: Option<(Instant, f64, f64)>,
    // Button forwarded to a mouse-reporting application, if any. Local text
    // selection uses `mouse_down` instead.
    app_mouse_button: Option<input::MouseButton>,
    // Sub-row pixel wheel deltas carried between events so touchpads that emit
    // fractions of a cell per event still scroll.
    wheel_remainder: f32,
    scrollbar_drag: bool,
    scrollbar_grab: f32,
    scrollbar_hover: bool,
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
            click_count: 0,
            last_click: None,
            app_mouse_button: None,
            wheel_remainder: 0.0,
            scrollbar_drag: false,
            scrollbar_grab: 0.0,
            scrollbar_hover: false,
            cursor_pos: (0.0, 0.0),
            cols: 80,
            rows: 24,
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let monitor = event_loop.primary_monitor().map(|monitor| {
            let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
            (size.width.round() as u32, size.height.round() as u32)
        });
        let (width, height) =
            initial_window_size(self.config.window.width, self.config.window.height, monitor);
        let attributes = Window::default_attributes()
            .with_title(DEFAULT_TITLE)
            .with_inner_size(LogicalSize::new(width, height));
        let window = Arc::new(event_loop.create_window(attributes)?);
        // Center on the primary monitor so the OS never places the window over
        // the taskbar or past the screen edge.
        if let Some(monitor) = event_loop.primary_monitor() {
            let outer = window.outer_size();
            let monitor_pos = monitor.position();
            let monitor_size = monitor.size();
            let x = monitor_pos.x + (monitor_size.width.saturating_sub(outer.width) / 2) as i32;
            let y = monitor_pos.y + (monitor_size.height.saturating_sub(outer.height) / 2) as i32;
            window.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        }
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
            scaled_font_size(self.config.font.size, window.scale_factor()),
            self.config.font.line_height,
        )?;
        let theme = Theme::from_config(&self.config.theme);
        platform::apply_dark_frame(&window, theme.background);
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
        let terminal = Terminal::new_with_events(
            cols,
            rows,
            self.config.scrollback.lines,
            Some(self.proxy.clone()),
        );
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
        let scrollbar_hover = self.scrollbar_hover || self.scrollbar_drag;
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
        renderer.set_scrollbar_hover(scrollbar_hover);
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
        // Shift+PageUp/PageDown scroll the local viewport when there is
        // scrollback. Without history they fall through as modified keys so
        // full-screen applications still receive them.
        if self.modifiers.shift_key() {
            let page = self.page_lines();
            let lines = match event.logical_key {
                Key::Named(NamedKey::PageUp) => Some(page),
                Key::Named(NamedKey::PageDown) => Some(-page),
                _ => None,
            };
            if let Some(lines) = lines {
                let has_history = self
                    .terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.scrollable_history() > 0);
                if has_history {
                    self.scroll_local(lines);
                    return;
                }
            }
        }
        // Shift+Tab is backtab; `map_key` cannot tell them apart because Tab is
        // a named key.
        let key = if self.modifiers.shift_key()
            && matches!(event.logical_key, Key::Named(NamedKey::Tab))
        {
            KeyInput::BackTab
        } else {
            let Some(key) = map_key(&event.logical_key) else {
                return;
            };
            key
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
        // Keep the scrollbar expanded for the whole drag, even when the pointer
        // slips off the thumb.
        self.set_scrollbar_hover(self.scrollbar_drag || self.scrollbar_hit(x, y).is_some());

        if self.scrollbar_drag {
            self.scroll_to_thumb(y as f32 - self.scrollbar_grab);
            return;
        }

        if let Some(button) = self.app_mouse_button {
            // A forwarded button is down: report motion regardless of Shift so
            // the gesture started by the press stays balanced.
            let mode = self.app_mouse_mode();
            if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                self.send_mouse(input::MouseEventKind::Motion, Some(button));
            }
            return;
        }

        // A local drag must keep extending the selection even if Shift was
        // released mid-gesture, before any 1003 motion steals the event.
        if self.mouse_down {
            if let Some(position) = self.pos_at(x, y) {
                if let Some(selection) = &mut self.selection {
                    selection.update(position);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            return;
        }

        let mode = self.app_mouse_mode();
        if mode.contains(TermMode::MOUSE_MOTION) && !self.modifiers.shift_key() {
            self.send_mouse(input::MouseEventKind::Motion, None);
        }
    }

    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        if self.app_mouse_mode().intersects(TermMode::MOUSE_MODE) && !self.modifiers.shift_key() {
            let (x, y) = match delta {
                MouseScrollDelta::LineDelta(x, y) => (x, y),
                MouseScrollDelta::PixelDelta(position) => (position.x as f32, position.y as f32),
            };
            let kind = if y > 0.0 {
                Some(input::MouseEventKind::WheelUp)
            } else if y < 0.0 {
                Some(input::MouseEventKind::WheelDown)
            } else if x > 0.0 {
                Some(input::MouseEventKind::WheelRight)
            } else if x < 0.0 {
                Some(input::MouseEventKind::WheelLeft)
            } else {
                None
            };
            if let Some(kind) = kind {
                self.send_mouse(kind, None);
            }
            return;
        }

        let cell_h = match &self.renderer {
            Some(renderer) => renderer.fonts().cell_h as f32,
            None => return,
        };
        let lines = wheel_lines(delta, cell_h, &mut self.wheel_remainder);
        if lines == 0 {
            return;
        }

        let mode = self.app_mouse_mode();
        if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // The alternate grid has no scrollback. Mode 1007 turns the wheel
            // into cursor keys so pager-style TUIs still scroll.
            let key = if lines > 0 {
                KeyInput::Up
            } else {
                KeyInput::Down
            };
            if let Some(bytes) = input::encode(key, Mods::default(), mode) {
                for _ in 0..lines.unsigned_abs() {
                    self.write_input(&bytes);
                }
            }
            return;
        }

        self.scroll_local(lines);
    }

    fn on_mouse(&mut self, state: ElementState, button: MouseButton) {
        let Some(input_button) = map_mouse_button(button) else {
            return;
        };

        if state == ElementState::Pressed {
            if self.scrollbar_drag && button != MouseButton::Left {
                // The pointer belongs to the drag; a stray button must not
                // paste or reach the application mid-gesture.
                return;
            }

            if button == MouseButton::Left {
                if let Some((geom, hit)) = self.scrollbar_hit(self.cursor_pos.0, self.cursor_pos.1)
                {
                    // Release any button forwarded before the drag takes over
                    // the pointer, so the application is not left stuck down.
                    if let Some(tracked) = release_button(self.app_mouse_button.take()) {
                        self.send_mouse(input::MouseEventKind::Release, Some(tracked));
                    }
                    self.mouse_down = false;
                    match hit {
                        ScrollbarHit::Thumb(grab) => {
                            self.scrollbar_drag = true;
                            self.scrollbar_grab = grab;
                            self.scroll_to_thumb(self.cursor_pos.1 as f32 - grab);
                        }
                        ScrollbarHit::Track => {
                            let page = self.page_lines();
                            let lines = if (self.cursor_pos.1 as f32) < geom.thumb_y {
                                page
                            } else {
                                -page
                            };
                            self.scroll_local(lines);
                        }
                    }
                    return;
                }
            }

            if self.app_mouse_button.is_none()
                && button == MouseButton::Left
                && self.modifiers.alt_key()
                && !self.modifiers.control_key()
                && !self.modifiers.shift_key()
            {
                let (x, y) = self.cursor_pos;
                if let Some(position) = self.pos_at(x, y) {
                    // Alt+drag is reserved for block selection, even in mouse-reporting apps.
                    // A button already forwarded to the app keeps its own release.
                    self.selection = Some(Selection::block(position));
                    self.mouse_down = true;
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                return;
            }

            // Double and triple clicks stay local even when the application
            // reports the mouse. That is what makes word/line selection work
            // inside TUIs such as OpenCode while single clicks still reach the
            // application (tab switching, buttons, and so on).
            let click_position = if button == MouseButton::Left {
                self.pos_at(self.cursor_pos.0, self.cursor_pos.1)
            } else {
                None
            };
            if let Some(position) = click_position {
                let now = Instant::now();
                self.click_count = next_click_count(
                    self.last_click,
                    self.click_count,
                    now,
                    self.cursor_pos.0,
                    self.cursor_pos.1,
                );
                self.last_click = Some((now, self.cursor_pos.0, self.cursor_pos.1));
                if self.click_count >= 2 {
                    self.app_mouse_button = None;
                    self.mouse_down = false;
                    let whole_line = self.click_count == 3;
                    let Some((start, end)) = self.bounds_at(position, whole_line) else {
                        return;
                    };
                    let mut selection = Selection::new(LogicalPos {
                        row: position.row,
                        col: start,
                    });
                    selection.update(LogicalPos {
                        row: position.row,
                        col: end,
                    });
                    self.selection = Some(selection);
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    return;
                }
            }

            // A single click starts a new interaction. Drop any previous
            // selection even when the click is forwarded to an application;
            // otherwise a stale highlight survives clicks outside it.
            if button == MouseButton::Left && self.selection.is_some() {
                self.selection = None;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }

            if forwards_mouse_press(self.app_mouse_mode(), self.modifiers.shift_key()) {
                self.app_mouse_button = Some(input_button);
                self.mouse_down = false;
                self.send_mouse(input::MouseEventKind::Press, Some(input_button));
                return;
            }

            // Shift+drag and non-reporting apps keep local text selection.
            self.app_mouse_button = None;

            // A right click that stays local pastes; the forwarding branch
            // above has already claimed the click for mouse-reporting apps.
            if button == MouseButton::Right {
                self.paste_clipboard();
                return;
            }
            if button != MouseButton::Left {
                return;
            }

            let Some(position) = click_position else {
                return;
            };
            self.selection = Some(Selection::visual(position));
            self.mouse_down = true;
        } else if self.scrollbar_drag && button == MouseButton::Left {
            self.scrollbar_drag = false;
            let (x, y) = self.cursor_pos;
            self.set_scrollbar_hover(self.scrollbar_hit(x, y).is_some());
            return;
        } else if let Some(tracked) = release_button(self.app_mouse_button.take()) {
            // Route by the state captured at press time: a forwarded press is
            // released even when Shift or the mouse mode changed mid-gesture.
            self.send_mouse(input::MouseEventKind::Release, Some(tracked));
            self.mouse_down = false;
            return;
        } else {
            self.mouse_down = false;
            if button == MouseButton::Left && self.config.selection.copy_on_select {
                if let Some(selection) = &self.selection {
                    let collapsed = selection.anchor == selection.focus;
                    if should_copy_on_release(self.click_count, collapsed) {
                        self.copy_selection();
                    }
                }
            }
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Inclusive cell range of the word or row containing `position`.
    fn bounds_at(&self, position: LogicalPos, whole_line: bool) -> Option<(usize, usize)> {
        let terminal = self.terminal.as_ref()?;
        let snapshot = terminal.snapshot();
        let line = snapshot.lines.get(position.row)?;
        Some(if whole_line {
            line_bounds(line)
        } else {
            word_bounds(line, position.col)
        })
    }

    fn app_mouse_mode(&self) -> TermMode {
        self.terminal
            .as_ref()
            .map(Terminal::mode)
            .unwrap_or_else(TermMode::empty)
    }

    fn scrollbar_geometry(&self, hovered: bool) -> Option<ScrollbarGeometry> {
        let renderer = self.renderer.as_ref()?;
        let terminal = self.terminal.as_ref()?;
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        scrollbar::geometry(
            size.width,
            renderer.padding(),
            renderer.fonts().cell_h,
            self.rows,
            terminal.scrollable_history(),
            terminal.display_offset(),
            hovered,
        )
    }

    fn scrollbar_hit(&self, x: f64, y: f64) -> Option<(ScrollbarGeometry, ScrollbarHit)> {
        // Hit-test the expanded width so the pointer stays on the thumb while
        // it is hovered. The reserved gutter keeps this strip clear of text.
        let geom = self.scrollbar_geometry(true)?;
        let x = x as f32;
        let y = y as f32;
        if x < geom.x || x > geom.x + geom.width {
            return None;
        }
        if y < geom.track_y || y > geom.track_y + geom.track_h {
            return None;
        }
        let hit = if y >= geom.thumb_y && y <= geom.thumb_y + geom.thumb_h {
            ScrollbarHit::Thumb(y - geom.thumb_y)
        } else {
            ScrollbarHit::Track
        };
        Some((geom, hit))
    }

    /// Expands the scrollbar while the pointer is over it. Redraws only when
    /// the hover state changes.
    fn set_scrollbar_hover(&mut self, hover: bool) {
        if self.scrollbar_hover == hover {
            return;
        }
        self.scrollbar_hover = hover;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn scroll_to_thumb(&mut self, thumb_top: f32) {
        let Some(geom) = self.scrollbar_geometry(true) else {
            return;
        };
        let Some(history) = self.terminal.as_ref().map(Terminal::scrollable_history) else {
            return;
        };
        let current = self
            .terminal
            .as_ref()
            .map(Terminal::display_offset)
            .unwrap_or(0);
        let target = scrollbar::offset_for_thumb_top(thumb_top, &geom, history);
        self.scroll_local(target as i32 - current as i32);
    }

    /// Scrolls the viewport locally by `lines` (positive scrolls back into
    /// history). Returns whether the offset changed.
    fn scroll_local(&mut self, lines: i32) -> bool {
        let Some(terminal) = self.terminal.as_mut() else {
            return false;
        };
        if lines == 0 || terminal.scrollable_history() == 0 {
            return false;
        }
        let before = terminal.display_offset();
        terminal.scroll_display(lines);
        if terminal.display_offset() == before {
            return false;
        }
        // Selection positions are logical viewport offsets; scrolling makes them stale.
        self.selection = None;
        self.mouse_down = false;
        // The wheel or a key may bring the scrollbar under a parked pointer.
        let (x, y) = self.cursor_pos;
        let hover = self.scrollbar_drag || self.scrollbar_hit(x, y).is_some();
        self.set_scrollbar_hover(hover);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        true
    }

    /// Rows moved by a page key or a track click.
    fn page_lines(&self) -> i32 {
        self.rows.saturating_sub(1).max(1) as i32
    }

    fn send_mouse(&mut self, kind: input::MouseEventKind, button: Option<input::MouseButton>) {
        let Some(renderer) = &self.renderer else {
            return;
        };
        let (row, column) = cell_at(
            self.cursor_pos.0,
            self.cursor_pos.1,
            renderer.padding() as f64,
            renderer.fonts().cell_w as f64,
            renderer.fonts().cell_h as f64,
            self.cols,
            self.rows,
        );
        let event = input::MouseEvent {
            kind,
            button,
            column,
            row,
            mods: mods_from(self.modifiers),
        };
        if let Some(bytes) = input::encode_mouse(event, self.app_mouse_mode()) {
            self.write_input(&bytes);
        }
    }

    fn pos_at(&self, x: f64, y: f64) -> Option<LogicalPos> {
        let renderer = self.renderer.as_ref()?;
        let terminal = self.terminal.as_ref()?;
        let (row, visual) = cell_at(
            x,
            y,
            renderer.padding() as f64,
            renderer.fonts().cell_w as f64,
            renderer.fonts().cell_h as f64,
            self.cols,
            self.rows,
        );
        let snapshot = terminal.snapshot();
        let line = snapshot.lines.get(row)?;
        Some(logical_pos_at(row, visual, line, &self.config.bidi))
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
        let snapshot = terminal.snapshot();
        let text = match selection.mode {
            SelectionMode::Simple => {
                selection.to_text(&snapshot, self.config.selection.trim_trailing)
            }
            SelectionMode::Visual | SelectionMode::Block => {
                let maps: Vec<VisualMap> = snapshot
                    .lines
                    .iter()
                    .map(|line| VisualMap::for_row(line, &self.config.bidi))
                    .collect();
                if selection.mode == SelectionMode::Block {
                    selection.to_text_block(&snapshot, &maps, self.config.selection.trim_trailing)
                } else {
                    selection.to_text_visual(&snapshot, &maps, self.config.selection.trim_trailing)
                }
            }
        };
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
                if let Some(terminal) = self.terminal.as_mut() {
                    if terminal.feed(&bytes) {
                        // Rows moved, so row-anchored selection positions are
                        // stale. An in-place repaint keeps the selection.
                        self.selection = None;
                        self.mouse_down = false;
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            UserEvent::Title(title) => {
                if !self.config.window.dynamic_title {
                    return;
                }
                if let Some(window) = &self.window {
                    if title.is_empty() {
                        // `OSC 0;` / `OSC 2;` with no payload restores the
                        // default title rather than blanking the window.
                        window.set_title(DEFAULT_TITLE);
                    } else {
                        window.set_title(&title);
                    }
                }
            }
            UserEvent::ResetTitle => {
                if let Some(window) = &self.window {
                    window.set_title(DEFAULT_TITLE);
                }
            }
            UserEvent::ClipboardStore(text) => {
                if text.len() > MAX_OSC52_BYTES {
                    tracing::warn!(
                        limit = MAX_OSC52_BYTES,
                        len = text.len(),
                        "OSC 52 clipboard store rejected"
                    );
                    return;
                }
                let Some(clipboard) = self.clipboard() else {
                    return;
                };
                if let Err(err) = clipboard.set_text(text) {
                    tracing::warn!(%err, "OSC 52 clipboard store failed");
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
            WindowEvent::CursorLeft { .. } => self.set_scrollbar_hover(false),
            WindowEvent::MouseInput { state, button, .. } => self.on_mouse(state, button),
            WindowEvent::Focused(focused) => {
                if !focused {
                    // A pressed button may never get its release after focus
                    // loss; balance the application and end local drags.
                    if let Some(button) = self.app_mouse_button.take() {
                        self.send_mouse(input::MouseEventKind::Release, Some(button));
                    }
                    self.mouse_down = false;
                    self.scrollbar_drag = false;
                    let (x, y) = self.cursor_pos;
                    self.set_scrollbar_hover(self.scrollbar_hit(x, y).is_some());
                }
                if self.app_mouse_mode().contains(TermMode::FOCUS_IN_OUT) {
                    self.write_input(&input::encode_focus(focused));
                }
            }
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

/// Converts the configured font size from logical pixels to the physical
/// pixels the renderer draws with. Without this the text is too small on
/// high-DPI displays, because winit reports window sizes in physical pixels.
fn scaled_font_size(size: f32, scale_factor: f64) -> f32 {
    (size as f64 * scale_factor) as f32
}

fn grid_size(width: u32, height: u32, cell_w: u32, cell_h: u32, padding: u32) -> (usize, usize) {
    // The scrollbar lives in a reserved gutter so its hovered width never
    // covers the last text column.
    let gutter = scrollbar::GUTTER.ceil() as u32;
    let usable_w = width.saturating_sub(padding.saturating_mul(2).saturating_add(gutter));
    let usable_h = height.saturating_sub(padding.saturating_mul(2));
    let cols = (usable_w / cell_w.max(1)).max(20) as usize;
    let rows = (usable_h / cell_h.max(1)).max(5) as usize;
    (cols, rows)
}

/// Caps the configured startup size so the window fits the primary monitor
/// (90% of its width, 80% of its height).
fn initial_window_size(width: u32, height: u32, monitor: Option<(u32, u32)>) -> (u32, u32) {
    let Some((monitor_w, monitor_h)) = monitor else {
        return (width, height);
    };
    let max_w = monitor_w.saturating_mul(9) / 10;
    let max_h = monitor_h.saturating_mul(4) / 5;
    (width.min(max_w), height.min(max_h))
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
        Key::Named(NamedKey::F1) => Some(KeyInput::Function(1)),
        Key::Named(NamedKey::F2) => Some(KeyInput::Function(2)),
        Key::Named(NamedKey::F3) => Some(KeyInput::Function(3)),
        Key::Named(NamedKey::F4) => Some(KeyInput::Function(4)),
        Key::Named(NamedKey::F5) => Some(KeyInput::Function(5)),
        Key::Named(NamedKey::F6) => Some(KeyInput::Function(6)),
        Key::Named(NamedKey::F7) => Some(KeyInput::Function(7)),
        Key::Named(NamedKey::F8) => Some(KeyInput::Function(8)),
        Key::Named(NamedKey::F9) => Some(KeyInput::Function(9)),
        Key::Named(NamedKey::F10) => Some(KeyInput::Function(10)),
        Key::Named(NamedKey::F11) => Some(KeyInput::Function(11)),
        Key::Named(NamedKey::F12) => Some(KeyInput::Function(12)),
        _ => None,
    }
}

fn map_mouse_button(button: MouseButton) -> Option<input::MouseButton> {
    match button {
        MouseButton::Left => Some(input::MouseButton::Left),
        MouseButton::Middle => Some(input::MouseButton::Middle),
        MouseButton::Right => Some(input::MouseButton::Right),
        _ => None,
    }
}

/// A press is forwarded only when the application reports the mouse and Shift
/// is not held. Shift is the documented escape hatch for local selection.
fn forwards_mouse_press(mode: TermMode, shift: bool) -> bool {
    mode.intersects(TermMode::MOUSE_MODE) && !shift
}

/// A release is forwarded only when the matching press was forwarded. Using the
/// tracked button rather than the live mode or Shift keeps a mid-gesture change
/// from losing or inventing a release.
fn release_button(tracked: Option<input::MouseButton>) -> Option<input::MouseButton> {
    tracked
}

/// Returns 1, 2, or 3 for consecutive clicks within 400 ms and 4 px. A fourth
/// consecutive click starts a new cycle at 1.
fn next_click_count(
    previous: Option<(Instant, f64, f64)>,
    previous_count: u8,
    now: Instant,
    x: f64,
    y: f64,
) -> u8 {
    let repeated = previous.is_some_and(|(then, then_x, then_y)| {
        now.saturating_duration_since(then) <= CLICK_TIMEOUT
            && (x - then_x).abs() <= CLICK_DISTANCE
            && (y - then_y).abs() <= CLICK_DISTANCE
    });
    if !repeated {
        return 1;
    }
    match previous_count {
        1 => 2,
        2 => 3,
        _ => 1,
    }
}

/// Copy-on-select copies when the gesture covered more than one cell, or when
/// it was a double/triple click whose word or line happens to be one cell.
fn should_copy_on_release(click_count: u8, collapsed: bool) -> bool {
    click_count >= 2 || !collapsed
}

fn mods_from(modifiers: ModifiersState) -> Mods {
    Mods {
        ctrl: modifiers.control_key(),
        alt: modifiers.alt_key(),
        shift: modifiers.shift_key(),
    }
}

fn logical_pos_at(row: usize, visual: usize, line: &RowSnapshot, bidi: &BidiConfig) -> LogicalPos {
    LogicalPos {
        row,
        col: VisualMap::for_row(line, bidi).logical_at(visual),
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

    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};

    #[test]
    fn grid_size_uses_padding_and_minimums() {
        assert_eq!(grid_size(1200, 800, 10, 20, 4), (117, 39));
        assert_eq!(grid_size(100, 100, 10, 20, 4), (20, 5));
    }

    #[test]
    fn grid_size_reserves_the_scrollbar_gutter() {
        let usable = 1200 - 4 * 2;
        assert_eq!(
            grid_size(1200, 800, 1, 20, 4).0,
            usable - scrollbar::GUTTER.ceil() as usize
        );
    }

    #[test]
    fn font_size_tracks_display_scale_factor() {
        assert_eq!(scaled_font_size(17.0, 1.0), 17.0);
        assert_eq!(scaled_font_size(17.0, 1.25), 21.25);
        assert_eq!(scaled_font_size(17.0, 1.5), 25.5);
    }

    #[test]
    fn initial_window_size_fits_the_primary_monitor() {
        assert_eq!(
            initial_window_size(1440, 800, Some((1600, 900))),
            (1440, 720)
        );
    }

    #[test]
    fn initial_window_size_keeps_config_without_a_monitor() {
        assert_eq!(initial_window_size(1200, 800, None), (1200, 800));
    }

    #[test]
    fn initial_window_size_fits_a_small_monitor() {
        assert_eq!(
            initial_window_size(1440, 800, Some((1024, 768))),
            (921, 614)
        );
    }

    #[test]
    fn map_key_recognizes_text_and_named_keys() {
        assert_eq!(
            map_key(&Key::Character("a".into())),
            Some(KeyInput::Char('a'))
        );
        assert_eq!(map_key(&Key::Named(NamedKey::Enter)), Some(KeyInput::Enter));
        assert_eq!(map_key(&Key::Named(NamedKey::CapsLock)), None);
    }

    #[test]
    fn modifiers_map_to_ctrl_and_alt() {
        let mods = mods_from(ModifiersState::CONTROL | ModifiersState::ALT);
        assert!(mods.ctrl);
        assert!(mods.alt);
    }

    #[test]
    fn modifiers_map_include_shift() {
        let mods = mods_from(ModifiersState::SHIFT);
        assert!(mods.shift);
        assert!(!mods.ctrl);
        assert!(!mods.alt);
    }

    #[test]
    fn map_key_recognizes_function_keys() {
        assert_eq!(
            map_key(&Key::Named(NamedKey::F1)),
            Some(KeyInput::Function(1))
        );
        assert_eq!(
            map_key(&Key::Named(NamedKey::F12)),
            Some(KeyInput::Function(12))
        );
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

    #[test]
    fn mouse_press_forwards_only_in_reporting_modes_without_shift() {
        for mode in [
            TermMode::MOUSE_REPORT_CLICK,
            TermMode::MOUSE_DRAG,
            TermMode::MOUSE_MOTION,
        ] {
            assert!(forwards_mouse_press(mode, false));
            assert!(!forwards_mouse_press(mode, true));
        }
        assert!(!forwards_mouse_press(TermMode::empty(), false));
        assert!(!forwards_mouse_press(TermMode::SGR_MOUSE, false));
        assert!(!forwards_mouse_press(TermMode::ALT_SCREEN, false));
    }

    #[test]
    fn local_release_after_local_press_is_not_forwarded() {
        // Local gestures never record a tracked app button, so the release has
        // nothing to report even if a mouse mode is active.
        assert_eq!(release_button(None), None);
        assert_eq!(
            release_button(Some(input::MouseButton::Left)),
            Some(input::MouseButton::Left)
        );
    }

    fn bidi_line() -> RowSnapshot {
        RowSnapshot {
            cells: ["a", "b", "c", "م", "ر", "ح"]
                .iter()
                .map(|text| Cell {
                    text: (*text).to_string(),
                    width: 1,
                    style: Style::default(),
                })
                .collect(),
            wrapped: false,
        }
    }

    #[test]
    fn logical_position_uses_the_row_bidi_map() {
        let line = bidi_line();
        let bidi = BidiConfig {
            enabled: true,
            base_direction: BaseDirection::Ltr,
        };
        assert_eq!(
            logical_pos_at(2, 5, &line, &bidi),
            LogicalPos { row: 2, col: 3 }
        );
    }

    #[test]
    fn disabled_bidi_hit_tests_identity() {
        let line = bidi_line();
        let bidi = BidiConfig {
            enabled: false,
            base_direction: BaseDirection::Ltr,
        };
        assert_eq!(
            logical_pos_at(0, 5, &line, &bidi),
            LogicalPos { row: 0, col: 5 }
        );
    }

    #[test]
    fn first_click_counts_as_one() {
        assert_eq!(next_click_count(None, 0, Instant::now(), 10.0, 10.0), 1);
    }

    #[test]
    fn nearby_clicks_within_the_timeout_increase_the_count() {
        let now = Instant::now();
        let previous = Some((now, 10.0, 10.0));
        assert_eq!(
            next_click_count(previous, 1, now + Duration::from_millis(200), 12.0, 11.0),
            2
        );
        assert_eq!(
            next_click_count(previous, 2, now + Duration::from_millis(200), 12.0, 11.0),
            3
        );
    }

    #[test]
    fn fourth_click_restarts_the_cycle() {
        let now = Instant::now();
        let previous = Some((now, 10.0, 10.0));
        assert_eq!(
            next_click_count(previous, 3, now + Duration::from_millis(200), 10.0, 10.0),
            1
        );
    }

    #[test]
    fn distant_click_starts_a_new_cycle() {
        let now = Instant::now();
        let previous = Some((now, 10.0, 10.0));
        assert_eq!(
            next_click_count(previous, 1, now + Duration::from_millis(100), 20.0, 10.0),
            1
        );
    }

    #[test]
    fn slow_click_starts_a_new_cycle() {
        let now = Instant::now();
        let previous = Some((now, 10.0, 10.0));
        assert_eq!(
            next_click_count(previous, 1, now + Duration::from_millis(500), 10.0, 10.0),
            1
        );
    }

    #[test]
    fn collapsed_single_click_does_not_copy() {
        assert!(!should_copy_on_release(1, true));
    }

    #[test]
    fn collapsed_double_click_copies() {
        assert!(should_copy_on_release(2, true));
    }
}
