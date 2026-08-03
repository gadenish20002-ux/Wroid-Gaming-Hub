# Hub APK Sideload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add safe, terminal-free single-APK inspection and installation to Wroid Gaming Hub.

**Architecture:** Split HTTP header parsing from bounded body consumption so the authenticated upload route can stream to private ticket-based storage. Reuse Android package preflight, then delegate installation to a hidden Wroid worker that records an atomic status independently of the browser connection.

**Tech Stack:** Rust, loopback HTTP over `std::net`, serde JSON, clap, existing `wroid-android` and `wroid-waydroid`, plain HTML/CSS/JavaScript.

## Global Constraints

- Normal JSON requests remain limited to 2 MiB; APK uploads are nonzero, exact-length, non-chunked, and limited to 4 GiB.
- Browser-controlled values never become server paths; only validated 192-bit lowercase-hex tickets resolve private files.
- State directory mode is 0700 and files are 0600 with `O_NOFOLLOW` and exclusive creation.
- Installation always repeats preflight and never exposes an incompatible-force override in Hub.
- Staged APKs are deleted after rejection, discard, or worker completion; known stale artifacts expire after 24 hours.

---

### Task 1: Reusable package preflight

**Files:**
- Modify: `crates/wroid-cli/src/commands/app.rs`

**Interfaces:**
- Produces: `pub(crate) struct PackagePreflight`, `pub(crate) fn package_preflight(&Path)`, and `pub(crate) fn validate_install_preflight(&PackagePreflight, bool)`.

- [ ] Add a compile-time/unit test in `hub.rs` that consumes the package preflight fields and fails while they are private.
- [ ] Run the focused test and confirm the privacy failure.
- [ ] Expose only the reusable type, fields, and functions; keep formatting helpers private.
- [ ] Run app and Hub tests.

### Task 2: Streaming authenticated upload and private staging

**Files:**
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Produces: parsed `RequestHead`, bounded normal-body reader, ticket validator, private sideload path resolver, exact upload streamer, inspect/discard/status handlers.

- [ ] Add focused tests for header parsing, duplicate/invalid length, transfer encoding, route authorization, 4 GiB limit, short body, ticket syntax, private permissions, rejection cleanup, and successful inspection JSON.
- [ ] Run each focused test group and confirm the expected missing-behavior failures.
- [ ] Implement the smallest header/body split and private staging helpers that satisfy the tests.
- [ ] Wire upload, status, and discard routes and run all Hub tests.

### Task 3: Detached install worker

**Files:**
- Modify: `crates/wroid-cli/src/cli.rs`
- Modify: `crates/wroid-cli/src/commands/mod.rs`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Produces: hidden `install-apk-worker --ticket <ticket>` command and Hub `POST /api/apk/install` ticket action.

- [ ] Add parser and worker tests for hidden command arguments, valid ticket resolution, installing status, final success/failure, bounded detail, artifact cleanup, and exact spawned arguments.
- [ ] Run focused tests and confirm missing command/worker failures.
- [ ] Implement worker execution, atomic status writes, Waydroid readiness, install dispatch, and Hub spawning.
- [ ] Run CLI, app, and Hub tests.

### Task 4: Inline package intake UI

**Files:**
- Modify: `crates/wroid-cli/assets/hub/index.html`
- Modify: `crates/wroid-cli/assets/hub/styles.css`
- Modify: `crates/wroid-cli/assets/hub/app.js`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

**Interfaces:**
- Consumes: `/api/apk/upload`, `/api/apk/install`, `/api/apk/status`, `/api/apk/discard`.

- [ ] Add asset assertions for the sideload control, intake live region, progress semantics, install/discard actions, XHR progress, and status polling; run to RED.
- [ ] Add the accessible inline markup and DOM bindings.
- [ ] Implement upload, render, install, poll, discard, and reset state with bounded error presentation.
- [ ] Style the industrial scanner strip for desktop/mobile and reduced motion.
- [ ] Run Hub asset tests and JavaScript syntax validation.

### Task 5: Product verification and release install

**Files:**
- Modify if needed: `README.md`, `docs/roadmap.md`

**Interfaces:**
- Produces: verified release binary and desktop installation.

- [ ] Run focused Rust tests, full workspace tests, Clippy with warnings denied, rustfmt check, and JavaScript syntax check.
- [ ] Launch Hub against an isolated profile directory and visually inspect desktop and narrow viewport states.
- [ ] Exercise upload/inspect/discard with a synthetic APK and upload/inspect against a real installed Waydroid APK; do not install or mutate user Android packages during automated verification.
- [ ] Build release and run `wroid desktop install`.
- [ ] Stop test Hub/worker processes, stop Waydroid if verification started it, and audit leases/state.
