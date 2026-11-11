# Phase 5: Windows-Terminal-style window chrome

## Goal

The tab strip becomes the window's title bar: no native caption, tabs flush
with the top edge of the window, and minimise / maximise / close painted into
the right end of the same band. The window keeps resizing from every edge,
Aero Snap, and a drop shadow.

Today the window shows the Windows caption ("Termyx" plus the app icon)
and the 32 px strip is painted below it inside the client area. The user
compared that with Windows Terminal and asked for the Windows Terminal layout.

## Global constraints

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test --all-targets`, `cargo build --release` must all pass.
- No dead code. `#[allow(dead_code)]` is forbidden. An item with only test
  consumers is gated `#[cfg(test)]` with a why-comment naming the consumer, and
  the consuming task drops the gate.
- No `unwrap`/`expect` outside tests. Errors at the edge: `thiserror` per
  module, `anyhow` in the app; a recoverable failure logs and continues.
- The terminal grid stays logical; visual reordering happens per row through
  `VisualMap`; PTY bytes are fed verbatim.
- One responsibility per file and function; small diffs; no unrelated
  reformatting; comments explain why, not what; no commented-out code.
- The M5 budgets still hold: 200x50 frame under 8 ms, idle memory under 80 MB.

## Visual target (authoritative)

Reference: Windows Terminal on Windows 11, dark theme.

- The client area starts at the window's top edge. There is no caption row, no
  app icon and no window title text in the chrome.
- The tab strip is still 32 logical px tall and still spans the full window
  width. It is the first thing inside the frame.
- Painted controls, right to left at the strip's right end, each 46 logical px
  wide and `strip_h` tall:
  - `✕` (window close): hover fill `#c42b1c`, glyph `#ffffff`.
  - `▢` (restore when maximised, single square otherwise): hover fill
    `theme.tab_hover`.
  - `—` (minimise): hover fill `theme.tab_hover`.
  Glyphs are centred by their ink box, as the `+`/`⌄`/`✕` chrome glyphs already
  are, and use `theme.tab_title` on hover and `theme.tab_title_inactive`
  otherwise.
- Tabs never overlap the three controls: the controls' width is reserved out
  of the strip's usable width before tabs are sized, exactly as the `+`/`⌄`
  buttons already are.
- The strip's remaining empty space (after the last tab and `+`/`⌄`, before the
  controls) is a drag handle: press and drag moves the window, double-click
  toggles maximise.

## Out of scope (record in README, do not implement)

- The Snap Layouts flyout that Windows shows when hovering the native maximise
  button. It is a DWM feature of real caption buttons; painted buttons cannot
  summon it. Dragging to a screen edge still snaps.
- Mirroring the three controls to the left edge under a right-to-left system
  locale. Windows Terminal does this; we do not, yet.
- Custom taskbar-jump-list, thumbnail-toolbar or DWM backdrop (Mica/Acrylic).

## File structure

- `src/platform.rs` — the raw Win32 calls that winit does not expose: restoring
  the resize frame on a frameless window, and (Windows only) the no-op
  fallbacks elsewhere.
- `src/ui/tab_bar.rs` — the caption buttons join the tab-strip geometry, so
  paint and hit-testing keep sharing one description.
- `src/render/mod.rs` — painting the three controls and their hover states.
- `src/app.rs` — routing presses on the controls, the drag handle, and
  re-deriving geometry when the scale factor changes.

## Task 1: Frameless window that still resizes and snaps

**Files:** `src/platform.rs`, `src/app.rs`

**Interfaces:**
- Produces: `platform::apply_custom_chrome(window: &Window)`.
- Consumes: nothing new.

**Why this shape.** `winit` cannot remove only the caption: `set_decorations(false)`
turns the window into a `WS_POPUP` with no resize frame, no snap and no shadow.
Creating it frameless and then restoring `WS_THICKFRAME` (plus
`WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU`) keeps `DefWindowProc`'s native
edge resizing, Aero Snap and the DWM shadow, while leaving the client area
starting at the window's top edge. `platform.rs` already holds raw Win32 FFI
for `DwmSetWindowAttribute`, so this needs no new crate.

- [ ] **Step 1: Write the failing test**

Windows-only geometry cannot be unit-tested, but the decision it feeds can.
Add to `src/app.rs`:

```rust
#[test]
fn a_frameless_window_reserves_the_same_strip_as_a_framed_one() {
    // `tab_strip_px` is the single conversion from the logical strip height to
    // the physical inset; removing the caption must not change it, or the grid
    // would jump by the caption's height.
    let mut app = App::headless_for_tests();
    app.window_scale = 1.5;
    assert_eq!(app.tab_strip_px(), (TAB_STRIP_HEIGHT * 1.5) as u32);
}
```

If `App` cannot be built headlessly, test the free function it delegates to
(`strip_px(TAB_STRIP_HEIGHT, scale)`) instead and say so in the report.

- [ ] **Step 2: Run the test and watch it fail**

`cargo test --all-targets a_frameless_window_reserves_the_same_strip`
Expected: compile error, because the helper does not exist.

- [ ] **Step 3: Create the window frameless**

In `src/app.rs::create_window`, add `.with_decorations(false)` to the
`Window::default_attributes()` chain, keeping `.with_title(FALLBACK_TITLE)`
(the taskbar still reads it) and `.with_inner_size(...)`.

- [ ] **Step 4: Restore the frame**

Add to `src/platform.rs`:

```rust
/// Restores the resize frame, snap and shadow that removing the caption also
/// removes.
///
/// `set_decorations(false)` makes the window a `WS_POPUP`: frameless but also
/// un-resizable, un-snappable and shadowless. Re-adding `WS_THICKFRAME` (and
/// the system-menu/minimise/maximise bits) hands edge hit-testing and snapping
/// back to `DefWindowProc` while the caption area stays gone.
#[cfg(target_os = "windows")]
pub fn apply_custom_chrome(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    const GWL_STYLE: i32 = -16;
    const WS_THICKFRAME: isize = 0x0004_0000;
    const WS_MINIMIZEBOX: isize = 0x0002_0000;
    const WS_MAXIMIZEBOX: isize = 0x0001_0000;
    const WS_SYSMENU: isize = 0x0008_0000;
    const WS_POPUP: isize = 0x8000_0000u32 as isize;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_FRAMECHANGED: u32 = 0x0020;

    unsafe extern "system" {
        fn GetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32) -> isize;
        fn SetWindowLongPtrW(hwnd: *mut core::ffi::c_void, index: i32, value: isize) -> isize;
        fn SetWindowPos(
            hwnd: *mut core::ffi::c_void,
            after: *mut core::ffi::c_void,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
    }

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as *mut core::ffi::c_void;
    // SAFETY: `hwnd` is winit's live window handle; `GWL_STYLE` holds a
    // `LONG_PTR`, and the window is re-framed with `SWP_FRAMECHANGED` so the
    // non-client area is recomputed instead of being cached.
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let style = (style & !WS_POPUP) | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style);
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn apply_custom_chrome(window: &winit::window::Window) {
    // Other platforms have no caption to remove here; winit's frameless window
    // already keeps its own resize handling.
    let _ = window;
}
```

Call it from `create_window` immediately after `event_loop.create_window(...)`
returns and before the window is centred (the frame change alters the outer
size).

- [ ] **Step 5: Verify by hand and run the gate**

`cargo build --release`, launch, and confirm: no caption row; the strip sits
flush with the top; dragging a window edge resizes; dragging the window to a
screen edge snaps; the drop shadow is present; the taskbar button still reads
"Termyx".

- [ ] **Step 6: Commit**

`feat: make the tab strip the window title bar`

## Task 2: Painted minimise, maximise and close

**Files:** `src/ui/tab_bar.rs`, `src/render/mod.rs`, `src/app.rs`, `src/theme.rs`

**Interfaces:**
- Consumes: `platform::apply_custom_chrome` (Task 1).
- Produces: `TabBarLayout::{minimize, maximize, close_window}` rects;
  `TabHit::{Minimize, Maximize, CloseWindow}`; `StripHover::{Minimize,
  Maximize, CloseWindow}`; `StripAction::{MinimizeWindow, ToggleMaximize,
  CloseWindow}`; `App::toggle_maximized()`.

- [ ] **Step 1: Write the failing tests**

In `src/ui/tab_bar.rs`:

```rust
#[test]
fn the_caption_buttons_are_pinned_to_the_right_edge() {
    let layout = TabBarLayout::new(400.0, 32.0, &["one"], |_| 40.0);
    assert_eq!(layout.close_window.x + layout.close_window.w, layout.width);
    assert_eq!(layout.close_window.x, layout.maximize.x + layout.maximize.w);
    assert_eq!(layout.maximize.x, layout.minimize.x + layout.minimize.w);
    assert_eq!(layout.minimize.w, CAPTION_BUTTON_WIDTH);
}

#[test]
fn tabs_stop_before_the_caption_buttons() {
    let layout = TabBarLayout::new(1200.0, 32.0, &["one", "two", "three"], |_| 300.0);
    let last = layout.tabs.last().expect("a tab");
    assert!(layout.plus.x + layout.plus.w <= layout.minimize.x);
    assert!(last.rect.x + last.rect.w <= layout.plus.x);
}

#[test]
fn the_caption_buttons_stay_reachable_on_a_crowded_strip() {
    let titles: Vec<&str> = (0..30).map(|_| "a long shell title").collect();
    let layout = TabBarLayout::new(1200.0, 32.0, &titles, |_| 120.0);
    for rect in [layout.minimize, layout.maximize, layout.close_window] {
        assert!(rect.x >= 0.0 && rect.x + rect.w <= layout.width);
        assert!(layout.hit(rect.x + 1.0, rect.y + 1.0).is_some());
    }
}
```

In `src/app.rs`:

```rust
#[test]
fn clicking_a_caption_button_maps_to_its_window_action() {
    assert_eq!(strip_action(Some(TabHit::Minimize)), StripAction::MinimizeWindow);
    assert_eq!(strip_action(Some(TabHit::Maximize)), StripAction::ToggleMaximize);
    assert_eq!(strip_action(Some(TabHit::CloseWindow)), StripAction::CloseWindow);
}
```

- [ ] **Step 2: Run the tests and watch them fail**

`cargo test --all-targets` — expected: compile errors for the missing fields,
variants and constants.

- [ ] **Step 3: Extend the geometry**

In `src/ui/tab_bar.rs` add:

```rust
/// Width of one of the three window controls, matching the Windows caption.
pub const CAPTION_BUTTON_WIDTH: f32 = 46.0;
```

In `TabBarLayout::new`, before computing `available`, reserve the three
controls from the right:

```rust
let caption = (3.0 * CAPTION_BUTTON_WIDTH * k).min(width.max(0.0));
let controls_right = width;
let available = (width - buttons - caption).max(0.0);
```

and place them right to left from `controls_right`:

```rust
let close_window = Rect { x: controls_right - 46.0 * k, y: 0.0, w: 46.0 * k, h: strip_h };
let maximize     = Rect { x: close_window.x - 46.0 * k, y: 0.0, w: 46.0 * k, h: strip_h };
let minimize     = Rect { x: maximize.x     - 46.0 * k, y: 0.0, w: 46.0 * k, h: strip_h };
```

`hit` checks the controls BEFORE the tabs (they are never covered, but the
order keeps the rule explicit) and after the existing close/body checks so a
zero-width close box cannot shadow them.

- [ ] **Step 4: Paint them**

Add `Renderer::draw_caption_buttons(&mut self, layout, ui_px, hover)` called
after `draw_buttons` and after `draw_band_rule`, filling the hovered control
(`#c42b1c` for close, `theme.tab_hover` for the other two) and drawing the
glyphs `✕`, `▢`/`❐`, `—` with `draw_text_centred`.

Add the two theme tokens the glyph colours need — reuse the existing
`theme.tab_title` / `theme.tab_title_inactive` and add only:

```rust
/// Fill behind the close control while it is hovered; the Windows accent red.
pub caption_close_hover: Rgb,
```

- [ ] **Step 5: Route the presses**

`StripAction` gains `MinimizeWindow`, `ToggleMaximize`, `CloseWindow`.
`strip_action` maps the three new hits. `on_mouse`'s strip arm handles them:

```rust
StripAction::MinimizeWindow => { if let Some(window) = &self.window { window.set_minimized(true); } }
StripAction::ToggleMaximize => self.toggle_maximized(),
StripAction::CloseWindow => return true,
```

with

```rust
/// Maximises a normal window and restores a maximised one.
fn toggle_maximized(&self) {
    let Some(window) = &self.window else { return };
    window.set_maximized(!window.is_maximized());
}
```

