# Waydroid ANR and Pointer Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove Trebuchet from managed game startup, disable Android pointer diagnostics through the fixed privileged boundary, and warn about Btrfs CoW latency risk.

**Architecture:** A pure startup policy chooses a package launch instead of the full Android desktop whenever a game package is requested. The existing fixed helper verification operation additionally runs two literal Android settings commands before reporting readiness. Storage probing classifies Btrfs CoW without mutating user data.

**Tech Stack:** Rust 2021, std process/filesystem APIs, Linux `statfs`/`ioctl`, existing Wroid helper protocol, Cargo tests.

## Global Constraints

- The Hub, daemon, worker, and Controls Studio remain unprivileged.
- The helper receives no caller-controlled Android command, setting, namespace, or value.
- Storage diagnostics never modify Android data.
- Existing UI-only, no-launch, no-UI, and helper cleanup behavior remains supported.
- Production changes follow strict red-green TDD.

---

### Task 1: Launch games without opening Trebuchet

**Files:**
- Modify: `crates/wroid-inject/src/game_session.rs`

**Interfaces:**
- Produces: private `AndroidOpenAction` and `android_open_action(show_ui: bool, launch_package: bool) -> AndroidOpenAction`.
- Consumes: existing `DesktopWaydroidSession::{show_full_ui, launch_package}`.

- [ ] **Step 1: Write the failing startup-policy test**

```rust
#[test]
fn package_launch_bypasses_full_android_desktop() {
    assert_eq!(android_open_action(true, true), AndroidOpenAction::Package);
    assert_eq!(android_open_action(false, true), AndroidOpenAction::Package);
    assert_eq!(android_open_action(true, false), AndroidOpenAction::FullUi);
    assert_eq!(android_open_action(false, false), AndroidOpenAction::None);
}
```

- [ ] **Step 2: Run the focused test and verify RED**

```bash
cargo test -p wroid-inject game_session::tests::package_launch_bypasses_full_android_desktop -- --exact
```

Expected: compilation fails because the selector does not exist.

- [ ] **Step 3: Implement the minimal startup policy**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AndroidOpenAction { None, FullUi, Package }

