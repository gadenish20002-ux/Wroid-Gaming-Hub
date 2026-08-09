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
- [x] Make bridge install transactional and preserve unrelated LXC config
  changes during rollback and cleanup.
- [x] Serialize every privileged bridge workflow with a crash-safe kernel lease;
  expose the active owner to CLI, Hub, and Controls Studio before teardown.
- [x] Serialize desktop launch preparation before Waydroid teardown so two
  simultaneous Hub/editor/CLI launches cannot race session restoration.
- [x] Preserve gameplay, Waydroid shutdown, and bridge cleanup errors together
  when multiple lifecycle stages fail.
- [x] Add evdev keyboard capture, capability validation, and WASD normalization.
- [x] Exercise live physical keyboard input through a temporary managed Waydroid session.
- [x] Add periodic hold reaffirmation for Android joystick compatibility.
- [x] Add evdev relative-mouse capture, button normalization, and a host diagnostic CLI.
- [x] Cancel contacts and release grabs/bridge state on Ctrl+Esc, Ctrl+C, SIGHUP,
  SIGTERM, runtime failure, and normal session exit.
- [x] Track Waydroid focus on KDE Plasma 6, release evdev grabs, and cancel
  active contacts on focus loss; show an explicit fallback on other desktops.
- [x] Reserve F12 as a manual release/reacquire hotkey so desktop switching
  remains available while evdev devices are captured.
- [ ] Validate at least ten simultaneous contacts on a real Waydroid session.
- [x] Report reader-to-inject p50/p95/p99/max latency for production sessions.
- [x] Expose a bounded no-APK production input self-test in Hub with tracing,
  latency reporting, automatic timeout, and normal lifecycle restoration.
- [x] Carry evdev kernel timestamps through SYN_REPORT-aware batches and report
  kernel-to-inject p50/p95/p99/max only for submitted touch frames.
- [x] Persist bounded structured production-session metrics and show input and
  kernel p95, touch frames, and peak contacts in the selected game's Hub Hero.
- [x] Remove control-plan/string cloning from mouse motion, button, keyboard,
  and periodic reaffirm hot paths.
- [x] Keep steady-state mouse-aim frames inline and commit validated touch state
  in place, eliminating frame/map allocation from successful MOVE dispatch.
- [ ] Measure kernel event timestamp-to-inject p50/p95/p99 latency on hardware.

## Phase 2: Runtime daemon and security boundary

- [x] Add the first `wroid-daemon` crate with daemon-owned session bookkeeping.
- [x] Add in-memory daemon preparation for profile v2 control plans.
- [x] Expose daemon profile v2 preparation through protocol v1 from the CLI
  (`wroid session prepare-v2`).
- [x] Add the per-user `wroidd` daemon process and versioned typed IPC, with
  private socket permissions, peer credentials, bounded messages, and a
  crash-safe singleton lease.
- [x] Route normal Hub launch and Stop through typed protocol v2, with `wroidd`
  owning, signalling, and reaping the fixed-argument `launch-v2` child.
- [ ] Add the minimal privileged helper with leased device access.
  - [x] Move the gameplay hot path to the desktop user and isolate the temporary
    LXC bridge in a typed helper with fixed cleanup/recovery commands.
  - [x] Ship a standalone root-owned helper install/status path and block Hub
    gameplay when ownership, permissions, or staged-release equality fail.
  - [x] Eliminate per-launch sudo with an exact root:input `4750` helper,
    effective-root check, absolute Waydroid path, and sanitized environment.
  - [x] Replace the Hub's setup terminal with leased Polkit authorization over
    a detached, write-sealed memfd release source.
  - [x] Keep production Android boot/render checks rootless and allow only one
    fixed helper-side `getevent -pl` touchscreen readiness probe.
  - [ ] Replace direct helper activation with versioned daemon/helper IPC.
- [ ] Move the remaining direct CLI execution paths onto the daemon API.
- [ ] Add production session lifecycle, focus ownership, and configuration rollback.

## Phase 3: Profile v2 and gaming controls

- [x] Add normalized coordinates and aspect-aware viewport transforms.
- [x] Add profile v2 joystick dead-zone metadata and validation.
- [x] Add runtime joystick dead-zone application for analog input.
- [x] Add profile-to-runtime joystick geometry materialization.
- [x] Add profile v2 runtime control plan materialization for taps and joysticks.
- [x] Add sustained key/mouse hold actions with Down/Up lifecycle for automatic
  fire and other press-and-hold HUD controls.
- [x] Reject non-executable input/action pairs in profile validation and only
  offer compatible input sources in Controls Studio.
- [x] Add profile v2 runtime control plan materialization for relative mouse aim.
- [x] Add profile v2 layers and modifier chords across validation, runtime
  dispatch, Controls Studio, Hub readiness, and production session tooling.
- [ ] Add schema migrations and finish production daemon profile wiring.
- [x] Add a persistent virtual joystick runtime state machine.
- [x] Wire physical WASD input to the persistent joystick in a host smoke path.
- [x] Wire physical WASD through the temporary Waydroid bridge into Android.
- [x] Keep held WASD directions alive with periodic reaffirmation and hold diagnostics.
- [x] Add `wroid play-v2` for profile-defined joysticks, taps, holds, and toggle mouse aim.
- [x] Ship starter profiles for Brawl Stars, Standoff 2, PUBG Mobile, and Free Fire.
- [ ] Connect profile-defined controls to the daemon-managed Waydroid session.
- [ ] Wire profile-defined relative mouse aim through the daemon-managed session.
- [ ] Add physical and virtual gamepad support.

## Phase 4: Desktop gaming hub