`MinimizeWindow`/`ToggleMaximize` must also clear `strip_hover` (the pointer
is no longer over anything once the window moves) and request a redraw.

- [ ] **Step 6: Gate and commit**

`feat: paint the window controls into the tab strip`

## Task 3: Drag handle, DPI rebuild, and docs

**Files:** `src/app.rs`, `src/ui/tab_bar.rs`, `src/render/mod.rs`, `README.md`,
`docs/superpowers/specs/2026-11-07-termyx-mvp-design.md`

**Interfaces:**
- Produces: `App::chrome_press(&mut self, hit: Option<TabHit>) -> ChromePress`;
  `ScaleFactorChanged` handling in `window_event`.
- Consumes: Task 1's frameless window, Task 2's control rects.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn empty_strip_space_moves_the_window_but_a_tab_does_not() {
    assert_eq!(chrome_press(None), ChromePress::Drag);
    assert_eq!(chrome_press(Some(TabHit::Tab(0))), ChromePress::Tab);
    assert_eq!(chrome_press(Some(TabHit::Plus)), ChromePress::Tab);
    assert_eq!(chrome_press(Some(TabHit::Minimize)), ChromePress::Tab);
}
```

and, in `src/ui/tab_bar.rs`, a test that a press in the gap between the last
tab and the controls is *not* over any control:

```rust
#[test]
fn the_gap_before_the_caption_buttons_is_empty_space() {
    let layout = TabBarLayout::new(400.0, 32.0, &["one"], |_| 40.0);
    let x = layout.plus.x + layout.plus.w + 4.0;
    assert!(x < layout.minimize.x, "the test needs empty space to probe");
    assert_eq!(layout.hit(x, 16.0), None);
}
```

- [ ] **Step 2: Run them and watch them fail**

- [ ] **Step 3: Implement the drag handle**

In `on_mouse`'s strip arm, before the `StripAction` match:

```rust
if chrome_press(self.layout.as_ref().and_then(|l| {
    l.hit(self.cursor_pos.0 as f32, self.cursor_pos.1 as f32)
})) == ChromePress::Drag
{
    // Empty chrome space is the window's drag handle, the way a caption is.
    if button == MouseButton::Left {
        if self.double_clicked_strip() {
            self.toggle_maximized();
        } else if let Some(window) = &self.window {
            window.drag_window();
        }
    }
    return false;
}
```

`double_clicked_strip` compares the press position and
`std::time::Instant::now()` against an `App` field
`last_strip_click: Option<(Instant, (f64, f64))>`, using the existing
`DOUBLE_CLICK` interval constant where one exists (add `const STRIP_DOUBLE_CLICK:
Duration = Duration::from_millis(400)` otherwise). It must never touch
`mouse_down`/`click_count`/`last_click`, which belong to the grid.

- [ ] **Step 4: Rebuild on a scale change**

Add to `window_event`:

```rust
WindowEvent::ScaleFactorChanged { scale_factor, inner_size_writer: _ } => {
    self.on_scale_factor_changed(scale_factor);
}
```

`on_scale_factor_changed` reloads the font set at
`scaled_font_size(config.font.size, scale_factor)`, resets the renderer's top
inset to `tab_strip_px()`, rebuilds the layout, resizes the surface and every
session's grid, and requests a redraw. A font reload failure logs at error and
keeps the old font set.

- [ ] **Step 5: Document**

- `README.md`: a `## Window chrome` section stating that the tab strip is the
  title bar, that the three controls are painted, and the three out-of-scope
  items from the top of this plan.
- The spec's Phase 4/5 list gains a line for the custom chrome, marked done.
- Drop the "OS title bar" wording from any place that claims a caption.

- [ ] **Step 6: Gate, manual comparison, commit**

`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --all-targets`, `cargo build --release`, then launch the release
binary side by side with Windows Terminal and compare: strip at the very top,
no caption, controls in the correct order at the right end, tab hover and close
behaviour unchanged.

`feat: drag the window from the tab strip and rebuild it on a DPI change`

## Verification

- All four gates green; test count recorded in the ledger.
- Manual: resize from each edge and each corner; snap left/right by dragging to
  an edge; double-click the empty strip to maximise and to restore; minimise
  and restore from the taskbar; close with the painted `✕`; move the window to
  a 150 % monitor and confirm the strip, its controls and the grid all scale
  without overlapping.
