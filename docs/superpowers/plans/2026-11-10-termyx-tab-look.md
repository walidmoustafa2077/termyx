# Termyx — Tab Strip Visual Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the tab strip look like the Windows Terminal reference: every tab carries the app icon and a readable title, tabs hug their content, the title clips with an ellipsis, tabs have a top/side border and rounded top corners, and the close glyph behaves like the reference (always on the active tab, on hover elsewhere).

**Architecture:** No new module. `src/session.rs` keeps the label store. `src/ui/tab_bar.rs` keeps the geometry (pure, unit-tested) and changes only its width constants. `src/render/mod.rs` keeps `draw_chrome` and gains three small painting helpers (rounded-top fill, RGBA blit, ellipsis text). `src/render/ui_text.rs` gains `draw_text_ellipsis`. `src/icon.rs` exposes the already-bundled 256×256 RGBA icon so the renderer can downsample it — no new dependency and no new asset.

**Why this is a plan and not a patch:** the shipped strip deviates from the plan's own Visual target in five ways (no icon, no ellipsis, `TITLE_SIZE = 120` reserves three times the chrome a tab needs, no tab borders, `✕` only on the active tab), and the title is often the ConPTY executable path rather than a tab name. Each fix has a testable seam.

**Spec:** `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` §12 (Phase 4) plus this plan's Visual target, which is copied from the reference screenshot.

## Global Constraints

- One window, many sessions; a session is one `Terminal` plus one `PtySession`.
- The terminal grid stays logical; nothing in this plan reorders cells. PTY bytes are fed verbatim.
- The UI thread owns the window, renderer, sessions, and interaction state.
- Errors at the edge: `thiserror` per module, `anyhow` in the app. No `unwrap`/`expect` outside tests. A missing icon is a logged warning, never a panic.
- No dead code. `#[allow(dead_code)]` is forbidden. An item whose only consumer is a test is `#[cfg(test)]` with a why-comment naming the consuming task.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, `cargo build --release` must pass before each commit.
- Commit messages use `feat:`, `fix:`, `test:`, `docs:`.
- Measured M5 budgets must not regress: 200×50 full frame < 8 ms. The icon blit is at most 24×24 pixels per tab; resample it at most once per tab per frame.
- Every geometry constant stays physical-pixel aware: `k = strip_h / TAB_STRIP_HEIGHT` inside `TabBarLayout::new` and inside `draw_chrome`, and chrome text is drawn at `UI_SIZE_PX * window.scale_factor()`.

## Visual target (authoritative, from the reference screenshot)

Per tab, left to right, inside a rounded-top rectangle:

1. `12` logical px left padding, the `16` px app icon, `6` px gap.
2. The title in Segoe UI at `12` px: `#ffffff` on the active tab, `#cccccc` elsewhere, clipped with `…`.
3. `6` px gap, the `✕` close glyph at `10` px: always on the active tab, on hover for any other tab, on a `#333333` rounded square while hovered.
4. `12` px right padding.

Tab fill: active `#1f1f1f`, hover `#1a1a1a`, otherwise the strip `#0c0c0c`, with a `1` px `#2b2b2b` border on the top and both sides and an `8` px top corner radius. Width hugs `width_of(title) + 56` logical px, capped at `220` and floored at `80`; on a crowded strip tabs shrink below the floor so `+` and `⌄` stay reachable.

After the last tab: `+` and `⌄`, each `32` px wide, glyph horizontally centred, `#1f1f1f` fill while hovered. A `1` px `#2b2b2b` rule spans the window at the strip's bottom edge.

## File structure

- `src/session.rs` — the label store and the accept/reject rule for shell titles.
- `src/ui/tab_bar.rs` — pure tab geometry: widths, close rects, button rects.
- `src/icon.rs` — decode the bundled PNG once, expose RGBA bytes.
- `src/render/mod.rs` — `draw_chrome` and its painting helpers.
- `src/render/ui_text.rs` — chrome text and the ellipsis variant.

---

### Task 1: Give every tab an identity

