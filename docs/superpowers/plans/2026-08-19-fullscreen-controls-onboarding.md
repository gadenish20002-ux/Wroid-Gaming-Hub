# Fullscreen Gameplay And Controls Onboarding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Launch preset-resolution Waydroid games as a scaled fullscreen surface and make keyboard/mouse control setup self-explanatory.

**Architecture:** `DesktopWaydroidSession` selects a Gamescope presentation only for package gameplay, while calibration and system UI remain direct Waydroid windows. The existing KWin relay recognizes package-specific Waydroid and Gamescope windows. Controls Studio derives a four-step guide from its existing calibration, selection, and test state.

**Tech Stack:** Rust, Waydroid, Gamescope 3.16+, KDE Plasma 6 KWin scripting, GTK/WebKitGTK 4.1, vanilla HTML/CSS/JavaScript, Node test runner.

## Global Constraints

- Preserve 1280x720, 1600x900, and 1920x1080 as Android render presets.
- Use aspect-fit FSR scaling; do not stretch the game viewport.
- Missing `/usr/bin/gamescope` must retain the direct Waydroid launch path.
- Calibration and non-game Android actions remain direct and windowed.
- F12 releases/reacquires input and Ctrl+Esc stops the managed session.
- Do not change Profile V2 or add gamepad support.

---

### Task 1: Package-Aware Focus Guard

**Files:**
- Modify: `crates/wroid-cli/src/commands/kwin_focus.rs`

**Interfaces:**
- Produces: `is_game_surface(window)` in the generated KWin script, matching `waydroid`, `waydroid.<package>`, and `gamescope`.

- [ ] **Step 1: Strengthen the existing script test so it fails**

Assert that `focus_script(":1.42")` contains prefix matching for
`waydroid.` and a Gamescope identity branch in
`script_matches_waydroid_by_stable_window_identity`.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p wroid-cli kwin_focus::tests::script_matches_waydroid_by_stable_window_identity -- --exact
```

Expected: FAIL because the script currently accepts only exact `waydroid`.

- [ ] **Step 3: Implement the minimal matcher**

Generate this identity logic inside `focus_script`:

```javascript
const waydroid = value => value === "waydroid" || value.startsWith("waydroid.");
return waydroid(windowClass) || waydroid(windowName) ||
    windowClass === "gamescope" || windowName === "gamescope";
```

Keep the existing D-Bus callback and focus wire format unchanged.

- [ ] **Step 4: Run the focused test and full crate tests**

```bash
cargo test -p wroid-cli kwin_focus::tests::script_matches_waydroid_by_stable_window_identity -- --exact
cargo test -p wroid-cli
```

- [ ] **Step 5: Commit**

```bash
git add crates/wroid-cli/src/commands/kwin_focus.rs
git commit -m "Input: recognize package game windows"
```

### Task 2: Gamescope Fullscreen Presentation

**Files:**
- Modify: `crates/wroid-inject/src/waydroid_session.rs`
- Modify: `crates/wroid-inject/src/game_session.rs`

**Interfaces:**
- Produces: `WaydroidPresentation::{Direct, Gamescope { width, height }}`.
- Produces: `DesktopWaydroidSession::start_presented(user, presentation)`.
- Consumes: the existing selected `GameSessionOptions.width` and `.height`.

- [ ] **Step 1: Add failing argument-policy tests**

Add tests proving:

```rust
assert_eq!(
    gamescope_arguments(1280, 720),
    ["-w", "1280", "-h", "720", "-f", "--expose-wayland",
     "--force-windows-fullscreen", "-S", "fit", "-F", "fsr",
     "--sharpness", "5", "--", "/usr/bin/waydroid", "session", "start"]
);
assert_eq!(presentation_for_game(true, true, 1280, 720),
           WaydroidPresentation::Gamescope { width: 1280, height: 720 });