- [x] Add the first unprivileged library, game details, resolution presets, and
  hardware/runtime diagnostics.
- [x] Add the first unprivileged visual profile v2 controls editor.
- [x] Persist per-profile calibration backgrounds and capture a selected
  Waydroid window through the browser display-capture permission flow.
- [x] Add rootless per-user binary, desktop-entry, and icon installation.
- [x] Refresh packages, edited profiles, and session leases when Hub regains
  focus, deduplicating concurrent events without background gameplay polling.
- [x] Add launch-time GPU/DRM/Waydroid graphics preflight with a
  software-renderer launch blocker in both CLI and Hub.
- [x] Detect multi-GPU host/Waydroid mismatch and provide an atomic,
  rollback-safe active-GPU setup action.
- [x] Suppress GPU mismatch/setup recommendations while Waydroid graphics
  properties are offline or unknown.
- [x] Add Android ABI, Play Store, native-bridge, and per-game compatibility
  status for the four starter games.
- [x] Cover essential FPS reload and right-click ADS controls in the shipped
  PUBG Mobile and Standoff 2 starter maps.
- [x] Add safe in-editor keyboard/mouse binding preview.
- [x] Add direct physical-key capture for single, directional-cluster, and
  mouse-aim toggle bindings while preserving reserved runtime hotkeys.
- [x] Restore the previous desktop Waydroid session after success, failure,
  cancelled sudo, or launcher crash.
- [x] Add save-and-launch live Android profile testing from Controls Studio.
- [x] Retain the previous valid control map on changed saves and load it
  reversibly from Controls Studio without overwriting the active profile.
- [x] Apply Hub and Controls Studio resolution presets to the real Android
  render surface with transactional property rollback and live-size checks.
- [x] Persist selected input devices and render target outside browser origins,
  with atomic shared settings for Hub and Controls Studio.
- [x] Start desktop Waydroid rootlessly from Hub Store/UI actions, wait for
  Package Manager readiness, and refresh installed games without racing play.
- [x] Run normal Hub game launches without a terminal, expose verified active
  session state, supervise them through `wroidd`, and retain pidfd identity as
  the safe fallback for direct and pre-upgrade launches.
- [x] Persist bounded private game outcomes and surface late background failures
  in the selected game's Hero without requiring a terminal.
- [x] Report Waydroid's host-driven refresh target and presentation-feedback
  state without relying on unsupported FPS properties.
- [x] Add optional Feral GameMode Auto/Off launch integration with a trusted
  daemon-owned wrapper plan and direct fallback when unavailable.
- [x] Report capacity on Waydroid's actual writable data volume before large
  game packages and resource updates are installed.
- [x] Add live Waydroid-window overlay calibration with crop alignment and
  persistent aligned frames.
- [x] Expose per-game calibration readiness and open an installed game plus
  Controls Studio through one rootless Hub action.
- [x] Add automatic first-run hardware and Waydroid validation.

## Phase 5: Compatibility and release

- [x] Inspect APK/XAPK/APKM/APKS/OBB format and native ABI before installation
  through a bounded, extraction-free central-directory parser.
- [x] Add terminal-free Hub sideload for single APKs with streaming upload,
  pre-install compatibility readout, detached install status, and safe discard.
- [ ] Support split APK bundles and game data packages.
- [ ] Add optional image component management.
- [x] Add ARM translation detection and an explicit Waydroid Helper setup flow.
- [x] Recognize official PUBG regional/BGMI and Free Fire MAX package variants,
  and atomically derive exact-package controls without overwriting user maps.
- [x] Read saved ABI/native-bridge properties while Waydroid is stopped and
  preserve an explicit unknown state when evidence is unavailable.
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

- The launcher is a localhost desktop web UI with application-menu integration;
  native toolkit packaging and background service integration are not implemented yet.
- The visual editor can calibrate over a user-authorized live Waydroid window,
  but it is not an always-on overlay during gameplay.
- No production daemon-managed global input capture.
- The transactional bridge lifecycle and standalone root-owned helper are
  implemented, but policy-controlled activation and the reconciliation daemon are not.
- Profile preparation and normal Hub process ownership use per-user daemon IPC;
  live profile controls still execute inside the daemon-supervised desktop-user
  `play-v2` worker until capture and cleanup move into daemon-native components.
- The LXC bridge helper needs one graphical Polkit authorization during its
  one-time installation. Gameplay later stops Waydroid automatically for
  temporary bridge setup and does not prompt again.
- No gamepad mapping.
- No macro execution.
- No XAPK/APKM/OBB install flow.
- No anti-cheat bypasses or protection evasion.

## Next Useful Milestones

1. Productionize the persistent touchscreen bridge.
   - Keep the successful Android `getevent` integration path.
   - Add stable device discovery and bridge reconciliation.
   - Move direct helper activation behind versioned daemon/helper IPC.
   - Verify ten simultaneous contacts and deterministic cleanup.

2. Integrate safe host input capture with the managed session.
   - Reuse the completed evdev keyboard reader, relative mouse reader, WASD normalizer, live Android smoke path, hold reaffirmation loop, runtime joystick dead zones, and persistent mouse-aim primitive.
   - Execute profile-materialized `mouse_aim` controls from normalized relative mouse motion.
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
   - The local gaming hub, screenshot-backed controls editor, and live
     Waydroid-window calibration are implemented.
   - Native desktop packaging remains.

5. Package install expansion.
   - Single-APK install includes package structure, encryption, and Waydroid ABI
     preflight with an explicit incompatible override.
   - XAPK/APKM/OBB are intentionally out of scope until explicitly designed.
