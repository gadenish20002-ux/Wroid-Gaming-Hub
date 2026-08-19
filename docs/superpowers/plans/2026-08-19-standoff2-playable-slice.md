# Standoff 2 Playable Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Block known games when active Magisk is detected, surface the remediation in Hub, and verify the existing Standoff 2 keyboard/mouse path on the reference host.

**Architecture:** The existing compatibility report gains a pure root classifier fed by one fixed active-overlay probe and the optional Android package inventory. Both direct and Hub launches call one known-game readiness guard before changing Waydroid state. A tiny browser-side compatibility helper makes active root a disabled primary action without adding polling or automatic root removal.

**Tech Stack:** Rust 2021, serde_json, existing Waydroid command adapter, vanilla JavaScript UMD modules, Node.js built-in assertions, Cargo tests.

## Global Constraints

- Detect proven active Magisk from `/var/lib/waydroid/overlay/system/etc/init/magisk`; retain manager-only packages as non-blocking not-detected evidence.
- Do not classify `waydroid.host_data_path/adbroot` or stale app-data directories as active root.
- Do not hide root, spoof integrity, modify game files, or automatically execute a root-removal command.
- Unknown root state is a non-blocking warning; detected root blocks known-game launch before Waydroid teardown or physical input capture.
- Custom packages retain their existing launch behavior.
- Existing daemon/helper/input hot paths and their performance characteristics remain unchanged.
- Production changes follow strict red-green TDD.

---

### Task 1: Root classification and shared launch guard

**Files:**
- Modify: `crates/wroid-cli/src/commands/compatibility.rs`
- Modify: `crates/wroid-cli/src/commands/launch_v2.rs`

**Interfaces:**
- Produces: private `RootMarkerProbe::{Present, Absent, Unknown}`.
- Produces: private `RootAccessState::{Detected, NotDetected, Unknown}` with `as_str() -> &'static str`.
- Produces: private `RootAccess { state: RootAccessState, evidence: Option<&'static str> }`.
- Produces: private `classify_root_access(marker: RootMarkerProbe, installed_packages: Option<&[String]>) -> RootAccess`.
- Produces: `CompatibilityReport::ensure_known_game_launch_ready(&self, package: &str) -> Result<()>`.
- Extends: compatibility JSON with `rootAccess.state`, `rootAccess.evidence`, and `rootAccess.detail`.

- [ ] **Step 1: Add failing classifier and JSON tests**

Add focused tests to `compatibility.rs`:

```rust
#[test]
fn active_magisk_overlay_requires_action() {
    let access = classify_root_access(RootMarkerProbe::Present, Some(&packages(&[
        "com.android.settings",
    ])));
    assert_eq!(access.state, RootAccessState::Detected);
    assert_eq!(access.evidence, Some("magisk_overlay"));
}

#[test]
fn manager_package_without_overlay_is_not_detected_and_non_blocking() {
    for package in ["io.github.huskydg.magisk", "com.topjohnwu.magisk"] {
        let access = classify_root_access(
            RootMarkerProbe::Absent,
            Some(&packages(&["com.android.settings", package])),
        );
        assert_eq!(access.state, RootAccessState::NotDetected);
        assert_eq!(access.evidence, Some("magisk_manager_package_only"));
    }
}

#[test]
fn absent_active_signals_are_clean_even_when_stale_data_exists_elsewhere() {
    let access = classify_root_access(
        RootMarkerProbe::Absent,
        Some(&packages(&["com.android.settings"])),
    );
    assert_eq!(access.state, RootAccessState::NotDetected);
    assert_eq!(access.evidence, None);
}

#[test]
fn incomplete_root_evidence_stays_unknown() {
    assert_eq!(
        classify_root_access(RootMarkerProbe::Unknown, Some(&packages(&[]))).state,
        RootAccessState::Unknown,
    );
    assert_eq!(
        classify_root_access(RootMarkerProbe::Absent, None).state,
        RootAccessState::Unknown,
    );
}
```

Extend the report fixture with `root_marker: RootMarkerProbe::Absent`. Add an
active-root report assertion that `health()` is `action_required`, JSON contains
`rootAccess.state == "detected"`, and finding code is `android-root-detected`.

- [ ] **Step 2: Add failing launch-guard tests**

Build a report with installed Standoff 2 and `root_marker: Present`, then assert:

```rust
let error = report
    .ensure_known_game_launch_ready("com.axlebolt.standoff2")
    .unwrap_err();
assert!(error.to_string().contains("Magisk"));
assert!(report
    .ensure_known_game_launch_ready("com.example.custom")
    .is_ok());
```