fn android_open_action(show_ui: bool, launch_package: bool) -> AndroidOpenAction {
    if launch_package {
        AndroidOpenAction::Package
    } else if show_ui {
        AndroidOpenAction::FullUi
    } else {
        AndroidOpenAction::None
    }
}
```

Replace the two independent startup `if` statements with one `match`.
`Package` launches and logs the selected package; `FullUi` opens the desktop;
`None` performs neither operation.

- [ ] **Step 4: Verify GREEN and commit**

```bash
cargo test -p wroid-inject game_session::tests::package_launch_bypasses_full_android_desktop -- --exact
cargo test -p wroid-inject
git add crates/wroid-inject/src/game_session.rs
git commit -m "Inject: bypass Trebuchet for game launches"
```

Expected: both test commands exit 0 before the commit.

### Task 2: Disable Android pointer diagnostics in the fixed helper operation

**Files:**
- Modify: `crates/wroid-inject/src/privileged_bridge.rs`
- Modify: `README.md`

**Interfaces:**
- Produces: private `android_show_touches_off_command() -> Command`, `android_pointer_location_off_command() -> Command`, and `disable_android_pointer_diagnostics_privileged() -> io::Result<()>`.
- Consumes: existing `fixed_privileged_command` and `wait_for_android_input_privileged`.

- [ ] **Step 1: Write failing exact-command tests**

Extend `helper_protocol_android_probe_has_fixed_arguments` to assert both
commands use `/usr/bin/lxc-attach` and these literal tails:

```text
--clear-env -- /system/bin/settings put system show_touches 0
--clear-env -- /system/bin/settings put system pointer_location 0
```

Add the characterization test
`helper_protocol_withholds_ready_when_android_cleanup_fails`: its callback
returns `io::Error::other("pointer cleanup failed")`; assert the protocol
returns that error and writes no readiness reply. This protocol behavior
already exists, so the exact-command assertions provide the RED gate for the
new cleanup behavior.

- [ ] **Step 2: Run focused tests and verify RED**

```bash
cargo test -p wroid-inject privileged_bridge::tests::helper_protocol_android_probe_has_fixed_arguments -- --exact
cargo test -p wroid-inject privileged_bridge::tests::helper_protocol_withholds_ready_when_android_cleanup_fails -- --exact
```

Expected: missing command constructors or missing behavior causes failure.

- [ ] **Step 3: Implement fixed cleanup**

Construct both commands with the existing absolute `lxc-attach` prefix and no
dynamic arguments. Run them after touchscreen discovery and before returning
success from `wait_for_android_input_privileged`. A non-zero exit returns an
`io::Error` naming the fixed setting and containing bounded combined output.
Do not add helper arguments, broker fields, profile options, or an arbitrary
settings function.

- [ ] **Step 4: Document helper behavior**

Update `README.md` to state that managed readiness disables `show_touches` and
`pointer_location`, and that a changed helper must be rebuilt, staged through
`wroid desktop install`, then installed with `wroid helper install`.

- [ ] **Step 5: Verify GREEN and commit**

```bash
cargo test -p wroid-inject privileged_bridge
cargo test -p wroid-inject
git add crates/wroid-inject/src/privileged_bridge.rs README.md
git commit -m "Inject: disable Android pointer diagnostics"
```

Expected: both test commands exit 0 before the commit.

### Task 3: Report Btrfs CoW latency risk without mutation

**Files:**
- Modify: `crates/wroid-cli/src/commands/storage.rs`
- Modify: `docs/game-compatibility.md`

**Interfaces:**
- Produces: private `CopyOnWriteState::{NotBtrfs, Disabled, Enabled, Unknown}`, `copy_on_write_state(path: &Path) -> CopyOnWriteState`, and `classify_storage(available_bytes: u64, cow: CopyOnWriteState) -> (&'static str, String)`.
- Extends: `StorageReport::as_json()` with `copyOnWrite` equal to `not_btrfs`, `disabled`, `enabled`, or `unknown`.

- [ ] **Step 1: Write failing classification tests**

```rust
#[test]
fn capacity_warnings_precede_btrfs_cow_warning() {
    assert_eq!(classify_storage(7 * GIB, CopyOnWriteState::Enabled).0, "critical");
    assert_eq!(classify_storage(20 * GIB, CopyOnWriteState::Enabled).0, "warning");
}

#[test]
fn healthy_capacity_warns_only_for_btrfs_cow() {
    assert_eq!(classify_storage(50 * GIB, CopyOnWriteState::Enabled).0, "warning");
    assert_eq!(classify_storage(50 * GIB, CopyOnWriteState::Disabled).0, "ready");
    assert_eq!(classify_storage(50 * GIB, CopyOnWriteState::NotBtrfs).0, "ready");
    assert_eq!(classify_storage(50 * GIB, CopyOnWriteState::Unknown).0, "ready");
}
```

Add a JSON assertion on the literal `copyOnWrite` value.

- [ ] **Step 2: Run storage tests and verify RED**

```bash
cargo test -p wroid-cli commands::storage::tests -- --nocapture
```

Expected: compilation fails because the enum and classifier are missing.

- [ ] **Step 3: Implement read-only Btrfs probing**

Use `libc::statfs` and Btrfs magic `0x9123683e`. On Btrfs, open the directory
read-only and call `FS_IOC_GETFLAGS` (`0x80086601`) into `libc::c_long`, matching
Linux `_IOR('f', 1, long)`;
`FS_NOCOW_FL` (`0x00800000`) means `Disabled`. Failed probes return `Unknown`.
Never call `SETFLAGS`, `chattr`, copy, rename, or delete.

Capacity below 8 GiB stays `critical`; below 40 GiB stays the current warning;
only otherwise does `Enabled` produce the CoW latency warning.

- [ ] **Step 4: Document, verify GREEN, and commit**

Explain the read-only warning in `docs/game-compatibility.md`, then run:

```bash
cargo test -p wroid-cli commands::storage::tests -- --nocapture
cargo test -p wroid-cli
git add crates/wroid-cli/src/commands/storage.rs docs/game-compatibility.md
git commit -m "CLI: warn about Waydroid Btrfs CoW"
```

Expected: both test commands exit 0 before the commit.

### Task 4: Verify the complete fix

**Files:**
- Verify only.

**Interfaces:**
- Consumes all behavior produced by Tasks 1-3.

- [ ] **Step 1: Format and inspect**

```bash
cargo fmt --all -- --check
git diff --check
git status --short
```

Expected: checks exit 0; only intentional state and the user's existing
untracked `.codex/` directory appear.

- [ ] **Step 2: Run complete tests and release build**

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --workspace
CARGO_INCREMENTAL=0 cargo build --release -p wroid-cli -p wroid-daemon -p wroid-inject --bin wroid-helper
```

Expected: both commands exit 0 with zero failed tests.

- [ ] **Step 3: Perform mutation checks**

Temporarily reverse the package/full-UI selector and verify Task 1's test fails;
restore it. Remove one fixed settings command and verify Task 2's exact-command
test fails; restore it. Classify `CopyOnWriteState::Enabled` as ready and verify
Task 3's test fails; restore it. Re-run all focused tests after restoration.

- [ ] **Step 4: Inspect live prerequisites read-only**

```bash
waydroid status
ls -l /usr/lib/wroid/wroid-helper ~/.local/share/libexec/wroid/wroid-helper
lsattr -d ~/.local/share/waydroid/data
```

Record whether the installed helper needs reinstalling. Do not start Waydroid
or request authorization without first announcing the visible action.

- [ ] **Step 5: Review history and acceptance coverage**

```bash
git log -4 --oneline
git status --short
```

Confirm every design criterion has code or test evidence and report any
live-only verification that remains.
