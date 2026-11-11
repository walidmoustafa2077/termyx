# Termyx Middle-Click Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a middle click paste the clipboard in the terminal grid and close the tab under the pointer in the tab strip, while leaving left- and right-click behavior unchanged.

**Architecture:** Two small changes in `src/app.rs`. A new `pastes_locally` predicate names the buttons that paste after the application-mouse forwarding branch declines the press. `strip_press` takes the pressed `MouseButton` instead of a `left` boolean and maps a middle press on `TabHit::Tab` to the existing `StripAction::Close`, so tab teardown reuses `close_tab`. No new files, state, or dependencies.

**Tech Stack:** Rust 2021, winit 0.30 mouse events, the existing `strip_press` table test, `arboard` through `App::paste_clipboard`, `alacritty_terminal` terminal modes.

**Spec:** `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` (additive to the mouse-input and Phase 4 tab-interaction behavior; it changes no rendering or grid invariant).

## Global Constraints

- The terminal grid is **logical** (write order). Nothing here reorders cells or rewrites PTY bytes.
- The UI thread owns the window, renderer, terminal, and interaction state; there is one PTY reader thread.
- Application mouse reporting still wins: in a reporting mode without Shift, a middle press is forwarded to the application and must not paste locally.
- No `unwrap`/`expect` in runtime paths; tests may unwrap.
- Small diffs only. Do not reformat or refactor unrelated code.
- TDD: write the failing test, run it, implement, run it, commit.
- Before claiming done: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`.
- Commit messages use `feat:`, `fix:`, `test:`, `docs:`, or `chore:`.
- Baseline note: `git status` currently shows an unrelated modification to `src/page.rs`. Leave it untouched and never stage it.

---

### Task 1: Middle-click pastes in the grid

**Files:**
- Modify: `src/app.rs` — the local paste branch in `App::on_mouse` (around lines 1523–1531), a new helper after `forwards_mouse_press` (around line 2616), and a new test after `mouse_press_forwards_only_in_reporting_modes_without_shift` (around line 2957).
- Modify: `README.md` — add a `## Mouse` section before `## Tabs` (line 90).

**Interfaces:**
- Consumes: `App::paste_clipboard()` (existing, no signature change), `forwards_mouse_press(mode: TermMode, shift: bool) -> bool` (existing), `MouseButton` from `winit::event`.
- Produces: `fn pastes_locally(button: MouseButton) -> bool` — true for `Right` and `Middle`, false for every other button. This is private to `src/app.rs`.

- [ ] **Step 1: Write the failing test**

Insert this test immediately after `mouse_press_forwards_only_in_reporting_modes_without_shift` in the `#[cfg(test)]` module of `src/app.rs`:

```rust
    #[test]
    fn middle_and_right_clicks_paste_in_the_grid() {
        for button in [MouseButton::Right, MouseButton::Middle] {
            assert!(pastes_locally(button), "{button:?} must paste locally");
        }
        for button in [MouseButton::Left, MouseButton::Back, MouseButton::Forward] {
            assert!(!pastes_locally(button), "{button:?} must not paste locally");
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test middle_and_right_clicks_paste_in_the_grid -- --nocapture`

Expected: FAIL to compile with `error[E0425]: cannot find function 'pastes_locally' in this scope`.

- [ ] **Step 3: Add the predicate**

Insert this function immediately after `forwards_mouse_press` in `src/app.rs`:

```rust
/// Buttons that paste the clipboard when the press stays local.
///
/// The forwarding branch in [`App::on_mouse`] runs first, so an application
/// that reports the mouse still receives a right or middle press. A local left
/// press keeps its selection behavior.
fn pastes_locally(button: MouseButton) -> bool {
    matches!(button, MouseButton::Right | MouseButton::Middle)
}
```

- [ ] **Step 4: Route the middle button through the paste branch**

Replace the right-click-only branch in `App::on_mouse`:

```rust
            // A right click that stays local pastes; the forwarding branch
            // above has already claimed the click for mouse-reporting apps.
            if button == MouseButton::Right {
                self.paste_clipboard();
                return false;
            }
```

with:

```rust
            // A right or middle click that stays local pastes; the forwarding
            // branch above has already claimed the click for mouse-reporting
            // apps.
            if pastes_locally(button) {
                self.paste_clipboard();
                return false;
            }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test middle_and_right_clicks_paste_in_the_grid -- --nocapture`

Expected: PASS — 1 passed; 0 failed.

- [ ] **Step 6: Document local paste**

Insert this section immediately before `## Tabs` in `README.md`:

```markdown
## Mouse

- Drag to select text; double-click selects a word and triple-click selects the
  line.
- Right-click or middle-click the grid pastes the clipboard.
- While an application reports the mouse, hold **Shift** to select locally.
```

- [ ] **Step 7: Verify formatting and commit**

Run: `cargo fmt --check`

Expected: no output and exit code 0. If it reports a diff, run `cargo fmt`, re-run `cargo fmt --check`, and include the formatting in this commit.

```bash
git add src/app.rs README.md
git commit -m "feat: paste the clipboard on a middle click"
```

---

### Task 2: Middle-click closes a tab

**Files:**
- Modify: `src/app.rs` — the `StripPress::Act` doc (around line 1931), `strip_press` (around lines 1943–1965), its call site in `App::on_mouse` (around lines 1286–1293), and the test module (the existing `strip_press_selects_one_action_for_every_press` table test around line 3241 plus two new tests after it).
- Modify: `README.md` — append one bullet to the `## Mouse` section added by Task 1.
- Modify: `AGENTS.md` — the `Current scope` `In:` list (around lines 61–62).

**Interfaces:**
- Consumes: `StripAction::Close(usize)` and `App::close_tab(index) -> bool` (both existing), `MouseButton` from `winit::event`.
- Produces: `fn strip_press(press: ChromePress, button: MouseButton, in_strip: bool, menu_open: bool, double: bool, action: Option<StripAction>) -> StripPress` — the second parameter changes from `left: bool` to the pressed button. A middle press on `ChromePress::Tab` with `StripAction::Select(index)` produces `StripPress::Act(StripAction::Close(index))`; every other non-left press produces `StripPress::Act(StripAction::Ignore)`.

- [ ] **Step 1: Write the failing tests**

Insert these two tests immediately after `strip_press_selects_one_action_for_every_press` in the `#[cfg(test)]` module of `src/app.rs`:

```rust
    #[test]
    fn a_middle_press_on_a_tab_body_closes_it() {
        assert_eq!(
            strip_press(
                ChromePress::Tab,
                MouseButton::Middle,
                true,
                false,
                false,
                Some(StripAction::Select(2))
            ),
            StripPress::Act(StripAction::Close(2))
        );
    }

    #[test]
    fn a_middle_press_off_a_tab_body_is_consumed_without_an_action() {
        for (press, action) in [
            (ChromePress::Drag, None),
            (ChromePress::Tab, Some(StripAction::Close(2))),
            (ChromePress::Tab, Some(StripAction::NewTab)),
            (ChromePress::Tab, Some(StripAction::OpenMenu)),
            (ChromePress::Tab, Some(StripAction::MinimizeWindow)),
            (ChromePress::Tab, Some(StripAction::ToggleMaximize)),
            (ChromePress::Tab, Some(StripAction::CloseWindow)),
        ] {
            assert_eq!(
                strip_press(press, MouseButton::Middle, true, false, false, action),
                StripPress::Act(StripAction::Ignore),
                "{press:?} with {action:?} must not run a left-button action"
            );
        }
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test a_middle_press -- --nocapture`

Expected: FAIL to compile with `error[E0061]: this function takes 6 arguments but 7 arguments were supplied` and a type mismatch on the second argument, because `strip_press` still takes `left: bool`.

- [ ] **Step 3: Change `strip_press` to take the pressed button**

Replace the whole existing `strip_press` function and its doc comment with:

```rust
/// Decides what one press inside the strip band means.
///
/// `action` is the painted target under the pointer, `button` is the button
/// that landed, `in_strip` says whether the point is inside the band at all,
/// and `menu_open` gives the dropdown priority over everything painted below
/// it. A left press on the empty band can drag or double-click, and a left
/// press on a painted target runs its action. A middle press on a tab body
/// closes that tab. Every other press belongs to chrome but carries no action,
/// so it is consumed rather than reaching the terminal.
fn strip_press(
    press: ChromePress,
    button: MouseButton,
    in_strip: bool,
    menu_open: bool,
    double: bool,
    action: Option<StripAction>,
) -> StripPress {
    if !in_strip || menu_open {
        return StripPress::Ignore;
    }
    if button != MouseButton::Left {
        // A middle press on a tab body closes that tab, the browser-style
        // gesture. A middle press on any other target - the close box, `+`,
        // `⌄`, a caption control, or the empty band - is still chrome's but
        // has no action. So is every right press. None may drag the window or
        // reach the grid.
        return StripPress::Act(match (button, action) {
            (MouseButton::Middle, Some(StripAction::Select(index))) => {
                StripAction::Close(index)
            }
            _ => StripAction::Ignore,
        });
    }
    if press == ChromePress::Drag {
        return if double {
            StripPress::ToggleMaximize
        } else {
            StripPress::Drag
        };
    }
    StripPress::Act(action.unwrap_or(StripAction::Ignore))
}
```

Update the `StripPress::Act` doc to read:

```rust
    /// A painted control: run its action. A non-left press in the band gets
    /// [`StripAction::Ignore`] (or a close for a middle press on a tab body),
    /// which consumes it without running a left-button action.
```

- [ ] **Step 4: Pass the button from the call site**

In `App::on_mouse`, replace:

```rust
            strip = strip_press(
                press,
                button == MouseButton::Left,
                in_strip,
                self.menu.is_some(),
                double,
                action,
            );
```

with:

```rust
            strip = strip_press(
                press,
                button,
                in_strip,
                self.menu.is_some(),
                double,
                action,
            );
```

- [ ] **Step 5: Update the existing table test**

Replace the whole `strip_press_selects_one_action_for_every_press` test with:

```rust
    #[test]
    fn strip_press_selects_one_action_for_every_press() {
        // Every branch the event loop can take is decided here: an empty-band
        // left press drags, its second press toggles, each painted control runs
        // its action, a right press is consumed without one, and a press
        // outside the band falls through to the grid.
        assert_eq!(
            strip_press(
                ChromePress::Drag,
                MouseButton::Left,
                true,
                false,
                false,
                None
            ),
            StripPress::Drag
        );
        assert_eq!(
            strip_press(
                ChromePress::Drag,
                MouseButton::Left,
                true,
                false,
                true,
                None
            ),
            StripPress::ToggleMaximize
        );

        let controls = [
            StripAction::Select(1),
            StripAction::Close(1),
            StripAction::NewTab,
            StripAction::OpenMenu,
            StripAction::MinimizeWindow,
            StripAction::ToggleMaximize,
            StripAction::CloseWindow,
            StripAction::Ignore,
        ];
        for action in controls {
            assert_eq!(
                strip_press(
                    ChromePress::Tab,
                    MouseButton::Left,
                    true,
                    false,
                    false,
                    Some(action)
                ),
                StripPress::Act(action),
                "a painted control must run {action:?}"
            );
        }

        // A press outside the strip is the grid's, whichever button and double
        // click state it carries.
        assert_eq!(
            strip_press(
                ChromePress::Drag,
                MouseButton::Left,
                false,
                false,
                true,
                None
            ),
            StripPress::Ignore
        );
        assert_eq!(
            strip_press(
                ChromePress::Tab,
                MouseButton::Middle,
                false,
                false,
                false,
                Some(StripAction::Select(1))
            ),
            StripPress::Ignore
        );

        // A right press on a painted control is chrome's but must not run the
        // left-button action.
        for press in [ChromePress::Drag, ChromePress::Tab] {
            assert_eq!(
                strip_press(
                    press,
                    MouseButton::Right,
                    true,
                    false,
                    false,
                    Some(StripAction::ToggleMaximize)
                ),
                StripPress::Act(StripAction::Ignore),
                "a right press must be consumed without a control action"
            );
        }

        // The dropdown owns the pointer even when a strip control is under it.
        assert_eq!(
            strip_press(
                ChromePress::Tab,
                MouseButton::Left,
                true,
                true,
                false,
                Some(StripAction::OpenMenu)
            ),
            StripPress::Ignore
        );
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test strip_press -- --nocapture`

