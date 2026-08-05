# Rootless Runtime Readiness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the desktop-user production game session reach live input without invoking Waydroid's root-only shell action.

**Architecture:** Android user readiness and render-property confirmation stay in `DesktopWaydroidSession`; it captures readiness from its owned session child's forwarded stdout and uses the user D-Bus property API. The existing setuid bridge helper gains one argument-free, fixed `getevent -pl` verification request and keeps its stdout channel for the session lifetime; all malformed input still forces stop and cleanup.

**Tech Stack:** Rust 2021, `std::process`, line-delimited private pipe protocol, Waydroid CLI/D-Bus, existing `wroid-inject` unit and live tests.

## Global Constraints

- The production desktop worker must not execute `waydroid shell`.
- The privileged helper must accept no caller-provided command, property, package, or Android device name.
- `/usr/bin/waydroid` remains absolute and executes with a cleared environment.
- EOF, malformed protocol, probe failure, and worker death retain forced Waydroid stop and bridge cleanup.
- The input hot path remains subprocess-free.

---

### Task 1: Rootless Android readiness and render confirmation

**Files:**
- Modify: `crates/wroid-inject/src/waydroid_session.rs`

**Interfaces:**
- Consumes: `DesktopUser::get_property(&self, key: &str) -> io::Result<String>` and `DesktopWaydroidSession::wait_until_android_user_ready(&mut self, user_id: u32)`.
- Produces: `DesktopWaydroidSession::wait_until_android_ready(&mut self) -> io::Result<()>` and `DesktopWaydroidSession::confirm_resolution(&self, width: u32, height: u32) -> io::Result<()>`.

- [ ] **Step 1: Write failing tests**

Add tests proving `confirm_resolution_properties` accepts exact persisted width/height and rejects a mismatched readback with the requested and observed sizes in the error.

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject confirm_resolution
```

Expected: compilation fails because `confirm_resolution_properties` does not exist.

- [ ] **Step 3: Implement the minimal rootless methods**

Add `confirm_resolution_properties<C: WaydroidPropertyControl>` using only the two fixed `persist.waydroid.width` and `persist.waydroid.height` keys. Capture and forward the owned `waydroid session start` stdout/stderr, publish parsed user-ready events through an internal channel, and have `confirm_resolution` delegate to the property helper.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the Task 1 test command and expect both new tests to pass.

### Task 2: Fixed helper Android-input verification protocol

**Files:**
- Modify: `crates/wroid-inject/src/privileged_bridge.rs`

**Interfaces:**
- Consumes: the existing helper child stdin/stdout and fixed `WROID_TOUCHSCREEN_NAME`.
- Produces: `PrivilegedBridgeHelper::verify_android_input(&mut self) -> io::Result<()>`; accepted request `VERIFY_ANDROID_INPUT\n`; exact reply `WROID_ANDROID_INPUT_READY 1\n`.

- [ ] **Step 1: Write failing protocol tests**

Add tests that parse `VERIFY_ANDROID_INPUT\n` and `CLEANUP\n` as distinct commands, reject unknown/oversized commands, and construct the privileged probe with literal arguments `shell -- getevent -pl`.

- [ ] **Step 2: Run the tests and verify RED**

Run:

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject helper_protocol
```

Expected: compilation fails because the command enum/parser and probe builder do not exist.

- [ ] **Step 3: Implement the bounded protocol and client method**

Replace the read-to-EOF cleanup parser with a maximum-64-byte line parser and command loop. The root helper performs a bounded fixed `getevent -pl` probe, emits the exact ready reply, and accepts cleanup; any other outcome follows the existing forced-stop cleanup path. Retain `BufReader<ChildStdout>` in `PrivilegedBridgeHelper` and implement the request/reply method.

- [ ] **Step 4: Run focused tests and verify GREEN**

Run the Task 2 command and all `wroid-inject` tests.

### Task 3: Route the production session through rootless readiness

**Files:**
- Modify: `crates/wroid-inject/src/game_session.rs`
- Modify: `docs/waydroid-input-bridge.md`
- Modify: `docs/roadmap.md`

**Interfaces:**
- Consumes: `DesktopWaydroidSession::{wait_until_android_ready,confirm_resolution}` and `PrivilegedBridgeHelper::verify_android_input`.
- Produces: a production startup path with no direct root-only Waydroid diagnostics.

- [ ] **Step 1: Add a failing bridge-dispatch test**

Extract the session bridge verification boundary so a helper-backed bridge must receive the fixed verification request while an in-process root diagnostic retains the legacy direct check. Test the helper-backed branch through a real pipe-backed protocol fixture.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-inject production_bridge
```

Expected: failure because `SessionBridge` has no verification operation.

- [ ] **Step 3: Wire the production sequence**

Make `SessionBridge` mutable and add its verification operation. Replace production calls to `wait_for_android_boot_completed`, `wait_for_android_display_size`, and `wait_for_android_input_device` with the rootless session methods and bridge verification. Preserve root diagnostic functions for their dedicated binaries. Update the bridge documentation and roadmap to distinguish production from root diagnostics.

- [ ] **Step 4: Run all automated gates**

Run:

```bash
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test --workspace --all-features
CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
git diff --check
```

- [ ] **Step 5: Build, install, and perform live acceptance**

Build the workspace release, run `wroid desktop install`, reinstall the changed root helper with `wroid helper install`, then run:

```bash
wroid launch-v2 ~/.config/wroid/profiles-v2/pubg-mobile.json \
  --no-launch --no-ui --no-grab --trace-input --exit-after-seconds 5
```

Expected: `Unified game session is live`, automatic diagnostic timeout, clean stop, Waydroid `STOPPED`, no Wroid process, and no managed bridge include.
