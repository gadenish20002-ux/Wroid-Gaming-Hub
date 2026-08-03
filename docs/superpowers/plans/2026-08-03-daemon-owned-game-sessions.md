# Daemon-Owned Game Sessions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `wroidd` own, stop, and reap Hub-launched `launch-v2` game workers.

**Architecture:** Extend the authenticated typed IPC with an atomic game-launch request. Keep process supervision inside `wroid-daemon`, while `wroid-cli` remains the launch worker and Hub client.

**Tech Stack:** Rust, serde JSON over Unix sockets, Linux `SO_PEERCRED`, `std::process::Child`, libc signals, existing Hub HTTP API.

## Global Constraints

- IPC must never accept arbitrary executables or free-form argument vectors.
- Preserve the 0600 socket, same-UID peer authentication, 1 MiB request cap, private 0600 game log, and typed profile validation.
- Preserve direct `launch-v2` for diagnostics and the existing pidfd stop path as a legacy fallback.
- Implement behavior test-first and run Rust commands with `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216`.

---

### Task 1: Typed daemon launch model

**Files:**
- Modify: `crates/wroid-daemon/src/lib.rs`
- Modify: `crates/wroid-daemon/src/ipc.rs`
- Create: `crates/wroid-daemon/src/process.rs`

**Interfaces:**
- Produces: `GameLaunchRequest`, process-aware `SessionSnapshot`, and daemon-owned launch/stop/reap operations.

- [ ] **Step 1: Write failing state and protocol tests**

Add tests proving launch JSON contains only typed profile/path/resolution/device fields, a runtime session records a PID, and success/failure reap transitions are `Stopped`/`Failed` with bounded detail.

- [ ] **Step 2: Run the focused tests and confirm red**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon` and confirm missing launch/process APIs cause failure.

- [ ] **Step 3: Implement the typed request and state transitions**

Add `DaemonRequest::LaunchProfileV2 { launch: GameLaunchRequest }`, increment `PROTOCOL_VERSION`, extend snapshots with `process_id` and `detail`, and add explicit `mark_running`, `mark_stopping`, `mark_stopped`, and `mark_failed` manager operations with transition checks.

- [ ] **Step 4: Implement the process supervisor**

Resolve and validate `/proc/<peer-pid>/exe`; validate the profile and `/dev/input` paths; build the fixed `launch-v2 --width --height [--keyboard] [--mouse]` arguments; open the private game log; spawn a process group; reject a second active worker; signal owned children on stop; and reap children in the daemon service loop.

- [ ] **Step 5: Run daemon tests green**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-daemon` and require all tests to pass.

### Task 2: Route Hub launch and stop through `wroidd`

**Files:**
- Modify: `crates/wroid-cli/src/commands/runtime_daemon.rs`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Consumes: `DaemonRequest::LaunchProfileV2`, `DaemonRequest::List`, and `DaemonRequest::Stop`.
- Produces: `runtime_daemon::launch_game(...)` and `runtime_daemon::stop_game()` used by Hub actions.

- [ ] **Step 1: Replace the direct-spawn test with a failing typed-request test**

Assert the Hub launch adapter supplies session id, profile, canonical path, resolution, and discovered device paths without constructing an argument vector.

- [ ] **Step 2: Run the focused CLI tests and confirm red**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli hub::tests` and confirm the missing daemon adapter fails.

- [ ] **Step 3: Implement daemon launch and stop adapters**

Generate a valid collision-resistant Hub session id, send the typed launch request, render the returned PID, find the single active daemon session for stop, and preserve the existing pidfd stop fallback when no daemon-owned active session exists.

- [ ] **Step 4: Remove Hub child ownership**

Delete `spawn_background_game_reaper` and the direct background `Command::spawn` path. Keep the private-log helpers only where still used by tests or other workflows; remove dead code after `cargo check` identifies it.

- [ ] **Step 5: Run CLI tests green**

Run `CARGO_INCREMENTAL=0 RUST_MIN_STACK=16777216 cargo test -p wroid-cli` and require all tests to pass.

### Task 3: Documentation, full verification, and installed release

**Files:**
- Modify: `README.md`
- Modify: `SPEC.md`
- Modify: `docs/product-readiness-roadmap.md`

**Interfaces:**
- Consumes: the completed daemon-owned launch flow.
- Produces: accurate operator documentation and verified release artifacts.

- [ ] **Step 1: Update product documentation**

Document that Hub game processes survive Hub closure under `wroidd`, that Stop is daemon-backed with a legacy fallback, and mark the daemon process-ownership roadmap item complete.

- [ ] **Step 2: Run the full quality gate**

Run workspace tests, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all -- --check`, Node syntax checks for extracted Hub JavaScript, and `git diff --check`.

- [ ] **Step 3: Build and install the release**

Build `wroid`, `wroidd`, and `wroid-helper` with release settings; run `wroid desktop install`; verify installed/staged hashes match build outputs without installing the privileged helper.

- [ ] **Step 4: Perform a stopped-runtime smoke audit**

Verify Hub state generation and daemon ping/list behavior while Waydroid is stopped, then leave no Wroid, daemon, browser, helper, or temporary test processes running.