- [ ] **Step 3: Run focused tests and verify RED**

```bash
cargo test -p wroid-cli commands::compatibility::tests -- --nocapture
```

Expected: compilation fails because the root types, classifier, probe field,
JSON output, and shared guard do not exist.

- [ ] **Step 4: Implement the minimal root model and filesystem adapter**

Use a fixed marker path and distinguish absence from an unreadable probe:

```rust
const MAGISK_OVERLAY: &str = "/var/lib/waydroid/overlay/system/etc/init/magisk";
const MAGISK_PACKAGES: [&str; 2] = ["io.github.huskydg.magisk", "com.topjohnwu.magisk"];

fn probe_magisk_overlay(path: &Path) -> RootMarkerProbe {
    match fs::symlink_metadata(path) {
        Ok(_) => RootMarkerProbe::Present,
        Err(error) if error.kind() == io::ErrorKind::NotFound => RootMarkerProbe::Absent,
        Err(_) => RootMarkerProbe::Unknown,
    }
}
```

Classification precedence is overlay, incomplete unknown, manager-package-only
not detected, and finally clean. Add an
action finding with code `android-root-detected` and removal guidance, or a
warning with code `android-root-unknown`. Include the typed root
object in `CompatibilityReport` and `as_json()`.

- [ ] **Step 5: Implement the shared known-game guard**

`ensure_known_game_launch_ready` returns immediately for packages not present
in `game_catalog`. For a known game it rejects `Detected` with the root detail,
then delegates to the existing package-installed validation. Replace the
`launch_v2` call to `ensure_package_installed_if_known` with the new guard.

- [ ] **Step 6: Run focused and crate tests to verify GREEN**

```bash
cargo test -p wroid-cli commands::compatibility::tests -- --nocapture
cargo test -p wroid-cli commands::launch_v2::tests -- --nocapture
cargo test -p wroid-cli
```

Expected: all commands exit 0.

- [ ] **Step 7: Commit**

```bash
git add crates/wroid-cli/src/commands/compatibility.rs crates/wroid-cli/src/commands/launch_v2.rs
git commit -m "CLI: block rooted known-game sessions"
```

### Task 2: Hub root blocker and remediation

**Files:**
- Create: `crates/wroid-cli/assets/hub/compatibility-state.js`
- Create: `crates/wroid-cli/assets/hub/compatibility-state.test.js`
- Modify: `crates/wroid-cli/assets/hub/index.html`
- Modify: `crates/wroid-cli/assets/hub/app.js`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Consumes: `system.compatibility.rootAccess` from Task 1.
- Produces: browser/global and CommonJS API `WroidHubCompatibility.activeRootFinding(compatibility) -> finding | null`.
- Consumes: `CompatibilityReport::ensure_known_game_launch_ready(&self, package: &str) -> Result<()>` in the Hub launch handler.

- [ ] **Step 1: Write the failing JavaScript helper test**

```javascript
const assert = require("node:assert/strict");
const { activeRootFinding } = require("./compatibility-state.js");

const finding = activeRootFinding({
  rootAccess: { state: "detected" },
  findings: [
    { code: "android-root-detected", severity: "action", message: "Remove Magisk" },
  ],
});
assert.equal(finding.message, "Remove Magisk");
assert.equal(activeRootFinding({ rootAccess: { state: "not_detected" }, findings: [] }), null);
assert.equal(activeRootFinding({ rootAccess: { state: "unknown" }, findings: [] }), null);
```

- [ ] **Step 2: Run the helper test and verify RED**

```bash
node crates/wroid-cli/assets/hub/compatibility-state.test.js
```

Expected: FAIL because `compatibility-state.js` does not exist.

- [ ] **Step 3: Implement the pure compatibility helper**

Use the same UMD wrapper as `control-chips.js`. Return the
`android-root-detected` finding only when `rootAccess.state === "detected"`;
otherwise return `null`.

- [ ] **Step 4: Load and use the helper in Hub**

Add a `COMPATIBILITY_STATE_JS` include, authenticated GET route
`/compatibility-state.js`, and script tag before `app.js`. In `primaryActionFor`,
return `root` for an installed game when `activeRootFinding` returns a finding.
In `renderHero`, render `ROOT ACCESS DETECTED`, show the finding message, set
`data-action="blocked"`, and disable the launch button. Do not invoke
`compatibility-setup`, because that route manages ARM translation rather than
root removal.

- [ ] **Step 5: Add the synchronous Hub launch guard**