assert_eq!(presentation_for_game(false, true, 1280, 720),
           WaydroidPresentation::Direct);
```

The first boolean is `launch_package`; the second is Gamescope availability.

- [ ] **Step 2: Run the new tests and verify RED**

```bash
cargo test -p wroid-inject waydroid_session::tests::gamescope -- --nocapture
cargo test -p wroid-inject game_session::tests::package_game_uses_fullscreen_presentation -- --exact
```

Expected: compile failure because the presentation API is absent.

- [ ] **Step 3: Implement typed presentation selection**

Add the enum and pure selectors. Gamescope availability is an executable-file
check for `/usr/bin/gamescope`. `launch_package == false` always selects direct.
Print one explicit line for the chosen presentation.

- [ ] **Step 4: Implement desktop-user command construction**

Refactor `DesktopUser::command` through one private program builder so both
current-user and `runuser` launches receive the same `HOME`,
`XDG_RUNTIME_DIR`, D-Bus, and outer `WAYLAND_DISPLAY`. In Gamescope mode the
primary program is `/usr/bin/gamescope` and its child is
`/usr/bin/waydroid session start`; in direct mode preserve the current command.

- [ ] **Step 5: Route only package gameplay through presentation**

In `run_game_session`, compute the presentation from
`options.launch_package`, availability, width, and height before starting the
desktop session. Do not change `open_game_for_calibration` or `show_full_ui`.

- [ ] **Step 6: Run unit and workspace tests**

```bash
cargo test -p wroid-inject
cargo test -p wroid-cli
cargo test --workspace
```

- [ ] **Step 7: Commit**

```bash
git add crates/wroid-inject/src/waydroid_session.rs crates/wroid-inject/src/game_session.rs
git commit -m "Runtime: scale game sessions through Gamescope"
```

### Task 3: Controls Studio Quick Setup Strip

**Files:**
- Create: `crates/wroid-cli/assets/editor/setup-guide.js`
- Create: `crates/wroid-cli/assets/editor/setup-guide.test.js`
- Modify: `crates/wroid-cli/assets/editor/index.html`
- Modify: `crates/wroid-cli/assets/editor/styles.css`
- Modify: `crates/wroid-cli/assets/editor/app.js`
- Modify: `crates/wroid-cli/src/commands/editor.rs`

**Interfaces:**
- Produces: `window.WroidSetupGuide.steps({ backgroundSaved, selected, testing, dirty })`.
- Consumes: existing `captureWindow`, `setTesting`, and `saveProfile` actions.

- [ ] **Step 1: Write the failing setup-state tests**

Cover the four derived steps:

```javascript
assert.deepEqual(steps({ backgroundSaved: false, selected: -1, testing: false, dirty: false })
  .map(step => step.state), ["active", "pending", "pending", "pending"]);
assert.deepEqual(steps({ backgroundSaved: true, selected: 2, testing: true, dirty: true })
  .map(step => step.state), ["done", "done", "active", "pending"]);
```

- [ ] **Step 2: Run the test and verify RED**

```bash
node --test crates/wroid-cli/assets/editor/setup-guide.test.js
```

Expected: FAIL because `setup-guide.js` does not exist.

- [ ] **Step 3: Implement the pure guide model**

Export four labeled steps for capture, placement/binding, local test, and save.
Use UMD style matching `profile-model.js` so Node and the WebView share it.

- [ ] **Step 4: Add the accessible setup strip**

Insert `#setupGuide` above `.workspace-toolbar` with four buttons, short copy,
and status marks. Add `setup-guide.js` before `app.js`. Style it as a compact
horizontal sequence that wraps below 1200px and does not cover the viewport.

- [ ] **Step 5: Wire existing actions and render state**

Cache the new nodes in `elements`; call the pure model from `renderAll`. Capture
invokes `captureWindow`, test invokes `setTesting(true)`, and save invokes
`saveProfile(true)`. Placement focuses the first binding when none is selected
and scrolls the inspector into view. Change the main labels to `1. Capture game
window`, `3. Test bindings`, and `4. Save & play`.

