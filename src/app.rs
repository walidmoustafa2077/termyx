//! Window, surface, and event wiring.

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};

use alacritty_terminal::term::TermMode;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowId};

use crate::config::{BidiConfig, Config};
use crate::event::UserEvent;
use crate::fonts::{FontSet, UI_SIZE_PX};
use crate::icon;
use crate::input::{self, KeyInput, Mods};
use crate::platform;
use crate::render::Renderer;
use crate::scrollbar::{self, ScrollbarGeometry, ScrollbarHit};
use crate::selection::{line_bounds, word_bounds, Selection, SelectionMode};
use crate::session::{spawn_session, SessionId, TabSet, FALLBACK_TITLE};
use crate::term::{LogicalPos, RowSnapshot, Terminal};
use crate::theme::Theme;
use crate::ui::menu::{MenuAction, MenuItem, MenuLayout};
use crate::ui::tab_bar::{StripHover, TabBarLayout, TabHit};
use crate::visual::VisualMap;

/// Upper bound on a single paste, in Unicode scalar values. A larger clipboard
/// would freeze the UI thread while writing to a shell that may not drain it.
const MAX_PASTE_CHARS: usize = 1_048_576;

/// Upper bound on an OSC 52 payload, in bytes. The VT parser accumulates the
/// whole sequence before the UI thread writes it to the clipboard.
const MAX_OSC52_BYTES: usize = 100 * 1024;

/// Longest delay between two clicks of one multi-click gesture.
const CLICK_TIMEOUT: Duration = Duration::from_millis(400);

/// Largest cursor movement between two clicks of one multi-click gesture.
const CLICK_DISTANCE: f64 = 4.0;

/// How long the cursor stays visible or hidden while blinking.
const BLINK_INTERVAL: Duration = Duration::from_millis(500);