Before `open_game_background`, call:

```rust
CompatibilityReport::probe()
    .ensure_known_game_launch_ready(&profile.profile.package_name)?;
```

Place it after graphics readiness and before input-device selection/background
worker creation, so the HTTP action returns a bounded 422 error without
changing Waydroid state.

- [ ] **Step 6: Run focused tests and verify GREEN**

```bash
node crates/wroid-cli/assets/hub/compatibility-state.test.js
node --check crates/wroid-cli/assets/hub/compatibility-state.js
node --check crates/wroid-cli/assets/hub/app.js
cargo test -p wroid-cli commands::hub::tests -- --nocapture
cargo test -p wroid-cli
```

Expected: every command exits 0.

- [ ] **Step 7: Commit**

```bash
git add crates/wroid-cli/assets/hub/compatibility-state.js \
  crates/wroid-cli/assets/hub/compatibility-state.test.js \
  crates/wroid-cli/assets/hub/index.html \
  crates/wroid-cli/assets/hub/app.js \
  crates/wroid-cli/src/commands/hub.rs
git commit -m "Hub: surface active Android root blocker"
```

### Task 3: Documentation, regression gates, and reference-host preparation

**Files:**
- Modify: `README.md`
- Modify: `docs/game-compatibility.md`
- Modify: `docs/roadmap.md`
- Modify: `~/.config/wroid/preferences.json` during local verification only; do not commit.

**Interfaces:**
- Consumes: compatibility `rootAccess` JSON and shared launch guard from Tasks 1-2.
- Produces: documented clean-environment requirement and reproducible Standoff 2 acceptance procedure.

- [ ] **Step 1: Document the supported behavior**

State that known games can reject active Android root, Wroid detects proven
active Magisk signals and blocks launch with remediation, and Wroid never hides
root or bypasses integrity checks. Record the Standoff 2 1280x720 acceptance
workflow and the `< 5 ms` reader-to-inject p95 target.

- [ ] **Step 2: Run formatting and static gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
node --check crates/wroid-cli/assets/hub/app.js
node --check crates/wroid-cli/assets/hub/compatibility-state.js
node crates/wroid-cli/assets/hub/control-chips.test.js
node crates/wroid-cli/assets/hub/compatibility-state.test.js
git diff --check
```

Expected: all commands exit 0.

- [ ] **Step 3: Run complete tests and release build**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --workspace --all-features
CARGO_INCREMENTAL=0 cargo build --release -p wroid-cli -p wroid-daemon -p wroid-inject --bin wroid-helper
```

Expected: all tests pass and release binaries build.

- [ ] **Step 4: Verify live compatibility output**

With Magisk removed and Waydroid running:

```bash
target/release/wroid compatibility --json
target/release/wroid helper status
```

Expected: Standoff 2 is installed, `rootAccess.state` is `not_detected`, health
has no root action finding, and the production helper is release-matched.

- [ ] **Step 5: Save the explicit reference input devices**

Persist these capability-valid paths using the existing preferences API or its
atomic storage adapter:

```text
keyboard=/dev/input/by-id/usb-Homertech_Hexgears_Gaming_Keyboard-event-kbd
mouse=/dev/input/by-id/usb-Logitech_G403_HERO_Gaming_Mouse_0985315F3736-event-mouse
resolution=1280x720
```

Re-read the preferences and verify all three exact values before any managed
session starts.

- [ ] **Step 6: Run the live input self-test when no desktop match is active**

Open Hub, select Standoff 2, and run the existing 20-second production input
self-test. Exercise WASD, relative mouse movement, LMB, RMB, reload, jump, and
crouch. Verify the helper restores the previous desktop Waydroid state and the
trace contains no stuck contact after timeout.

- [ ] **Step 7: Record the remaining manual acceptance gate**

The user performs live-window HUD calibration and one 15-minute match because
these steps require visual choices and gameplay input. Record reader-to-inject
p95, kernel-to-inject p95, frame count, peak contacts, F12 behavior, Ctrl+Esc
cleanup, and any reproducible failure before claiming the complete playable
slice accepted.

- [ ] **Step 8: Commit documentation**

```bash
git add README.md docs/game-compatibility.md docs/roadmap.md
git commit -m "Docs: define clean Standoff 2 acceptance path"
```

- [ ] **Step 9: Inspect final state**

```bash
git status --short
git log -5 --oneline
```

Expected: only the user's existing untracked `.codex/` directory remains;
manual calibration/match evidence is reported separately if still pending.