**Files:**
- Modify: `src/session.rs`, `src/icon.rs`, `src/render/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `session::{TabSet, spawn_session}`, `icon::window_icon`, `render::ui_text::draw_text`.
- Produces: `icon::rgba() -> anyhow::Result<(Vec<u8>, u32)>`; `session::title_is_usable(&str) -> bool`; `Renderer::tab_icon` (loaded once in `Renderer::new`); the tab icon painted in `draw_chrome`.

- [ ] **Step 1: Write the failing tests**

In `src/icon.rs` `mod tests`:

```rust
    #[test]
    fn the_bundled_icon_is_available_as_rgba() {
        let (rgba, size) = rgba().expect("decode the bundled icon");
        assert_eq!(size, 256);
        assert_eq!(rgba.len(), (size * size * 4) as usize);
        assert!(rgba.chunks_exact(4).any(|px| px[3] > 0), "the icon is not transparent everywhere");
    }
```

In `src/session.rs` `mod tests`:

```rust
    #[test]
    fn a_conpty_executable_path_is_not_a_tab_title() {
        assert!(!title_is_usable(r"C:\WINDOWS\System32\WindX"));
        assert!(!title_is_usable(r"\\?\C:\Windows\System32\cmd.exe"));
        assert!(!title_is_usable("   "));
        assert!(!title_is_usable(""));
        assert!(title_is_usable("Windows PowerShell"));
        assert!(title_is_usable("OC | Next steps"));
        assert!(title_is_usable("nvim src/app.rs"));
    }
```

In `src/render/mod.rs` `mod tests`:

```rust
    #[test]
    fn the_active_tab_paints_its_app_icon() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, 32.0, &["one"], |_| 100.0);
        renderer.draw_chrome(&layout, 12.0, 0, None);

        // The icon slot starts 12 logical px into the tab and is 16 px square.
        let icon_pixel = pixel_at(&renderer, layout.tabs[0].rect.x + 20.0, 16.0);
        assert_ne!(icon_pixel, renderer.theme.tab_active, "the icon slot is empty");
    }
```

- [ ] **Step 2: Run the tests and see them fail**

`cargo test --all-targets icon:: session::tests::a_conpty render::tests::the_active_tab_paints_its_app_icon` — the first two fail to compile (`rgba`, `title_is_usable` missing), the third fails on an empty slot.

- [ ] **Step 3: Expose the icon bytes**

`src/icon.rs`: rename the existing private decoder's body into `pub fn rgba() -> anyhow::Result<(Vec<u8>, u32)>` and keep `window_icon()` calling it:

```rust
/// The bundled app icon as straight RGBA8 rows, top row first.
///
/// Decoded on demand: `window_icon` wants winit's own buffer type and the tab
/// strip wants raw bytes to downsample, so both build from one decoder.
pub fn rgba() -> anyhow::Result<(Vec<u8>, u32)> {
    decode_png(ICON_PNG)
}
```

- [ ] **Step 4: Reject a shell title that is really a program path**

`src/session.rs`:

```rust
/// Whether an OSC 0/2 title can label a tab.
///
/// ConPTY gives the child no title of its own, so a shell that never sets one
/// leaves the executable path in the title. Windows Terminal hides that path
/// behind the profile name; we drop the title and keep the profile name.
pub fn title_is_usable(title: &str) -> bool {
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    // `C:\...`, `\\?\C:\...` and `\\server\share\...` are program paths, not names.
    let path_like = title.starts_with(r"\\")
        || (title.len() >= 3
            && title.as_bytes()[1] == b':'
            && (title.as_bytes()[2] == b'\\' || title.as_bytes()[2] == b'/'));
    !path_like
}
```

Call it from `TabSlots::set_title`: return `false` without storing when `title_is_usable` rejects the input, so the profile-name label survives a ConPTY path.

- [ ] **Step 5: Label a new tab with its profile name**

`src/app.rs` `new_tab`, after `self.tabs.push(session)`:

```rust
        // The tab is named for the profile until the shell names itself.
        self.tabs.set_title(id, &profile.name);