pub struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    renderer: Option<Renderer>,
    /// The open shells. Each session owns its parser, its PTY child, and its
    /// reader thread; the window shows whichever session is active.
    tabs: TabSet,
    /// Strip geometry from the window width and every tab title. `None` until
    /// the first session exists.
    layout: Option<TabBarLayout>,
    /// The open `⌄` dropdown, if any. While it is `Some` the menu owns the
    /// pointer: presses, motion, the wheel, and Escape all route to it.
    menu: Option<MenuLayout>,
    /// What the pointer is over in the strip, painted as hover. The `+` and `⌄`
    /// buttons highlight too; empty band paints nothing.
    strip_hover: Option<StripHover>,
    clipboard: Option<arboard::Clipboard>,
    modifiers: ModifiersState,
    selection: Option<Selection>,
    mouse_down: bool,
    // The press starting the current gesture belonged to window chrome (the
    // tab strip or the menu), so its release must not copy a selection or
    // otherwise reach the grid.
    press_in_chrome: bool,
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
    blink: CursorBlink,
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
            tabs: TabSet::new(),
            layout: None,
            menu: None,
            strip_hover: None,
            clipboard: None,
            modifiers: ModifiersState::empty(),
            selection: None,
            mouse_down: false,
            press_in_chrome: false,
            click_count: 0,
            last_click: None,
            app_mouse_button: None,
            wheel_remainder: 0.0,
            scrollbar_drag: false,
            scrollbar_grab: 0.0,
            scrollbar_hover: false,
            blink: CursorBlink::idle(),
            // Off the strip and outside the window until the first CursorMoved:
            // (0, 0) is inside the strip band, so a press arriving first would
            // be eaten as a chrome press.
            cursor_pos: (-1.0, -1.0),
            cols: 80,
            rows: 24,
        }
    }

    /// Pixels reserved above the grid for the tab strip.
    ///
    /// `TAB_STRIP_HEIGHT` is logical while the renderer, the surface, and the
    /// mouse coordinates are physical, so the window's scale factor is applied
    /// here and nowhere else.
    fn tab_strip_px(&self) -> u32 {
        strip_px(self.scale_factor())
    }

    /// The window's scale factor, or 1.0 before the window exists.
    fn scale_factor(&self) -> f64 {
        self.window
            .as_ref()
            .map_or(1.0, |window| window.scale_factor())
    }

    /// Rebuilds the strip geometry. Called after a resize, a tab change, or a
    /// retitle, because a tab's width depends on every title in the strip.
    ///
    /// Both inputs are physical: the window width, and the strip height the
    /// renderer actually reserved above the grid (`top_inset`). Taking the
    /// height from the field the grid uses keeps the painted band and the
    /// reserved inset the same number after a DPI change.
    ///
    /// The hover is re-derived here because every layout change is the one
    /// point where the tab under a stationary pointer can become a different
    /// tab. Leaving the old index would paint a `✕` on the wrong tab and let a
    /// click close a shell whose button was never shown.
    fn rebuild_layout(&mut self) {
        let Some(renderer) = self.renderer.as_ref() else {
            return;
        };
        let strip_h = renderer.top_inset() as f32;
        let Some(window) = self.window.as_ref() else {
            return;
        };
        let width = window.inner_size().width as f32;
        let ui_px = UI_SIZE_PX * window.scale_factor() as f32;
        let titles = self.tabs.titles();
        let fonts = renderer.fonts();
        let layout = TabBarLayout::new(width, strip_h, &titles, |title| {
            fonts.ui_width(title, ui_px)
        });
        self.layout = Some(layout);
        // A keyboard switch or a new tab must rename the OS window, which an
        // OSC title alone cannot do once the opt-in is off. The layout rebuild
        // is the one place every tab change passes through. winit has no title
        // getter, so every resize re-applies it rather than comparing first.
        let active_title = self
            .tabs
            .active_index()
            .and_then(|index| titles.get(index).copied());
        window.set_title(&window_title(
            self.config.window.dynamic_title,
            active_title,
        ));
        if self.menu.is_some() {
            // The dropdown hangs off the chevron, which moves with the tabs and
            // the window width; re-anchor an open menu instead of leaving it
            // pointing at where the button used to be.
            self.open_menu();
        }
        self.refresh_strip_hover();
    }

    /// Opens the dropdown under the `⌄`, or closes it when it is already open.
    fn toggle_menu(&mut self) {
        if self.menu.is_some() {
            self.close_menu();
        } else {
            self.open_menu();
        }
    }

    /// Builds the dropdown from the live profile list and anchors it under the
    /// chevron. Also called to re-anchor an open menu after the strip moves.
    fn open_menu(&mut self) {
        let Some(layout) = self.layout.as_ref() else {
            return;
        };
        let anchor = layout.chevron;
        let (width, height) = self.window.as_ref().map_or((0.0, 0.0), |window| {
            let size = window.inner_size();
            (size.width as f32, size.height as f32)
        });
        let mut menu = MenuLayout::new(anchor, menu_items(&self.config), width, height);
        // Opening through the chevron puts the pointer outside the panel, but a
        // re-anchor can happen with the pointer already over a row.
        menu.hovered = menu.hit(self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
        self.menu = Some(menu);
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn close_menu(&mut self) {
        if self.menu.take().is_some() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
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
            .with_title(FALLBACK_TITLE)
            .with_inner_size(LogicalSize::new(width, height));
        let attributes = match icon::window_icon() {
            Ok(icon) => attributes.with_window_icon(Some(icon)),
            Err(err) => {
                tracing::warn!("window icon unavailable: {err:#}");
                attributes
            }
        };
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
        // Stored before the rest of the setup because `tab_strip_px` reads the
        // window's scale factor; every step below only needs the window it
        // already holds a handle to.
        self.window = Some(window.clone());

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
        renderer.set_top_inset(self.tab_strip_px());

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
            renderer.top_inset(),
        );

        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            if let Err(err) = surface.resize(w, h) {
                tracing::warn!(%err, "initial surface resize failed");
            }
        }

        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.cols = cols;
        self.rows = rows;

        // The first tab opens the config's default profile, so a bare launch
        // and a new tab started from the `+` button agree.
        let profile = crate::profiles::default_profile_index(&self.config);
        self.new_tab(profile)?;

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

    /// Rebuilds the grid for the new surface size. Every session is resized so
    /// a background tab does not keep a stale width for its shell. Returns
    /// whether the grid actually changed and therefore reflowed.
    fn resize_grid(&mut self, width: u32, height: u32) -> bool {
        let Some(renderer) = self.renderer.as_ref() else {
            return false;
        };
        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
            renderer.top_inset(),
        );
        if cols == self.cols && rows == self.rows {
            return false;
        }
        for session in self.tabs.iter_mut() {
            session.terminal.resize(cols, rows);
            if let Err(err) = session.pty.resize(cols as u16, rows as u16) {
                tracing::warn!(%err, "pty resize failed");
            }
        }
        self.cols = cols;
        self.rows = rows;
        true
    }

    /// Applies a session title to the window, keeping the `dynamic_title`
    /// opt-in. A blank title restores the default.
    fn apply_title(&self, title: &str) {
        let Some(window) = &self.window else {
            return;
        };
        if !self.config.window.dynamic_title {
            return;
        }
        let title = title.trim();
        window.set_title(if title.is_empty() {
            FALLBACK_TITLE
        } else {
            title
        });
    }

    /// Opens `profile` in a new tab and shows it.
    ///
    /// A failed spawn leaves the window as it was; the caller decides whether
    /// that is fatal (the first tab) or a warning (a later click).
    fn new_tab(&mut self, profile: usize) -> anyhow::Result<()> {
        let profiles = crate::profiles::profiles(&self.config);
        let Some(profile) = profiles.get(profile).or_else(|| profiles.first()) else {
            anyhow::bail!("no shell profiles configured");
        };
        tracing::debug!(profile = %profile.name, "opening a tab");
        let id = self.tabs.next_id();
        let session = spawn_session(
            id,
            &profile.shell,
            self.cols,
            self.rows,
            self.config.scrollback.lines,
            self.proxy.clone(),
        )?;
        // A forwarded press belongs to the tab that owns it. Balance it before
        // the new session becomes active, or the release would reach the wrong
        // shell and leave the original one stuck down.
        self.release_forwarded_button();
        self.tabs.push(session);
        // The tab is named for the profile until the shell names itself.
        self.tabs.set_title(id, &profile.name);
        // The new tab is active, so a selection on the old grid is stale.
        self.selection = None;
        self.mouse_down = false;
        self.rebuild_layout();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    /// Closes the tab at `index` and shows the neighbour that takes its place
    /// only when the closed tab was the one on screen.
    ///
    /// Dropping the session closes its child through `PtySession::drop`.
    /// Returns whether the set is now empty: a window with no tab has nothing
    /// left to show, so the caller exits.
    fn close_tab(&mut self, index: usize) -> bool {
        // The session that owns a forwarded button is about to be dropped, so
        // no release could ever reach it: balance the press first.
        self.release_forwarded_button();
        let _ = self.tabs.remove(index);
        self.selection = None;
        // The drag that was extending a selection in this tab has nothing left
        // to extend.
        self.mouse_down = false;
        // The tab under the pointer may have moved with the removal.
        self.rebuild_layout();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        self.tabs.is_empty()
    }

    /// Re-derives the strip hover from the pointer and the current layout.
    ///
    /// A layout change can move every tab under a stationary pointer (closing,
    /// opening, retitling or resizing), so the hover from before the change may
    /// name a tab that no longer sits there. Re-deriving keeps the painted `✕`
    /// and the next press on the same target: without it, a click at an
    /// unchanged pixel could close a tab whose `✕` was not visible when the
    /// click landed.
    fn refresh_strip_hover(&mut self) {
        let hover = self.layout.as_ref().and_then(|layout| {
            hover_after_layout_change(layout, self.cursor_pos.0, self.cursor_pos.1)
        });
        self.set_strip_hover(hover);
    }

    /// The strip action for a physical-pixel position, or `None` when the
    /// pointer is outside the band and the grid owns the event.
    ///
    /// The layout is built from physical pixels (window width and the height
    /// the renderer reserved), and winit reports mouse positions in that same
    /// space, so the hit test needs no scaling here.
    fn strip_action_at(&self, x: f64, y: f64) -> Option<StripAction> {
        let layout = self.layout.as_ref()?;
        if !in_strip(y, layout.strip_h) {
            return None;
        }
        Some(strip_action(layout.hit(x as f32, y as f32)))
    }

    /// Paints `hover` as the target under the pointer, redrawing only when it
    /// actually changes.
    fn set_strip_hover(&mut self, hover: Option<StripHover>) {
        if self.strip_hover == hover {
            return;
        }
        self.strip_hover = hover;
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// The parser of the tab the window shows.
    fn active_terminal(&mut self) -> Option<&mut Terminal> {
        Some(&mut self.tabs.active_mut()?.terminal)
    }

    fn render(&mut self) {
        let scrollbar_hover = self.scrollbar_hover || self.scrollbar_drag;
        let ui_px = UI_SIZE_PX * self.scale_factor() as f32;
        let Some(session) = self.tabs.active() else {
            return;
        };
        let Some(active) = self.tabs.active_index() else {
            return;
        };
        let (Some(renderer), Some(surface)) = (&mut self.renderer, &mut self.surface) else {
            return;
        };
        let mut snapshot = session.terminal.snapshot();
        // A blinking cursor is hidden on the off phase. Repainting the whole
        // frame is acceptable while damage tracking is out of scope.
        let blinking =
            snapshot.cursor.blinking && snapshot.cursor.visible && snapshot.display_offset == 0;
        if blinking {
            self.blink.start(Instant::now());
            if !self.blink.visible {
                snapshot.cursor.visible = false;
            }
        } else {
            self.blink.stop();
        }
        let maps: Vec<VisualMap> = snapshot
            .lines
            .iter()
            .map(|line| VisualMap::for_row(line, &self.config.bidi))
            .collect();
        renderer.set_scrollbar_hover(scrollbar_hover);
        renderer.draw(&snapshot, &maps, self.selection.as_ref());
        if let Some(layout) = self.layout.as_ref() {
            renderer.draw_chrome(layout, ui_px, active, self.strip_hover);
        }
        if let Some(menu) = self.menu.as_ref() {
            renderer.draw_menu(menu, ui_px);
        }
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

    /// Handles a key press. Returns whether the window should exit: closing the
    /// last tab leaves a terminal with nothing to show.
    fn on_key(&mut self, event: KeyEvent) -> bool {
        // Typing always reveals the cursor, so a keystroke is never typed into
        // a cursor that happens to be in its hidden phase.
        self.blink.restart(Instant::now());
        // The dropdown is a layer over the grid: Escape dismisses it rather
        // than reaching the shell.
        if self.menu.is_some() && matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
            self.close_menu();
            return false;
        }
        // The tab shortcuts are chrome, not text: a key they claim must never
        // also reach the shell.
        if let Some(action) = tab_shortcut(&event, self.modifiers) {
            // Only cycling may auto-repeat: holding the new or close shortcut
            // would otherwise open or close a tab per key repeat.
            if event.repeat && !matches!(action, TabAction::Next | TabAction::Previous) {
                return false;
            }
            return self.apply_tab_shortcut(action);
        }
        if self.modifiers.control_key() && self.modifiers.shift_key() {
            if let Key::Character(text) = &event.logical_key {
                match text.as_str() {
                    "c" | "C" => {
                        self.copy_selection();
                        return false;
                    }
                    "v" | "V" => {
                        if !event.repeat {
                            self.paste_clipboard();
                        }
                        return false;
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
                    .tabs
                    .active()
                    .is_some_and(|session| session.terminal.scrollable_history() > 0);
                if has_history {
                    self.scroll_local(lines);
                    return false;
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
                return false;
            };
            key
        };
        let mode = self
            .tabs
            .active()
            .map(|session| session.terminal.mode())
            .unwrap_or_else(TermMode::empty);
        let Some(bytes) = input::encode(key, mods_from(self.modifiers), mode) else {
            return false;
        };
        self.write_input(&bytes);
        false
    }

    /// Runs a tab shortcut. Returns whether the last tab is gone, which leaves
    /// the window with nothing to show.
    fn apply_tab_shortcut(&mut self, action: TabAction) -> bool {
        match action {
            TabAction::Next => self.cycle_tab(1),
            TabAction::Previous => self.cycle_tab(-1),
            TabAction::NewTab => {
                let index = crate::profiles::default_profile_index(&self.config);
                if let Err(err) = self.new_tab(index) {
                    tracing::warn!(%err, "could not open a tab");
                }
            }
            TabAction::CloseTab => {
                if let Some(index) = self.tabs.active_index() {
                    return self.close_tab(index);
                }
            }
            TabAction::SelectProfile(index) => self.open_profile(index),
        }
        false
    }

    /// Opens the profile a numbered shortcut names.
    ///
    /// A number past the end of the configured list opens nothing. Falling
    /// through to `new_tab`, which substitutes the first profile, would start a
    /// shell the user never asked for.
    fn open_profile(&mut self, index: usize) {
        if index >= crate::profiles::profiles(&self.config).len() {
            tracing::debug!(profile = index, "no profile for this shortcut");
            return;
        }
        if let Err(err) = self.new_tab(index) {
            tracing::warn!(%err, "could not open a profile");
        }
    }

    /// Runs a row of the `⌄` dropdown. The caller closes the menu first, so an
    /// action that opens a shell or a dialog starts from a clean strip.
    fn run_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Profile(index) => {
                if let Err(err) = self.new_tab(index) {
                    tracing::warn!(%err, "could not open a profile");
                }
            }
            MenuAction::Settings => self.open_settings(),
            MenuAction::CommandPalette => {
                tracing::info!("the command palette arrives in a later task");
            }
            MenuAction::About => {
                // `show()` blocks until the dialog closes. Run on the UI thread
                // it would stop the event loop and the window would freeze, and
                // Windows may not even paint the dialog. Building the whole
                // dialog in the worker keeps every non-`Send` value on that
                // thread; the handle is dropped right away, so the dialog
                // outlives this press without being joined.
                let spawned = std::thread::Builder::new()
                    .name("about-dialog".into())
                    .spawn(|| {
                        rfd::MessageDialog::new()
                            .set_title(FALLBACK_TITLE)
                            .set_description(about_text())
                            .set_level(rfd::MessageLevel::Info)
                            .show();
                    });
                if let Err(err) = spawned {
                    tracing::warn!(%err, "could not open the About dialog");
                }
            }
        }
    }

    /// Opens the config file, or the folder it belongs in when it does not
    /// exist yet.
    ///
    /// A terminal runs fine without a config file, so `Settings` on a first run
    /// is normal, not an error. Revealing the folder says where the file goes
    /// without inventing one; a failure at either step is logged, never shown
    /// as a dialog.
    fn open_settings(&self) {
        let path = crate::config::default_config_path();
        if path.exists() {
            if let Err(err) = platform::open_path(&path) {
                tracing::warn!(%err, "could not open the config file");
            }
            return;
        }
        let folder = path.parent().unwrap_or(&path);
        if let Err(err) = std::fs::create_dir_all(folder) {
            tracing::warn!(%err, "could not create the config folder");
        }
        tracing::info!(path = %path.display(), "no config file yet; opening its folder");
        if let Err(err) = platform::open_path(folder) {
            tracing::warn!(%err, "could not open the config folder");
        }
    }

    /// Shows the tab `delta` places from the active one, wrapping at both ends.
    /// A lone tab has nowhere to go, so the strip is left alone.
    fn cycle_tab(&mut self, delta: isize) {
        let Some(current) = self.tabs.active_index() else {
            return;
        };
        let Some(next) = cycle_index(current, delta, self.tabs.len()) else {
            return;
        };
        // The tab being hidden must release the button it was forwarded before
        // the new session becomes the one keyboard input reaches.
        self.release_forwarded_button();
        self.tabs.select(next);
        // The keyboard switch paints the tab the same way a click does.
        self.selection = None;
        self.rebuild_layout();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Writes text to the shell through the shared PTY path.
    fn send_text(&mut self, text: &str) {
        self.write_input(text.as_bytes());
    }

    fn write_input(&mut self, bytes: &[u8]) {
        let Some(session) = self.tabs.active_mut() else {
            return;
        };
        if let Err(err) = session.pty.write(bytes) {
            tracing::warn!(%err, "pty write failed");
        }
    }

    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_pos = (x, y);
        if let Some(menu) = self.menu.as_mut() {
            // The dropdown owns the pointer while it is open: motion picks the
            // row under it and nothing beneath the panel reacts.
            let hovered = menu.hit(x as f32, y as f32);
            if menu.hovered != hovered {
                menu.hovered = hovered;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            return;
        }
        // A tab body, its `✕`, and the two buttons all highlight.
        let hover = strip_hover(self.strip_action_at(x, y));
        self.set_strip_hover(hover);
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
        // The dropdown owns the pointer while it is open; the wheel must not
        // scroll the grid under the panel.
        if self.menu.is_some() {
            return;
        }
        // The strip is chrome: a wheel over it must not scroll the grid or
        // reach a mouse-reporting application.
        if self
            .strip_action_at(self.cursor_pos.0, self.cursor_pos.1)
            .is_some()
        {
            return;
        }
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

    /// Handles a mouse button. Returns whether the window should exit: closing
    /// the last tab leaves a terminal with nothing to show.
    fn on_mouse(&mut self, state: ElementState, button: MouseButton) -> bool {
        let Some(input_button) = map_mouse_button(button) else {
            return false;
        };
        self.blink.restart(Instant::now());

        if state == ElementState::Pressed {
            // With the dropdown open it owns the pointer: a press on a row is
            // consumed (running its action is a later task), and a press
            // anywhere else dismisses the panel. Neither reaches the strip or
            // the grid.
            if let Some(menu) = self.menu.as_ref() {
                let (x, y) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
                let press = menu_press(menu.hit(x, y), menu.rect.contains(x, y));
                let action = menu_choice(press, button)
                    .and_then(|index| menu.items.get(index).map(|item| item.action));
                // A forwarded button belongs to the tab that owns it; balance
                // it before the menu takes over the pointer.
                self.release_forwarded_button();
                // The press is chrome's, so its release cannot copy a stale
                // selection, and it starts no grid click chain.
                self.mouse_down = false;
                self.click_count = 0;
                self.last_click = None;
                self.press_in_chrome = true;
                self.close_menu();
                if let Some(action) = action {
                    self.run_menu_action(action);
                }
                return false;
            }
            // The whole band owns its presses, not just the controls: a miss on
            // a tab must not paste, select, reach the PTY, or grab the
            // scrollbar, whichever button was pressed.
            if let Some(action) = self.strip_action_at(self.cursor_pos.0, self.cursor_pos.1) {
                // A forwarded button belongs to the tab that owns it; balance
                // it before the strip takes over the pointer.
                self.release_forwarded_button();
                // The band starts a click chain of its own, so the next grid
                // click cannot pair with this one as a double click.
                self.mouse_down = false;
                self.click_count = 0;
                self.last_click = None;
                // The release that ends this press is the chrome's too.
                self.press_in_chrome = true;
                if button == MouseButton::Left {
                    match action {
                        StripAction::Select(index) => {
                            self.tabs.select(index);
                            self.selection = None;
                            self.rebuild_layout();
                        }
                        StripAction::Close(index) => return self.close_tab(index),
                        StripAction::NewTab => {
                            let index = crate::profiles::default_profile_index(&self.config);
                            if let Err(err) = self.new_tab(index) {
                                tracing::warn!(%err, "could not open a tab");
                            }
                        }
                        // The dropdown is the strip's own layer; the click is
                        // still chrome's, so it must not reach the terminal.
                        StripAction::OpenMenu => self.toggle_menu(),
                        StripAction::Ignore => {}
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                return false;
            }
        }

        if state == ElementState::Pressed {
            // Any press outside the band owns the next release, even if an
            // earlier band press never saw its own.
            self.press_in_chrome = false;
            if self.scrollbar_drag && button != MouseButton::Left {
                // The pointer belongs to the drag; a stray button must not
                // paste or reach the application mid-gesture.
                return false;
            }

            if button == MouseButton::Left {
                if let Some((geom, hit)) = self.scrollbar_hit(self.cursor_pos.0, self.cursor_pos.1)
                {
                    // Release any button forwarded before the drag takes over
                    // the pointer, so the application is not left stuck down.
                    self.release_forwarded_button();
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
                    return false;
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
                return false;
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
                        return false;
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
                    return false;
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
                return false;
            }

            // Shift+drag and non-reporting apps keep local text selection.
            self.app_mouse_button = None;

            // A right click that stays local pastes; the forwarding branch
            // above has already claimed the click for mouse-reporting apps.
            if button == MouseButton::Right {
                self.paste_clipboard();
                return false;
            }
            if button != MouseButton::Left {
                return false;
            }

            let Some(position) = click_position else {
                return false;
            };
            self.selection = Some(Selection::visual(position));
            self.mouse_down = true;
        } else {
            // Any release ends the press, so the chrome's claim on it dies here
            // whichever branch handles the event.
            let from_chrome = std::mem::take(&mut self.press_in_chrome);
            if self.scrollbar_drag && button == MouseButton::Left {
                self.scrollbar_drag = false;
                let (x, y) = self.cursor_pos;
                self.set_scrollbar_hover(self.scrollbar_hit(x, y).is_some());
                return false;
            } else if let Some(tracked) = release_button(self.app_mouse_button.take()) {
                // Route by the state captured at press time: a forwarded press
                // is released even when Shift or the mouse mode changed
                // mid-gesture.
                self.send_mouse(input::MouseEventKind::Release, Some(tracked));
                self.mouse_down = false;
                return false;
            } else {
                self.mouse_down = false;
                if button == MouseButton::Left && self.config.selection.copy_on_select {
                    if let Some(selection) = &self.selection {
                        let collapsed = selection.anchor == selection.focus;
                        if release_copies_selection(from_chrome, self.click_count, collapsed) {
                            self.copy_selection();
                        }
                    }
                }
            }
        }

        if let Some(window) = &self.window {
            window.request_redraw();
        }
        false
    }

    /// Inclusive cell range of the word or row containing `position`.
    fn bounds_at(&self, position: LogicalPos, whole_line: bool) -> Option<(usize, usize)> {
        let snapshot = self.tabs.active()?.terminal.snapshot();
        let line = snapshot.lines.get(position.row)?;
        Some(if whole_line {
            line_bounds(line)
        } else {
            word_bounds(line, position.col)
        })
    }

    fn app_mouse_mode(&self) -> TermMode {
        self.tabs
            .active()
            .map(|session| session.terminal.mode())
            .unwrap_or_else(TermMode::empty)
    }

    fn scrollbar_geometry(&self, hovered: bool) -> Option<ScrollbarGeometry> {
        let renderer = self.renderer.as_ref()?;
        let session = self.tabs.active()?;
        let window = self.window.as_ref()?;
        let size = window.inner_size();
        scrollbar::geometry(
            size.width,
            renderer.padding() + renderer.top_inset(),
            renderer.fonts().cell_h,
            self.rows,
            session.terminal.scrollable_history(),
            session.terminal.display_offset(),
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
        let Some(session) = self.tabs.active() else {
            return;
        };
        let history = session.terminal.scrollable_history();
        let current = session.terminal.display_offset();
        let target = scrollbar::offset_for_thumb_top(thumb_top, &geom, history);
        self.scroll_local(target as i32 - current as i32);
    }

    /// Scrolls the viewport locally by `lines` (positive scrolls back into
    /// history). Returns whether the offset changed.
    fn scroll_local(&mut self, lines: i32) -> bool {
        let Some(terminal) = self.active_terminal() else {
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
            GridGeometry::of(renderer),
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

    /// Balances a button forwarded to the application before the pointer
    /// changes owner: the release must reach the session that saw the press, or
    /// that application believes the button is still down forever.
    fn release_forwarded_button(&mut self) {
        if let Some(tracked) = release_button(self.app_mouse_button.take()) {
            self.send_mouse(input::MouseEventKind::Release, Some(tracked));
        }
    }

    fn pos_at(&self, x: f64, y: f64) -> Option<LogicalPos> {
        let renderer = self.renderer.as_ref()?;
        let snapshot = self.tabs.active()?.terminal.snapshot();
        let (row, visual) = cell_at(x, y, GridGeometry::of(renderer), self.cols, self.rows);
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
        let Some(session) = self.tabs.active() else {
            return;
        };
        let snapshot = session.terminal.snapshot();
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
            .tabs
            .active()
            .map(|session| session.terminal.mode().contains(TermMode::BRACKETED_PASTE))
            .unwrap_or(false);
        let bytes = input::paste(&text, bracketed);
        self.write_input(&bytes);
    }
}

/// Number of profiles that own a numbered `Ctrl+Shift+<digit>` shortcut.
///
/// Mirrors [`digit_profile`]: those rows name a key the window listens for, and
/// a further profile would name one nothing binds.
const PROFILE_SHORTCUTS: usize = 5;

/// The body of the About dialog. Kept pure so the wording is testable without
/// a window or an event loop.
fn about_text() -> String {
    format!(
        "Termyx {}\n\nA Rust terminal with RTL support.",
        env!("CARGO_PKG_VERSION")
    )
}

/// The rows of the `⌄` dropdown: one per profile, then the window commands.
///
/// The shortcut column is the key that actually reaches the row, so a profile
/// past [`PROFILE_SHORTCUTS`] leaves it empty instead of naming a key that
/// opens nothing.
fn menu_items(config: &Config) -> Vec<MenuItem> {
    let mut items: Vec<MenuItem> = crate::profiles::profiles(config)
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let shortcut = if index < PROFILE_SHORTCUTS {
                format!("Ctrl+Shift+{}", index + 1)
            } else {
                String::new()
            };
            MenuItem::profile(index, &profile.name, &shortcut)
        })
        .collect();
    // Neither row has a key binding, so neither may name one: an empty hint is
    // truthful, a dead shortcut is not.
    items.push(MenuItem::fixed(MenuAction::Settings, "Settings", ""));
    items.push(MenuItem::fixed(
        MenuAction::CommandPalette,
        "Command palette",
        "",
    ));
    items.push(MenuItem::fixed(MenuAction::About, "About", ""));
    items
}

/// What a press inside the tab strip asks the app to do.
///
/// The band owns every press in it, so a miss on a control is
/// [`StripAction::Ignore`] and never becomes a grid event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripAction {
    Select(usize),
    Close(usize),
    NewTab,
    OpenMenu,
    Ignore,
}

/// Maps a strip hit to its action. `hit` is `None` for a press on empty band.
///
/// A `✕` closes its tab whichever tab it is: the glyph and its square only
/// paint under the pointer, so a click on one is a deliberate act on the shell
/// the user can see, and the close lands on the first click as it does in
/// Windows Terminal.
fn strip_action(hit: Option<TabHit>) -> StripAction {
    match hit {
        Some(TabHit::Tab(index)) => StripAction::Select(index),
        Some(TabHit::Close(index)) => StripAction::Close(index),
        Some(TabHit::Plus) => StripAction::NewTab,
        Some(TabHit::Chevron) => StripAction::OpenMenu,
        None => StripAction::Ignore,
    }
}

/// The strip target the pointer must highlight for a strip action.
///
/// A hit on any tab's `✕` is a close rather than a select, but the button sits
/// inside its tab, so that tab is still the one under the pointer and must keep
/// its hover. The `+`/`⌄` buttons highlight as themselves.
fn strip_hover(action: Option<StripAction>) -> Option<StripHover> {
    match action {
        Some(StripAction::Select(index) | StripAction::Close(index)) => {
            Some(StripHover::Tab(index))
        }
        Some(StripAction::NewTab) => Some(StripHover::Plus),
        Some(StripAction::OpenMenu) => Some(StripHover::Chevron),
        Some(StripAction::Ignore) | None => None,
    }
}

/// The strip hover in `layout` under a pointer at `(x, y)`.
///
/// Used after a layout change moves the tabs under a stationary pointer: the
/// hover must name the tab that is there now, not the one that was there when
/// the pointer last moved. Outside the band nothing is hovered.
fn hover_after_layout_change(layout: &TabBarLayout, x: f64, y: f64) -> Option<StripHover> {
    if !in_strip(y, layout.strip_h) {
        return None;
    }
    strip_hover(Some(strip_action(layout.hit(x as f32, y as f32))))
}

/// What a press with the dropdown open asks the app to do.
///
/// The panel owns the pointer while it is open, so a press outside it only
/// dismisses; a press inside that lands on a row runs that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuPress {
    Choose(usize),
    Dismiss,
}

/// Maps a press against the open panel to its action. `hit` is the row under
/// the pointer and `inside` says whether the panel itself was hit.
///
/// A press outside the panel dismisses even when a stale row index is supplied:
/// leaving the panel always wins over whatever it last hovered.
fn menu_press(hit: Option<usize>, inside: bool) -> MenuPress {
    match (hit, inside) {
        (_, false) => MenuPress::Dismiss,
        (Some(index), true) => MenuPress::Choose(index),
        (None, true) => MenuPress::Dismiss,
    }
}

/// The row a press activates, if any.
///
/// Only the left button chooses a row. Another button still dismisses the panel
/// — the caller always closes it — but must not run the row under it, so a
/// right click cannot open the config file.
fn menu_choice(press: MenuPress, button: MouseButton) -> Option<usize> {
    match press {
        MenuPress::Choose(index) if button == MouseButton::Left => Some(index),
        _ => None,
    }
}

/// What a keyboard shortcut asks the tab strip to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabAction {
    Next,
    Previous,
    NewTab,
    CloseTab,
    /// Index into the configured profiles, already zero-based.
    SelectProfile(usize),
}

/// Shortcuts the window chrome owns. Kept free of `App` so the table can be
/// tested without an event loop.
///
/// Only the keys listed here are claimed. Everything else is left to the rest
/// of `on_key`: the copy and paste shortcuts first, then the terminal.
fn tab_shortcut(event: &KeyEvent, modifiers: ModifiersState) -> Option<TabAction> {
    shortcut_for(&event.logical_key, event.physical_key, modifiers)
}

/// The shortcut table itself.
///
/// Split from [`tab_shortcut`] because winit exposes no way to build a
/// `KeyEvent` outside its own backends, so the table can only be tested over
/// the key values an event carries.
fn shortcut_for(
    logical: &Key,
    physical: PhysicalKey,
    modifiers: ModifiersState,
) -> Option<TabAction> {
    // Alt and Super belong to the shell: `Ctrl+Alt+Shift+W` is `ESC ^W` to a
    // TUI, not a window command.
    if modifiers.alt_key() || modifiers.super_key() {
        return None;
    }
    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();
    if !ctrl {
        return None;
    }
    if let Key::Named(NamedKey::Tab) = logical {
        return Some(if shift {
            TabAction::Previous
        } else {
            TabAction::Next
        });
    }
    if !shift {
        return None;
    }
    // The digit is matched physically: Shift turns `2` into `@` on most
    // layouts, and winit only removes Ctrl from the logical key, so the
    // logical text cannot name the slot.
    if let Some(index) = digit_profile(physical) {
        return Some(TabAction::SelectProfile(index));
    }
    match logical {
        Key::Character(text) => match text.as_str() {
            "T" | "t" => Some(TabAction::NewTab),
            "W" | "w" => Some(TabAction::CloseTab),
            _ => None,
        },
        _ => None,
    }
}

/// The slot `delta` places from `current`, wrapping at both ends.
///
/// `None` when there is nowhere to move: no slot at all, or a lone one. The
/// caller reads that as "leave the strip alone" rather than repainting it.
fn cycle_index(current: usize, delta: isize, count: usize) -> Option<usize> {
    if count < 2 {
        return None;
    }
    Some((current as isize + delta).rem_euclid(count as isize) as usize)
}

/// Zero-based profile index for a physical number key, or `None` past the
/// fifth. Only the digits with a shortcut are listed, so `0` cannot underflow.
fn digit_profile(physical: PhysicalKey) -> Option<usize> {
    match physical {
        PhysicalKey::Code(KeyCode::Digit1) => Some(0),
        PhysicalKey::Code(KeyCode::Digit2) => Some(1),
        PhysicalKey::Code(KeyCode::Digit3) => Some(2),
        PhysicalKey::Code(KeyCode::Digit4) => Some(3),
        PhysicalKey::Code(KeyCode::Digit5) => Some(4),
        _ => None,
    }
}

/// Whether a physical y coordinate lies inside the strip band.
fn in_strip(y: f64, strip_h: f32) -> bool {
    y >= 0.0 && y < f64::from(strip_h)
}

/// Blink phase for a DECSCUSR blinking cursor. `deadline` is `None` while the
/// cursor is steady, so the event loop can wait for input instead of a timer.
#[derive(Debug, Clone, Copy)]
struct CursorBlink {
    visible: bool,
    deadline: Option<Instant>,
}

impl CursorBlink {
    fn idle() -> Self {
        Self {
            visible: true,
            deadline: None,
        }
    }

    /// Starts the cycle the first frame that draws a blinking cursor.
    fn start(&mut self, now: Instant) {
        if self.deadline.is_none() {
            self.visible = true;
            self.deadline = Some(now + BLINK_INTERVAL);
        }
    }

    /// Shows the cursor and restarts the cycle after user activity, so it never
    /// hides just as the user types.
    fn restart(&mut self, now: Instant) {
        if self.deadline.is_some() {
            self.visible = true;
            self.deadline = Some(now + BLINK_INTERVAL);
        }
    }

    /// Returns the cursor to steady and stops the timer.
    fn stop(&mut self) {
        self.visible = true;
        self.deadline = None;
    }

    /// Flips the phase once the deadline passes. Returns whether it flipped.
    fn advance(&mut self, now: Instant) -> bool {
        if !self.deadline.is_some_and(|deadline| now >= deadline) {
            return false;
        }
        self.visible = !self.visible;
        self.deadline = Some(now + BLINK_INTERVAL);
        true
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

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyOutput { session, bytes } => {
                if let FeedOutcome::Active { reflowed } =
                    feed_session(&mut self.tabs, session, &bytes)
                {
                    if reflowed {
                        // Rows moved, so row-anchored selection positions are
                        // stale. An in-place repaint keeps the selection.
                        self.selection = None;
                        self.mouse_down = false;
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            UserEvent::Exited { session } => {
                // A background tab closes without taking the view: `TabSet`
                // only moves the active slot when it was the one removed.
                let Some(index) = self.tabs.position(session) else {
                    // The tab is already gone; this is the reader of a session
                    // the user closed.
                    return;
                };
                tracing::debug!(session, index, "shell exited; closing its tab");
                if self.close_tab(index) {
                    // A window with no tab has nothing left to show.
                    event_loop.exit();
                }
            }
            UserEvent::Title { session, title } => {
                let (label_changed, retitle) = route_title(&mut self.tabs, session, &title);
                if retitle {
                    self.apply_title(&title);
                }
                // The strip paints every tab's label, so a background retitle
                // also needs a fresh layout and a repaint; an unchanged label
                // needs neither.
                if label_changed {
                    self.rebuild_layout();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            UserEvent::ResetTitle { session } => {
                let (label_changed, retitle) = route_title(&mut self.tabs, session, "");
                if retitle {
                    self.apply_title("");
                }
                if label_changed {
                    self.rebuild_layout();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.blink.advance(Instant::now()) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        match self.blink.deadline {
            Some(deadline) => event_loop.set_control_flow(ControlFlow::WaitUntil(deadline)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
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
                self.rebuild_layout();
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
            WindowEvent::CursorLeft { .. } => {
                // The pointer is off the window, so neither the strip nor a
                // menu row is hovered.
                self.set_strip_hover(None);
                self.set_scrollbar_hover(false);
                if let Some(menu) = self.menu.as_mut() {
                    if menu.hovered.take().is_some() {
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.on_mouse(state, button) {
                    // Closing the last tab leaves nothing to show.
                    event_loop.exit();
                }
            }
            WindowEvent::Focused(focused) => {
                if focused {
                    // Coming back to the window shows the cursor immediately.
                    self.blink.restart(Instant::now());
                }
                if !focused {
                    // A pressed button may never get its release after focus
                    // loss; balance the application and end local drags.
                    self.release_forwarded_button();
                    self.mouse_down = false;
                    // A strip press ends off-window too, so its release must
                    // not still suppress copy-on-select in the grid.
                    self.press_in_chrome = false;
                    self.scrollbar_drag = false;
                    let (x, y) = self.cursor_pos;
                    self.set_scrollbar_hover(self.scrollbar_hit(x, y).is_some());
                }
                if self.app_mouse_mode().contains(TermMode::FOCUS_IN_OUT) {
                    self.write_input(&input::encode_focus(focused));
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.on_key(event) {
                    // Closing the last tab by keyboard leaves nothing to show.
                    event_loop.exit();
                }
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

/// Logical-to-physical conversion for the tab strip: the single place the
/// strip height is scaled. Everything downstream (the renderer inset, the grid
/// size, the mouse origin) works in the physical pixels this returns.
fn strip_px(scale_factor: f64) -> u32 {
    (crate::ui::tab_bar::TAB_STRIP_HEIGHT * scale_factor as f32).round() as u32
}

/// Where a session's PTY output went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FeedOutcome {
    /// No session carries the id: a late event from a closed tab. It must never
    /// reach another session's parser.
    Dropped,
    /// Fed into a tab the window is not showing, so its grid stays current for
    /// when the user switches to it. Nothing on screen changed.
    Background,
    /// Fed into the tab on screen. `reflowed` marks that rows moved, which
    /// invalidates row-anchored selection positions.
    Active { reflowed: bool },
}

/// Feeds one session's output into the parser of that session.
///
/// Routing by id — not by "whatever terminal is active" — is what keeps two
/// tabs from merging their output.
fn feed_session(tabs: &mut TabSet, session: SessionId, bytes: &[u8]) -> FeedOutcome {
    let shown = tabs.active().map(|active| active.id);
    let Some(target) = tabs.session_mut(session) else {
        return FeedOutcome::Dropped;
    };
    let reflowed = target.terminal.feed(bytes);
    if shown == Some(session) {
        FeedOutcome::Active { reflowed }
    } else {
        FeedOutcome::Background
    }
}

/// Routes one title event to the tab it names.
///
/// The label always belongs to that tab, on screen or not; an id no session
/// carries changes nothing. Returns `(label_changed, retitle)`: whether
/// the stored label actually changed (the caller rebuilds and repaints only
/// then), and whether the caller must also apply the title to the window,
/// which is true only for the tab on screen.
fn route_title(tabs: &mut TabSet, session: SessionId, title: &str) -> (bool, bool) {
    let label_changed = tabs.set_title(session, title);
    let retitle = tabs.active().is_some_and(|active| active.id == session);
    (label_changed, retitle)
}

/// The OS window title for the tab on screen.
///
/// `dynamic_title` is the config opt-in: when it is off the window keeps
/// [`FALLBACK_TITLE`] whatever the tab says. With it on, the active tab's
/// label names the window, so switching tabs renames it; a window with no tab
/// has nothing to name.
fn window_title(dynamic_title: bool, active_title: Option<&str>) -> String {
    match active_title {
        Some(title) if dynamic_title => title.to_string(),
        _ => FALLBACK_TITLE.to_string(),
    }
}

fn grid_size(
    width: u32,
    height: u32,
    cell_w: u32,
    cell_h: u32,
    padding: u32,
    top_inset: u32,
) -> (usize, usize) {
    // The scrollbar lives in a reserved gutter so its hovered width never
    // covers the last text column.
    let gutter = scrollbar::GUTTER.ceil() as u32;
    let usable_w = width.saturating_sub(padding.saturating_mul(2).saturating_add(gutter));
    let usable_h = height.saturating_sub(padding.saturating_mul(2).saturating_add(top_inset));
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

/// Whether a local release copies the selection.
///
/// A release whose press window chrome consumed (the tab strip or the menu) is
/// not a selection gesture: it must not copy a selection that was already on
/// screen before the click, even though that selection is still non-collapsed.
fn release_copies_selection(from_chrome: bool, click_count: u8, collapsed: bool) -> bool {
    !from_chrome && should_copy_on_release(click_count, collapsed)
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

/// Where the grid starts and how big its cells are, in physical pixels. The x
/// origin is the window padding alone; the y origin also clears the tab strip,
/// so the two origins stay distinct here.
///
/// Derived from the renderer on every use rather than cached on `App`, so the
/// renderer stays the single source of grid geometry and a hit test cannot
/// disagree with the pixels last drawn.
#[derive(Clone, Copy)]
struct GridGeometry {
    padding: f64,
    top_inset: f64,
    cell_w: f64,
    cell_h: f64,
}

impl GridGeometry {
    fn of(renderer: &Renderer) -> Self {
        Self {
            padding: renderer.padding() as f64,
            top_inset: renderer.top_inset() as f64,
            cell_w: renderer.fonts().cell_w as f64,
            cell_h: renderer.fonts().cell_h as f64,
        }
    }
}

fn cell_at(x: f64, y: f64, grid: GridGeometry, cols: usize, rows: usize) -> (usize, usize) {
    let cols = cols.max(1);
    let rows = rows.max(1);
    let visual =
        (((x - grid.padding) / grid.cell_w).floor() as isize).clamp(0, cols as isize - 1) as usize;
    let row = (((y - grid.padding - grid.top_inset) / grid.cell_h).floor() as isize)
        .clamp(0, rows as isize - 1) as usize;
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

    use crate::config::{BaseDirection, ShellConfig};
    use crate::pty::PtySession;
    use crate::session::Session;
    use crate::term::{Cell, RowSnapshot, Style};

    #[test]
    fn grid_size_uses_padding_and_minimums() {
        assert_eq!(grid_size(1200, 800, 10, 20, 4, 0), (117, 39));
        assert_eq!(grid_size(100, 100, 10, 20, 4, 0), (20, 5));
    }

    #[test]
    fn grid_size_reserves_the_scrollbar_gutter() {
        let usable = 1200 - 4 * 2;
        assert_eq!(
            grid_size(1200, 800, 1, 20, 4, 0).0,
            usable - scrollbar::GUTTER.ceil() as usize
        );
    }

    #[test]
    fn the_tab_strip_costs_two_rows_of_grid() {
        let padding = 8;
        let (_, full) = grid_size(800, 600, 8, 16, padding, 0);
        let (_, with_strip) = grid_size(800, 600, 8, 16, padding, 32);
        assert_eq!(full - with_strip, 2);
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
        let grid = GridGeometry {
            padding: 4.0,
            top_inset: 0.0,
            cell_w: 10.0,
            cell_h: 20.0,
        };
        assert_eq!(cell_at(4.0, 4.0, grid, 80, 24), (0, 0));
        assert_eq!(cell_at(13.0, 24.0, grid, 80, 24), (1, 0));
        assert_eq!(cell_at(10_000.0, 10_000.0, grid, 80, 24), (23, 79));

        // The strip moves the rows down but never shifts the columns: x = 26 is
        // column 2 whether or not a strip is reserved, and y = 40 is row 0 only
        // because the 32 px strip is subtracted from y.
        let stripped = GridGeometry {
            top_inset: 32.0,
            ..grid
        };
        assert_eq!(cell_at(26.0, 40.0, stripped, 80, 24), (0, 2));
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

    #[test]
    fn a_strip_release_never_copies_an_older_selection() {
        // The band starts no selection, so its release is not a selection
        // gesture even when a non-collapsed selection is still on screen.
        assert!(!release_copies_selection(true, 1, false));
        assert!(!release_copies_selection(true, 2, false));

        // A local release keeps today's copy-on-select decision.
        assert!(release_copies_selection(false, 1, false));
        assert!(release_copies_selection(false, 2, true));
        assert!(!release_copies_selection(false, 1, true));
    }

    #[test]
    fn blink_flips_after_the_interval_and_stops_when_steady() {
        let now = Instant::now();
        let mut blink = CursorBlink::idle();
        blink.start(now);
        assert!(blink.visible);
        assert!(!blink.advance(now + BLINK_INTERVAL / 2));
        assert!(blink.advance(now + BLINK_INTERVAL));
        assert!(!blink.visible);
        assert!(blink.advance(now + BLINK_INTERVAL * 2));
        assert!(blink.visible);

        blink.stop();
        assert!(blink.visible);
        assert_eq!(blink.deadline, None);
        assert!(!blink.advance(now + BLINK_INTERVAL * 10));
    }

    #[test]
    fn blink_restart_shows_the_cursor_and_extends_the_phase() {
        let now = Instant::now();
        let mut blink = CursorBlink::idle();
        blink.start(now);
        blink.advance(now + BLINK_INTERVAL);
        assert!(!blink.visible);

        blink.restart(now + BLINK_INTERVAL);
        assert!(blink.visible);
        assert_eq!(blink.deadline, Some(now + BLINK_INTERVAL * 2));
    }

    #[test]
    fn steady_cursor_never_starts_a_timer() {
        let mut blink = CursorBlink::idle();
        blink.restart(Instant::now());
        assert_eq!(blink.deadline, None);
    }

    #[test]
    fn a_click_on_the_plus_hit_is_recognised() {
        let layout = crate::ui::tab_bar::TabBarLayout::new(800.0, 32.0, &["one", "two"], |_| 80.0);
        let hit = layout.hit(layout.plus.x + 4.0, 10.0);
        assert_eq!(hit, Some(crate::ui::tab_bar::TabHit::Plus));
    }

    #[test]
    fn a_click_below_the_strip_is_not_a_tab_hit() {
        let layout = crate::ui::tab_bar::TabBarLayout::new(800.0, 32.0, &["one"], |_| 80.0);
        assert!(layout.hit(10.0, 40.0).is_none());
    }

    #[test]
    fn a_press_on_empty_strip_band_is_consumed_as_ignore() {
        // Nothing sits under the pointer in the band, so the press stays the
        // strip's and never becomes a grid event.
        assert_eq!(strip_action(None), StripAction::Ignore);
    }

    #[test]
    fn a_press_on_a_menu_row_chooses_it() {
        assert_eq!(menu_press(Some(2), true), MenuPress::Choose(2));
    }

    #[test]
    fn a_press_inside_the_panel_but_between_rows_dismisses() {
        assert_eq!(menu_press(None, true), MenuPress::Dismiss);
    }

    #[test]
    fn a_press_outside_the_panel_always_dismisses() {
        assert_eq!(menu_press(None, false), MenuPress::Dismiss);
        assert_eq!(
            menu_press(Some(1), false),
            MenuPress::Dismiss,
            "leaving the panel wins over a stale row index"
        );
    }

    #[test]
    fn only_the_left_button_chooses_a_menu_row() {
        assert_eq!(
            menu_choice(MenuPress::Choose(2), MouseButton::Left),
            Some(2)
        );
        assert_eq!(
            menu_choice(MenuPress::Choose(2), MouseButton::Right),
            None,
            "a right click dismisses the panel without running its row"
        );
        assert_eq!(menu_choice(MenuPress::Dismiss, MouseButton::Left), None);
    }

    #[test]
    fn the_strip_controls_map_to_their_actions() {
        use crate::ui::tab_bar::TabHit;
        assert_eq!(strip_action(Some(TabHit::Tab(2))), StripAction::Select(2));
        assert_eq!(strip_action(Some(TabHit::Plus)), StripAction::NewTab);
        assert_eq!(strip_action(Some(TabHit::Chevron)), StripAction::OpenMenu);
    }

    fn six_profiles() -> Config {
        let mut text = String::new();
        for index in 1..=6 {
            text.push_str(&format!(
                "[[profiles]]\nname = \"Profile {index}\"\nprogram = \"shell{index}.exe\"\n"
            ));
        }
        Config::from_toml_str(&text).expect("the profiles must parse")
    }

    #[test]
    fn the_menu_lists_a_row_per_profile_then_the_window_commands() {
        let items = menu_items(&six_profiles());
        assert_eq!(items.len(), 6 + 3);
        assert_eq!(items[0].action, MenuAction::Profile(0));
        assert_eq!(items[0].label, "Profile 1");
        assert_eq!(items[5].action, MenuAction::Profile(5));
        assert_eq!(
            (
                items[6].action,
                items[6].label.as_str(),
                items[6].shortcut.as_str()
            ),
            (MenuAction::Settings, "Settings", "")
        );
        assert_eq!(
            (
                items[7].action,
                items[7].label.as_str(),
                items[7].shortcut.as_str()
            ),
            (MenuAction::CommandPalette, "Command palette", "")
        );
        assert_eq!(
            (
                items[8].action,
                items[8].label.as_str(),
                items[8].shortcut.as_str()
            ),
            (MenuAction::About, "About", "")
        );
    }

    #[test]
    fn the_about_text_names_the_app_and_its_version() {
        let text = about_text();
        assert!(
            text.contains("Termyx"),
            "the dialog must say which app it describes, got {text:?}"
        );
        assert!(
            text.contains(env!("CARGO_PKG_VERSION")),
            "the dialog must show the version this build reports, got {text:?}"
        );
    }

    #[test]
    fn only_the_first_five_profiles_name_a_shortcut() {
        let items = menu_items(&six_profiles());
        assert_eq!(items[0].shortcut, "Ctrl+Shift+1");
        assert_eq!(items[4].shortcut, "Ctrl+Shift+5");
        assert_eq!(
            items[5].shortcut, "",
            "a sixth profile has no shortcut, so its row must not invent one"
        );
    }

    #[test]
    fn a_close_click_closes_its_tab_even_when_the_tab_is_not_active() {
        use crate::ui::tab_bar::TabHit;
        assert_eq!(
            strip_action(Some(TabHit::Close(1))),
            StripAction::Close(1),
            "every visible ✕ must close on the first click"
        );
        assert_eq!(
            strip_action(Some(TabHit::Close(0))),
            StripAction::Close(0),
            "the target does not depend on which tab the window shows"
        );
    }

    #[test]
    fn a_stationary_pointer_re_hovers_the_tab_that_moves_under_it() {
        use crate::ui::tab_bar::{TabBarLayout, TabHit};
        // Three equal tabs; the pointer rests on the middle tab's `✕`.
        let before = TabBarLayout::new(600.0, 32.0, &["one", "two", "three"], |_| 100.0);
        let close = before.tabs[1].close;
        let (x, y) = (close.x + 1.0, close.y + 1.0);
        assert_eq!(before.hit(x, y), Some(TabHit::Close(1)));

        // Closing that tab slides the third into the same geometry, close box
        // included, so the same pixel is still a close target. The hover has to
        // follow it, or the second click closes a tab whose `✕` never painted.
        let after = TabBarLayout::new(600.0, 32.0, &["one", "three"], |_| 100.0);
        assert_eq!(after.hit(x, y), Some(TabHit::Close(1)));
        assert_eq!(
            hover_after_layout_change(&after, x as f64, y as f64),
            Some(StripHover::Tab(1)),
            "the pointer must re-hover the close target so its ✕ paints before a second click"
        );
    }

    #[test]
    fn the_active_tab_stays_hovered_over_its_close_button() {
        assert_eq!(
            strip_hover(Some(StripAction::Close(1))),
            Some(StripHover::Tab(1)),
            "the close button sits inside its tab, so the backdrop must stay"
        );
        assert_eq!(
            strip_hover(Some(StripAction::Select(0))),
            Some(StripHover::Tab(0))
        );
        assert_eq!(
            strip_hover(Some(StripAction::NewTab)),
            Some(StripHover::Plus),
            "the plus is its own hover target"
        );
        assert_eq!(
            strip_hover(Some(StripAction::OpenMenu)),
            Some(StripHover::Chevron),
            "the chevron is its own hover target"
        );
        assert_eq!(strip_hover(Some(StripAction::Ignore)), None);
        assert_eq!(strip_hover(None), None);
    }

    #[test]
    fn new_tab_and_close_tab_shortcuts_are_recognised() {
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(
            shortcut_for(
                &Key::Character("T".into()),
                PhysicalKey::Code(KeyCode::KeyT),
                ctrl_shift
            ),
            Some(TabAction::NewTab)
        );
        assert_eq!(
            shortcut_for(
                &Key::Character("W".into()),
                PhysicalKey::Code(KeyCode::KeyW),
                ctrl_shift
            ),
            Some(TabAction::CloseTab)
        );
    }

    #[test]
    fn ctrl_tab_cycles_forwards_and_back() {
        assert_eq!(
            shortcut_for(
                &Key::Named(NamedKey::Tab),
                PhysicalKey::Code(KeyCode::Tab),
                ModifiersState::CONTROL
            ),
            Some(TabAction::Next)
        );
        assert_eq!(
            shortcut_for(
                &Key::Named(NamedKey::Tab),
                PhysicalKey::Code(KeyCode::Tab),
                ModifiersState::CONTROL | ModifiersState::SHIFT
            ),
            Some(TabAction::Previous)
        );
    }

    #[test]
    fn a_shifted_digit_selects_a_profile_by_its_physical_key() {
        // Windows removes only Ctrl, so `Ctrl+Shift+2` arrives as `@`. The slot
        // has to come from the physical key or the shortcut never fires.
        assert_eq!(
            shortcut_for(
                &Key::Character("@".into()),
                PhysicalKey::Code(KeyCode::Digit2),
                ModifiersState::CONTROL | ModifiersState::SHIFT
            ),
            Some(TabAction::SelectProfile(1))
        );
    }

    #[test]
    fn a_logical_digit_still_selects_a_profile() {
        // A layout whose shifted digit stays the digit reports it both ways.
        assert_eq!(
            shortcut_for(
                &Key::Character("2".into()),
                PhysicalKey::Code(KeyCode::Digit2),
                ModifiersState::CONTROL | ModifiersState::SHIFT
            ),
            Some(TabAction::SelectProfile(1))
        );
    }

    #[test]
    fn copy_and_paste_keys_stay_out_of_the_shortcut_table() {
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(
            shortcut_for(
                &Key::Character("C".into()),
                PhysicalKey::Code(KeyCode::KeyC),
                ctrl_shift
            ),
            None
        );
        assert_eq!(
            shortcut_for(
                &Key::Character("V".into()),
                PhysicalKey::Code(KeyCode::KeyV),
                ctrl_shift
            ),
            None
        );
    }

    #[test]
    fn plain_and_shift_tab_stay_with_the_terminal() {
        assert_eq!(
            shortcut_for(
                &Key::Named(NamedKey::Tab),
                PhysicalKey::Code(KeyCode::Tab),
                ModifiersState::empty()
            ),
            None
        );
        assert_eq!(
            shortcut_for(
                &Key::Named(NamedKey::Tab),
                PhysicalKey::Code(KeyCode::Tab),
                ModifiersState::SHIFT
            ),
            None
        );
    }

    #[test]
    fn only_the_first_five_digits_name_profiles() {
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        // `0` must not underflow into a profile index.
        assert_eq!(
            shortcut_for(
                &Key::Character("0".into()),
                PhysicalKey::Code(KeyCode::Digit0),
                ctrl_shift
            ),
            None
        );
        assert_eq!(
            shortcut_for(
                &Key::Character("6".into()),
                PhysicalKey::Code(KeyCode::Digit6),
                ctrl_shift
            ),
            None
        );
        assert_eq!(
            shortcut_for(
                &Key::Character("%".into()),
                PhysicalKey::Code(KeyCode::Digit5),
                ctrl_shift
            ),
            Some(TabAction::SelectProfile(4))
        );
    }

    #[test]
    fn alt_or_super_keeps_a_key_for_the_terminal() {
        // `Ctrl+Alt+Shift+W` is `ESC ^W` to a TUI; the window must not eat it.
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(
            shortcut_for(
                &Key::Character("W".into()),
                PhysicalKey::Code(KeyCode::KeyW),
                ctrl_shift | ModifiersState::ALT
            ),
            None
        );
        assert_eq!(
            shortcut_for(
                &Key::Named(NamedKey::Tab),
                PhysicalKey::Code(KeyCode::Tab),
                ModifiersState::CONTROL | ModifiersState::SUPER
            ),
            None
        );
    }

    #[test]
    fn unmodified_text_is_not_a_shortcut() {
        assert_eq!(
            shortcut_for(
                &Key::Character("t".into()),
                PhysicalKey::Code(KeyCode::KeyT),
                ModifiersState::empty()
            ),
            None
        );
    }

    #[test]
    fn cycling_wraps_at_both_ends() {
        assert_eq!(cycle_index(1, 1, 3), Some(2));
        assert_eq!(cycle_index(1, -1, 3), Some(0));
        assert_eq!(cycle_index(2, 1, 3), Some(0));
        assert_eq!(cycle_index(0, -1, 3), Some(2));
        // A delta longer than one lap still lands inside the strip.
        assert_eq!(cycle_index(0, 7, 3), Some(1));
    }

    #[test]
    fn cycling_needs_more_than_one_tab() {
        assert_eq!(cycle_index(0, 1, 0), None);
        assert_eq!(cycle_index(0, 1, 1), None);
        assert_eq!(cycle_index(0, -1, 1), None);
    }

    #[test]
    fn the_strip_band_ends_at_the_reserved_height() {
        assert!(in_strip(0.0, 32.0));
        assert!(in_strip(31.9, 32.0));
        assert!(!in_strip(32.0, 32.0));
        assert!(!in_strip(-0.1, 32.0));
        assert!(!in_strip(0.0, 0.0));
    }

    #[test]
    fn closing_the_last_tab_leaves_no_active_session() {
        let mut slots = crate::session::TabSlots::new();
        slots.push(1);
        slots.push(2);
        slots.select(0);
        assert_eq!(slots.remove_active(), Some(1));
        assert_eq!(slots.remove_active(), Some(2));
        assert_eq!(slots.remove_active(), None);
    }

    /// An inert shell that stays open until the session is dropped, so the
    /// routing test never depends on a command finishing.
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

    /// The text of a session's first grid row, without the trailing blanks the
    /// snapshot pads every row with.
    fn first_row(session: &Session) -> String {
        let snapshot = session.terminal.snapshot();
        snapshot.lines[0]
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn output_reaches_only_the_session_it_names() {
        let mut tabs = TabSet::new();
        let background = tabs.next_id();
        let shown = tabs.next_id();
        tabs.push(test_session(background));
        tabs.push(test_session(shown));
        assert_eq!(tabs.active().map(|session| session.id), Some(shown));

        // A background tab still parses its own output so its grid is current
        // when the user switches to it, but it is not the tab on screen.
        assert_eq!(
            feed_session(&mut tabs, background, b"bg"),
            FeedOutcome::Background
        );
        assert_eq!(first_row(&tabs[0]), "bg");
        assert_eq!(first_row(&tabs[1]), "");

        // An id no session carries — a late read from a closed tab — must not
        // fall through to the active terminal. It changes no grid, and it is
        // not `Active`, the only outcome that requests a redraw.
        let closed = tabs.next_id();
        assert_eq!(
            feed_session(&mut tabs, closed, b"ghost"),
            FeedOutcome::Dropped
        );
        assert_eq!(first_row(&tabs[0]), "bg");
        assert_eq!(first_row(&tabs[1]), "");

        // Only output naming the shown tab reaches the visible grid.
        assert_eq!(
            feed_session(&mut tabs, shown, b"fg"),
            FeedOutcome::Active { reflowed: false }
        );
        assert_eq!(first_row(&tabs[0]), "bg");
        assert_eq!(first_row(&tabs[1]), "fg");
    }

    #[test]
    fn the_tab_strip_scales_once_with_the_window() {
        // The strip is 32 logical pixels. `strip_px` is the only place that
        // converts, because the renderer, the surface, and the mouse
        // coordinates all work in physical pixels.
        assert_eq!(strip_px(1.0), 32);
        assert_eq!(strip_px(1.25), 40);
        assert_eq!(strip_px(1.5), 48);
    }

    #[test]
    fn a_background_title_labels_its_tab_but_not_the_window() {
        // `App` needs a live event loop, so the routing decision is pinned
        // through `route_title`: its first flag repaints the strip and its
        // second renames the window.
        let mut tabs = TabSet::new();
        let background = tabs.next_id();
        let shown = tabs.next_id();
        tabs.push(test_session(background));
        tabs.push(test_session(shown));

        // A background tab's label lands on that tab and only that tab, and
        // the window keeps its name.
        let (changed, window) = route_title(&mut tabs, background, "build");
        assert!(changed, "a new background label must repaint the strip");
        assert!(!window);
        assert_eq!(tabs.titles(), vec!["build", FALLBACK_TITLE]);

        // The tab on screen names both its label and the window.
        let (changed, window) = route_title(&mut tabs, shown, "tests");
        assert!(changed);
        assert!(window);
        assert_eq!(tabs.titles(), vec!["build", "tests"]);

        // Resetting either tab clears just that label back to the fallback;
        // only the tab on screen may reset the window name.
        let (changed, window) = route_title(&mut tabs, background, "");
        assert!(changed);
        assert!(!window);
        assert_eq!(tabs.titles(), vec![FALLBACK_TITLE, "tests"]);
        let (changed, window) = route_title(&mut tabs, shown, "");
        assert!(changed);
        assert!(window);
        assert_eq!(tabs.titles(), vec![FALLBACK_TITLE, FALLBACK_TITLE]);

        // A late title from a closed tab changes no label and never names the
        // window.
        let closed = tabs.next_id();
        let (changed, window) = route_title(&mut tabs, closed, "ghost");
        assert!(!changed);
        assert!(!window);
        assert_eq!(tabs.titles(), vec![FALLBACK_TITLE, FALLBACK_TITLE]);
    }

    #[test]
    fn a_static_title_ignores_the_active_tab() {
        assert_eq!(window_title(false, Some("build")), FALLBACK_TITLE);
        assert_eq!(window_title(false, None), FALLBACK_TITLE);
    }

    #[test]
    fn a_dynamic_title_follows_the_active_tab() {
        assert_eq!(window_title(true, Some("build")), "build");
    }

    #[test]
    fn a_dynamic_title_with_no_tab_falls_back() {
        assert_eq!(window_title(true, None), FALLBACK_TITLE);
    }

    #[test]
    fn a_repeated_title_costs_nothing_but_a_new_one_repaints() {
        let mut tabs = TabSet::new();
        let background = tabs.next_id();
        let shown = tabs.next_id();
        tabs.push(test_session(background));
        tabs.push(test_session(shown));

        let (changed, window) = route_title(&mut tabs, background, "build");
        assert!(
            changed,
            "a background tab's new label must request a repaint"
        );
        assert!(!window, "a background tab must not rename the window");

        let (changed, window) = route_title(&mut tabs, background, "build");
        assert!(!changed, "a repeated label must cost no rebuild or repaint");
        assert!(!window);
    }
}