Expected: PASS — `strip_press_selects_one_action_for_every_press`, `a_middle_press_on_a_tab_body_closes_it`, and `a_middle_press_off_a_tab_body_is_consumed_without_an_action` all pass.

- [ ] **Step 7: Document the gesture**

Append this bullet to the `## Mouse` section in `README.md`:

```markdown
- Middle-click a tab, active or not, closes it; the tab's `✕` does the same on
  a left click.
```

- [ ] **Step 8: Record the behavior in the agent scope**

In `AGENTS.md`, change this phrase in the `Current scope` `In:` list:

```markdown
selection, copy-on-select, right-click paste, application mouse reporting (DEC
```

to:

```markdown
selection, copy-on-select, right- and middle-click paste, middle-click tab
close, application mouse reporting (DEC
```

- [ ] **Step 9: Verify formatting and commit**

Run: `cargo fmt --check`

Expected: no output and exit code 0. If it reports a diff, run `cargo fmt`, re-run `cargo fmt --check`, and include the formatting in this commit.

```bash
git add src/app.rs README.md AGENTS.md
git commit -m "feat: close a tab with a middle click"
```

---

### Task 3: Full verification

**Files:**
- No source changes expected. If `cargo fmt` rewrites anything, stage it in the Task 2 commit instead of creating an empty commit.

**Interfaces:**
- Consumes: both tasks' changes.
- Produces: evidence that the working tree satisfies the project gates.

- [ ] **Step 1: Run the formatting gate**

Run: `cargo fmt --check`

Expected: no output and exit code 0.

- [ ] **Step 2: Run the lint gate**

Run: `cargo clippy --all-targets -- -D warnings`

Expected: `Finished` with no warnings.

- [ ] **Step 3: Run the whole test suite**

Run: `cargo test`

Expected: all tests pass, including the three new tests.

- [ ] **Step 4: Build the release binary**

Run: `cargo build --release`

Expected: `Finished` and `target\release\termyx.exe` (or the crate's configured binary name) exists.

- [ ] **Step 5: Exercise the gestures by hand**

Run: `cargo run`

Check each item:

1. Copy `PASTE-ME` to the clipboard, then middle-click the grid. Expected: the text is inserted at the prompt. A left click still selects, and a right click still pastes.
2. Open three tabs. Middle-click the middle tab while another tab is active. Expected: the middle tab closes, the active tab stays active, and no other tab's shell is touched.
3. Middle-click the active tab. Expected: it closes and a neighbour becomes active.
4. Middle-click the last remaining tab. Expected: the window exits, exactly as the last tab's `✕` does.
5. Middle-click the empty strip, `+`, `⌄`, a tab's `✕`, or a caption control. Expected: nothing happens; the window does not drag and the grid does not paste.
6. In a mouse-reporting application (for example `vim` with `:set mouse=a`), middle-click the grid. Expected: the application receives the press (Vim performs its own middle-click action); the local clipboard is not pasted by Termyx.

- [ ] **Step 6: Confirm the diff is scoped**

Run: `git status --short` and `git diff HEAD --stat`

Expected: only `src/app.rs`, `README.md`, `AGENTS.md`, and the new plan file are changed. The pre-existing `src/page.rs` modification remains unstaged and untouched.

---

## Self-review

- **Spec coverage:** The request has two outcomes. Task 1 covers grid middle-click paste; Task 2 covers middle-click tab close. Both reuse existing paths (`paste_clipboard`, `close_tab`) rather than adding parallel state.
- **Forwarding precedence:** The middle-paste branch sits after `forwards_mouse_press`, so DEC mouse modes still own the press; the existing mode test covers that gate and the manual check exercises it.
- **Button table:** The updated table test keeps left, right, outside-band, and dropdown cases, so the signature change cannot silently alter them.
- **No placeholders:** Every source change, test, command, and commit message is written out.
