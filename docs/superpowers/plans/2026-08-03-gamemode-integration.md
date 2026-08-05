# GameMode integration implementation plan

**Goal:** Add optional, secure Feral GameMode activation for normal Hub game sessions.

**Architecture:** Persist an Auto/Off preference in the Hub, carry a boolean through the existing typed daemon request, and let the daemon construct either a direct launch or a sanitized launch through a trusted fixed-path `gamemoderun`.

**Tech stack:** Rust, serde JSON, std process/filesystem APIs, vanilla HTML/CSS/JavaScript.

---

### Task 1: Persist the performance preference

**Files:**
- Modify: `crates/wroid-cli/src/commands/preferences.rs`

- [x] Add failing tests for legacy-file defaulting, JSON patching, and persistence of `gameMode`.
- [x] Add `game_mode: bool`, defaulting to true, to preferences and patches.
- [x] Run the focused preference tests.

### Task 2: Extend the typed daemon request

**Files:**
- Modify: `crates/wroid-daemon/src/ipc.rs`
- Modify: `crates/wroid-cli/src/commands/runtime_daemon.rs`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

- [x] Add failing serialization and request-construction tests for `gameMode`.
- [x] Add the backward-compatible boolean protocol field.
- [x] Carry the preference from the Hub action into `launch_game`.
- [x] Run the focused daemon and CLI tests.

### Task 3: Build the secure daemon launch plan

**Files:**
- Modify: `crates/wroid-daemon/src/process.rs`

- [x] Add failing tests for enabled, disabled, unavailable, untrusted, and environment-sanitized plans.
- [x] Validate only fixed, canonical, root-owned, protected executable wrappers.
- [x] Build a direct or wrapped command plan without accepting paths from the client.
- [x] Remove `GAMEMODERUNEXEC` and `LD_PRELOAD` from the spawned command environment.
- [x] Run focused process tests, including real short-lived child lifecycle tests.

### Task 4: Add the compact Hub control

**Files:**
- Modify: `crates/wroid-cli/assets/hub/index.html`
- Modify: `crates/wroid-cli/assets/hub/styles.css`
- Modify: `crates/wroid-cli/assets/hub/app.js`
- Modify: `crates/wroid-cli/src/commands/hub.rs`

- [x] Add an asset regression test for the toggle and launch payload.
- [x] Add an accessible Auto/Off performance toggle beside the resolution preset.
- [x] Render persisted state, save changes, and include it only in normal launch payloads.
- [x] Run Hub tests and JavaScript syntax validation.

### Task 5: Document and verify

**Files:**
- Modify: `README.md`
- Modify: `SPEC.md`
- Modify: `docs/performance-budget.md`
- Modify: `docs/roadmap.md`

- [x] Document optional GameMode behavior and safety boundary.
- [x] Run `cargo fmt --all -- --check`.
- [x] Run all workspace tests with the repository stack settings.
- [x] Run Clippy for all targets/features with warnings denied.
- [x] Run JavaScript syntax checks and `git diff --check`.
- [x] Build release binaries, reinstall user-owned artifacts, and compare hashes.
- [x] Smoke-test protocol/runtime state without starting Waydroid.