```

- [ ] **Step 6: Load the icon once per renderer**

`src/render/mod.rs`:

```rust
/// The bundled app icon, kept as RGBA8 so each tab can be stamped from it.
pub(crate) struct TabIcon {
    rgba: Vec<u8>,
    size: u32,
}
```

`Renderer` gains `tab_icon: Option<TabIcon>`. `Renderer::new` fills it:

```rust
        let tab_icon = match crate::icon::rgba() {
            Ok((rgba, size)) => Some(TabIcon { rgba, size }),
            Err(error) => {
                tracing::warn!(%error, "no tab icon; the strip will draw without one");
                None
            }
        };
```

- [ ] **Step 7: Paint the icon**

`src/render/mod.rs`, a private helper beside `fill_rect`:

```rust
    /// Blits `icon` into a `size` px square at `(x, y)`, averaging each source
    /// block so a 256 px icon stays smooth at 16 px.
    fn blit_icon(&mut self, icon: &TabIcon, x: f32, y: f32, size: f32) {
        // ...
    }
```

Then in `draw_titles`, before the title text:

```rust
        let icon_size = 16.0 * k;
        let icon_x = tab.rect.x + tab_padding;
        let icon_y = (layout.strip_h - icon_size) / 2.0;
        if let Some(icon) = &self.tab_icon {
            self.blit_icon(icon, icon_x, icon_y, icon_size);
        }
```

`title_left` becomes `tab_padding + icon_size + 6.0 * k` so the title starts after the icon. Every tab draws its icon, active or not.

- [ ] **Step 8: Run the gate and commit**

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && cargo build --release`

```bash
git add -A && git commit -m "feat: give every tab an icon and a name"
```

---

### Task 2: Make the tab chrome match the reference

**Files:**
- Modify: `src/ui/tab_bar.rs`, `src/render/mod.rs`, `src/render/ui_text.rs`

**Interfaces:**
- Consumes: `ui::tab_bar::{TabBarLayout, TAB_STRIP_HEIGHT}`, `render::ui_text::{draw_text, draw_text_ellipsis}`.
- Produces: `ui_text::draw_text_ellipsis(renderer, text, x, y, px, color, max_width) -> f32`; `TabBarLayout` widths that hug the title; rounded-top tab fills and borders in `draw_chrome`.

- [ ] **Step 1: Write the failing tests**

In `src/ui/tab_bar.rs` `mod tests`:

```rust
    #[test]
    fn a_short_title_hugs_its_text() {
        let layout = TabBarLayout::new(1200.0, 32.0, &["one"], |_| 30.0);
        assert_eq!(layout.tabs[0].rect.w, 30.0 + TITLE_SIZE);
    }

    #[test]
    fn a_narrow_tab_still_leaves_room_for_every_chrome_piece() {
        assert!(MIN_TAB_WIDTH >= TITLE_SIZE);
    }
```

In `src/render/ui_text.rs` `mod tests`:

```rust
    #[test]
    fn a_clipped_title_ends_with_an_ellipsis() {
        // A title far wider than the clip must be shortened, not cut mid-glyph,
        // and the ellipsis must sit wholly inside the width.
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(400.0 as u32, 120);
        let width = draw_text_ellipsis(
            &mut renderer, "a very long tab title that cannot fit", 4.0, 4.0, 12.0,
            renderer.theme.foreground, 40.0,
        );
        assert!(width <= 40.0 + 0.01);
        assert!(renderer.pixels.iter().any(|p| p.chunks_exact(4).any(|px| px[3] > 0)));
    }
```

In `src/render/mod.rs` `mod tests`:

```rust
    #[test]
    fn an_inactive_tab_shows_its_close_glyph_only_when_hovered() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, 32.0, &["one", "two"], |_| 100.0);
        renderer.draw_chrome(&layout, 12.0, 0, None);
        let plain = pixel_at(&renderer, layout.tabs[1].close.x + 8.0, 16.0);
        renderer.draw_chrome(&layout, 12.0, 0, Some(crate::ui::tab_bar::StripHover::Tab(1)));
        let hovered = pixel_at(&renderer, layout.tabs[1].close.x + 3.0, 16.0);
        assert_eq!(plain, renderer.theme.tab_bar);
        assert_ne!(hovered, renderer.theme.tab_bar);
    }

    #[test]
    fn every_tab_has_its_top_border() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, 32.0, &["one", "two"], |_| 100.0);
        renderer.draw_chrome(&layout, 12.0, 0, None);
        assert_eq!(pixel_at(&renderer, layout.tabs[1].rect.x + 50.0, 0.0), renderer.theme.tab_border);
    }
```

