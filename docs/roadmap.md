# Roadmap

The delivery order is performance-first: persistent input and runtime ownership
precede the desktop UI. See [Architecture v2](architecture-v2.md) and the
[performance budget](performance-budget.md).

## Phase 0: Runtime foundation (in progress)

- [x] Define backend-independent touch contacts and synchronized frames.
- [x] Guarantee atomic runtime state commit after successful injection.
- [x] Add CI quality gates for formatting, Clippy, and workspace tests.
- [x] Accept architecture decisions for persistent input and privilege separation.
- [x] Split package, display, lifecycle, diagnostics, and input interfaces.
- [x] Add a benchmark harness for the shell compatibility backend.

## Phase 1: Low-latency Linux input

- [x] Implement a Type-B multitouch `uinput` injector.
- [x] Make the virtual touchscreen visible inside Waydroid and verify events with Android `getevent`.
- [ ] Productionize bridge lifecycle, reconciliation, and stable device discovery.
- [x] Add evdev keyboard capture, capability validation, and WASD normalization.
- [x] Exercise live physical keyboard input through a temporary managed Waydroid session.
- [x] Add periodic hold reaffirmation for Android joystick compatibility.
- [x] Add evdev relative-mouse capture, button normalization, and a host diagnostic CLI.
- [ ] Implement focus-loss and crash-safe contact cancellation across the complete session lifecycle.
- [ ] Validate at least ten simultaneous contacts on a real Waydroid session.
- [ ] Measure capture-to-inject p50/p95/p99 latency.

## Phase 2: Runtime daemon and security boundary

- [x] Add the first `wroid-daemon` crate with daemon-owned session bookkeeping.
- [x] Add in-memory daemon preparation for profile v2 control plans.
- [x] Expose daemon profile v2 preparation through the CLI (`wroid session prepare-v2`).
- [ ] Add the per-user `wroidd` daemon process and versioned typed IPC.
- [ ] Add the minimal privileged helper with leased device access.
- [ ] Move CLI execution onto the daemon API.
- [ ] Add production session lifecycle, focus ownership, and configuration rollback.

## Phase 3: Profile v2 and gaming controls

- [x] Add normalized coordinates and aspect-aware viewport transforms.
- [x] Add profile v2 joystick dead-zone metadata and validation.
- [x] Add runtime joystick dead-zone application for analog input.
- [x] Add profile-to-runtime joystick geometry materialization.
- [x] Add profile v2 runtime control plan materialization for taps and joysticks.
- [ ] Add schema migrations, layers, modifiers, and production daemon profile wiring.
- [x] Add a persistent virtual joystick runtime state machine.
- [x] Wire physical WASD input to the persistent joystick in a host smoke path.
- [x] Wire physical WASD through the temporary Waydroid bridge into Android.
- [x] Keep held WASD directions alive with periodic reaffirmation and hold diagnostics.
- [ ] Connect profile-defined controls to the daemon-managed Waydroid session.
- [ ] Implement relative mouse aim.
- [ ] Add physical and virtual gamepad support.

## Phase 4: Desktop gaming hub

- [ ] Add library, game details, performance settings, and diagnostics.
- [ ] Add a visual controls editor with live input testing.
- [ ] Add first-run hardware and Waydroid validation.

## Phase 5: Compatibility and release

- [ ] Inspect APK ABI and package format before installation.
- [ ] Support split APK bundles and game data packages.
- [ ] Add optional image components and ARM translation integration points.
- [ ] Test Intel, AMD, and NVIDIA across supported compositors.
- [ ] Package signed beta releases for major Linux distribution families.

## Legacy MVP checklist

## Completed CLI Foundation

- Rust workspace with `wroid-core`, `wroid-adb`, `wroid-waydroid`, and `wroid-cli`.
- Profile schema, JSON loading/saving, testing, and example profiles.
- ADB and Waydroid command wrappers.
- `wroid binding run` for single tap/swipe bindings.
- `wroid play` terminal keymapper.
- `wroid run` and `wroid run-profile` app launch plus keymapper workflows.
- Device screen and density detection.
- Profile creation, editing, scaling, import/export, duplicate, rename, and remove.
- Android app list, launch, install APK, and current activity commands.
- Virtual joystick profile model, scaling, and terminal execution.
- Doctor diagnostics with backend recommendation and Waydroid UNKNOWN-IP warning.

## Current Limitations

- No GUI.
- No overlay editor.
- No production daemon-managed global input capture.
- The production bridge lifecycle and privilege boundary are not implemented yet.
- Relative mouse capture exists as a host diagnostic path, but Android mouse aim is not wired yet.
- Profile v2 controls can be prepared in memory through `wroid session prepare-v2`, but are not wired into production daemon sessions or input injection yet.
- No gamepad mapping.
- No macro execution.
- No XAPK/APKM/OBB install flow.
- No anti-cheat bypasses or protection evasion.

## Next Useful Milestones

1. Productionize the persistent touchscreen bridge.
   - Keep the successful Android `getevent` integration path.
   - Add stable device discovery and bridge reconciliation.
   - Move privileged bridge operations behind a minimal helper.
   - Verify ten simultaneous contacts and deterministic cleanup.

2. Integrate safe host input capture with the managed session.
   - Reuse the completed evdev keyboard reader, relative mouse reader, WASD normalizer, live Android smoke path, hold reaffirmation loop, and runtime joystick dead zones.
   - Convert normalized mouse motion into profile v2 `mouse_aim` touch frames.
   - Use profile-to-runtime joystick materialization when building session controls.
   - Preserve explicit user permissions and focus ownership.
   - Keep behavior transparent and avoid protection evasion.
   - Move temporary root-owned orchestration behind the daemon/helper boundary.

3. Build a profile authoring workflow.
   - Inspect current app/package.
   - Capture screen size and content viewport.
   - Create profile from the current display.
   - Add bindings from CLI or a future editor.

4. GUI and overlay editor.
   - Not part of the current CLI foundation.
   - Should build on the existing profile schema and validation instead of replacing it.

5. Package install expansion.
   - APK install is implemented.
   - XAPK/APKM/OBB are intentionally out of scope until explicitly designed.
