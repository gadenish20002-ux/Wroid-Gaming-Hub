# Roadmap

The delivery order is performance-first: persistent input and runtime ownership
precede the desktop UI. See [Architecture v2](architecture-v2.md) and the
[performance budget](performance-budget.md).

## Phase 0: Runtime foundation (in progress)

- [x] Define backend-independent touch contacts and synchronized frames.
- [x] Guarantee atomic runtime state commit after successful injection.
- [x] Add CI quality gates for formatting, Clippy, and workspace tests.
- [x] Accept architecture decisions for persistent input and privilege separation.
- [ ] Split package, display, lifecycle, diagnostics, and input interfaces.
- [ ] Add a benchmark harness for the shell compatibility backend.

## Phase 1: Low-latency Linux input

- [ ] Implement a Type-B multitouch `uinput` injector.
- [ ] Make the virtual touchscreen visible inside Waydroid.
- [ ] Add evdev keyboard and relative-mouse capture.
- [ ] Implement focus-loss and crash-safe contact cancellation.
- [ ] Validate at least ten simultaneous contacts.
- [ ] Measure capture-to-inject p50/p95/p99 latency.

## Phase 2: Runtime daemon and security boundary

- [ ] Add the per-user `wroidd` daemon and versioned typed IPC.
- [ ] Add the minimal privileged helper with leased device access.
- [ ] Move CLI execution onto the daemon API.
- [ ] Add session lifecycle and configuration rollback.

## Phase 3: Profile v2 and gaming controls

- [ ] Add normalized coordinates and aspect-aware viewport transforms.
- [ ] Add schema migrations, layers, modifiers, hold/toggle modes, and dead zones.
- [ ] Implement persistent virtual joystick and relative mouse aim.
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
- No global input capture.
- No evdev/uinput backend.
- No gamepad mapping.
- No mouse aim behavior.
- No macro execution.
- No XAPK/APKM/OBB install flow.
- No anti-cheat bypasses or protection evasion.

## Next Useful Milestones

1. Improve terminal joystick ergonomics.
   - Better handling for terminals without release events.
   - Optional on-screen debug output for active directions.
   - Clearer runtime warnings when release events are unavailable.

2. Add non-global input backends only when safe.
   - Investigate evdev/uinput with explicit user permissions.
   - Keep behavior transparent and avoid protection evasion.

3. Build a profile authoring workflow.
   - Inspect current app/package.
   - Capture screen size.
   - Create profile from current display.
   - Add bindings from CLI or a future editor.

4. GUI and overlay editor.
   - Not part of the current CLI foundation.
   - Should build on the existing profile schema and validation instead of replacing it.

5. Package install expansion.
   - APK install is implemented.
   - XAPK/APKM/OBB are intentionally out of scope until explicitly designed.