- [ ] **Step 6: Serve the new static module**

Embed `setup-guide.js` in `editor.rs`, add its authenticated GET route, and add
it to the static-asset route tests.

- [ ] **Step 7: Run JS and Rust tests**

```bash
node --test crates/wroid-cli/assets/editor/setup-guide.test.js crates/wroid-cli/assets/editor/profile-model.test.js
cargo test -p wroid-cli commands::editor
```

- [ ] **Step 8: Commit**

```bash
git add crates/wroid-cli/assets/editor crates/wroid-cli/src/commands/editor.rs
git commit -m "Controls: add guided keyboard setup"
```

### Task 4: Hub Gameplay Guidance

**Files:**
- Modify: `crates/wroid-cli/assets/hub/control-chips.js`
- Modify: `crates/wroid-cli/assets/hub/control-chips.test.js`
- Modify: `crates/wroid-cli/assets/hub/index.html`
- Modify: `crates/wroid-cli/assets/hub/app.js`
- Modify: `crates/wroid-cli/assets/hub/styles.css`

**Interfaces:**
- Produces: `controlQuickStart(game)` returning binding summary and safety keys.

- [ ] **Step 1: Add a failing Standoff quick-start test**

Assert that the helper returns the shipped Standoff bindings and the global
`F12 — release input` and `Ctrl+Esc — stop game` hints.

- [ ] **Step 2: Run and verify RED**

```bash
node --test crates/wroid-cli/assets/hub/control-chips.test.js
```

- [ ] **Step 3: Implement and render the quick-start card**

Add a compact card to the Controls section. Derive text from the selected
profile kind; do not claim a binding absent from the starter. Render the card
on every `renderHero` selection change and show `Fullscreen scaling · FSR` in
the launch note when package gameplay is ready.

- [ ] **Step 4: Run Hub tests**

```bash
node --test crates/wroid-cli/assets/hub/control-chips.test.js crates/wroid-cli/assets/hub/compatibility-state.test.js
```

- [ ] **Step 5: Commit**

```bash
git add crates/wroid-cli/assets/hub
git commit -m "Hub: explain game controls before launch"
```

### Task 5: Verification, Installation, And Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/roadmap.md`
- Modify: `docs/game-compatibility.md`

**Interfaces:**
- Consumes: completed fullscreen and onboarding behavior.

- [ ] **Step 1: Run all automated gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
node --test crates/wroid-cli/assets/editor/*.test.js crates/wroid-cli/assets/hub/*.test.js
git diff --check
```

- [ ] **Step 2: Build and install the release**

```bash
cargo build --release --workspace
target/release/wroid desktop install
```

Use the existing helper installer only if the staged release reports it as
outdated; do not change root or anti-cheat state.

- [ ] **Step 3: Verify live fullscreen**

Stop the current managed test session, launch Standoff from Hub with Speed
1280x720, and inspect KWin state. The outer Gamescope window must be fullscreen
at 1920x1080 on DP-2 while Android `wm size` remains 1280x720. Focus and unfocus
the surface and verify the relay toggles device capture without stopping the
session.

- [ ] **Step 4: Verify Controls Studio**

Open `Open game & calibrate`; verify the setup strip opens, capture the live
Standoff window, select a marker, bind a key, run local input preview, and close
without modifying the user's map unless an intentional save is made.

- [ ] **Step 5: Update docs and commit**

Document Gamescope fullscreen, direct fallback, the four control steps, and
the in-game safety keys. Mark the fullscreen/onboarding roadmap slice complete.

```bash
git add README.md docs/roadmap.md docs/game-compatibility.md
git commit -m "Docs: explain fullscreen control workflow"
```

- [ ] **Step 6: Push main**

```bash
git status --short --branch
git push origin main
```