- [ ] **Step 2: Run the tests and see them fail**

`cargo test --all-targets tab_bar::tests::a_short_title_hugs ui_text::tests::a_clipped_title render::tests::an_inactive_tab render::tests::every_tab_has`

- [ ] **Step 3: Make the width hug the title**

`src/ui/tab_bar.rs`:

```rust
/// Logical px a tab spends on chrome: left padding, icon, gap, gap, close, padding.
pub const TITLE_SIZE: f32 = 56.0;
/// A tab narrower than this cannot show an icon, a glyph and a close box.
pub const MIN_TAB_WIDTH: f32 = 80.0;
```

Update `tabs_are_laid_out_left_to_right_and_are_capped` to assert `tabs[0].rect.w == MAX_TAB_WIDTH` with a `width_of` of `200.0`, and add a second assertion in the same test that a `width_of` of `30.0` yields `30.0 + TITLE_SIZE`.

- [ ] **Step 4: Add the ellipsis text helper**

`src/render/ui_text.rs`:

```rust
/// Draws `text` clipped to `max_width`, replacing the cut tail with `…`.
///
/// Tab titles come from the shell and are arbitrarily long; a hard clip hides
/// the end of the name, which is the part a user navigates by.
pub fn draw_text_ellipsis(
    renderer: &mut Renderer,
    text: &str,
    x: f32,
    y: f32,
    px: f32,
    color: Rgb,
    max_width: f32,
) -> f32 {
    // ...
}
```

Shorten by `char` boundaries until `ui_width(shortened + "…")` fits, then draw. Use it in `draw_titles` and for menu labels.

- [ ] **Step 5: Rounded top corners and borders**

Add to `src/render/mod.rs`:

```rust
    /// Fills `rect` with its top two corners rounded by `radius`, leaving the
    /// bottom corners square so a tab meets the strip's rule flush.
    fn fill_round_top(&mut self, rect: Rect, radius: f32, color: Rgb) {
        // ...
    }
```

Use `fill_round_top(tab.rect, 8.0 * k, fill)` for tab fills, then stroke the top edge and both sides one pixel wide with `theme.tab_border`:

- top: `(rect.x + radius, rect.y, rect.w - 2 * radius, 1)`
- left: `(rect.x, rect.y + radius, 1, rect.h - radius)`
- right: `(rect.x + rect.w - 1, rect.y + radius, 1, rect.h - radius)`

The `+` and `⌄` hover fill becomes `fill_round_top(rect, 4.0 * k, theme.tab_active)`.

- [ ] **Step 6: Close glyph on the active and the hovered tab**

In `draw_titles`, replace the active-only guard:

```rust
        let tab_hovered = hover == Some(crate::ui::tab_bar::StripHover::Tab(tab.index));
        let shows_close = tab.index == active || tab_hovered;
```

Paint the `#333333` rounded square behind `✕` when `tab_hovered`, then the glyph in `theme.foreground` for both states (the reference uses one colour). Centre the glyph in `tab.close` with `ui_width` and the em size instead of the `+ 3, + 1` nudge.

- [ ] **Step 7: Centre the `+` and `⌄` glyphs**

In `draw_buttons`, measure the glyph with `self.fonts().ui_width(glyph, ui_px)` and use `x + (rect.w - glyph_w) / 2.0`, `y + (rect.h - ui_px) / 2.0`; drop the `inset_x`/`top` constants.

- [ ] **Step 8: Run the gate and commit**

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets && cargo build --release`

```bash
git add -A && git commit -m "feat: match the tab strip to the Windows Terminal reference"
```

---

## Verification

- Launch `target/release/termyx.exe` with two tabs open and compare against the reference screenshot: icon, name, hugging width, ellipsis, `✕` states, `+`/`⌄` position, bottom rule.
- Confirm the strip still owns its presses (Task 9's rules) and the menu still opens under `⌄`.
- Record the comparison in `README.md` under the tabs section when Task 12 of the Phase 4 plan writes it.
