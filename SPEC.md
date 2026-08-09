# Wroid Gaming Hub SPEC

Wroid Gaming Hub is a Linux gaming frontend for Android games running through Waydroid.

## Goal

Build a BlueStacks-like gaming layer for Linux focused on Android games, with profile-driven controls and Waydroid-friendly workflows.

## Current Scope

The project currently supports:

- JSON control profiles.
- Tap and swipe bindings.
- Terminal keymapper execution.
- Virtual joystick profile bindings and terminal execution.
- ADB and Waydroid shell input backends.
- App list, launch, install APK, and current activity commands.
- Device screen and density detection.
- Local profile registry management.
- Profile coordinate scaling.
- Doctor diagnostics and backend recommendation.
- Profile v2 low-latency sessions through evdev and persistent uinput.
- Desktop-user gameplay runtime with a typed, short-lived privileged helper
  limited to the validated Waydroid LXC input bridge.
- One-time standalone helper installation plus Hub readiness checks for root
  ownership, non-writable permissions, and exact staged-release contents.
- Terminal-free Hub helper setup through detached Polkit authorization and a
  write-sealed memfd source, with an interprocess installation lease.
- Private per-user `wroidd` protocol v2 over a mode-`0600` Unix socket, with
  peer-UID verification, bounded messages, a singleton lease, and typed session
  prepare/launch/start/state/stop/list operations. Normal Hub launches are
  daemon-owned child processes with fixed argument construction and exit reaping.
- Concurrent virtual joysticks, keyboard/mouse taps, sustained hold controls
  for automatic fire, and toggle mouse aim.
- Profile v2 Hold/Toggle layers and modifier chords, with deterministic
  highest-declared-layer and modifier-sibling precedence, continuous-action
  reconciliation, and zero-latency Tap edges.
- Controls Studio layer authoring, filtering, activation/modifier capture, and
  matching input preview, plus per-profile layer readiness in the Hub.
- SYN_REPORT-aware mouse batching plus reader-to-inject and evdev
  kernel-to-inject p50/p95/p99/max session telemetry.
- Sub-pixel-accurate mouse aim: scaled motion carries its remainder across
  events, so sensitivity below 1.0 and ADS multipliers keep slow tracking
  proportional instead of discarding it.
- Descriptor-driven input readers that park in `poll` on the evdev device
  instead of waking on a fixed timer.
- Latency-tuned release profile (fat LTO, single codegen unit, abort on panic)
  and a rootless `wroid-inject-latency` benchmark that reports injection
  p50/p95/p99/max and verifies ten simultaneous contacts.
- Bounded structured last-session performance records surfaced in the Hub with
  input/kernel p95, touch-frame count, peak contacts, and 5 ms budget warning.
- Automatic Waydroid focus protection on KDE Plasma 6 plus an F12 manual
  release/reacquire escape hatch for captured keyboard and mouse devices.
- Starter profiles for Brawl Stars, Standoff 2, PUBG Mobile, and Free Fire.
- Explicit target-game families for PUBG global/Korea/Vietnam/Taiwan/BGMI and
  Free Fire/Free Fire MAX, with atomic no-overwrite exact-package profile
  adoption and independent per-edition calibration.
- Local visual profile v2 controls editor with persistent screenshot/window-
  capture backgrounds, drag, and resize.
- Live Waydroid-window calibration beneath the editable control map, with
  aspect-correct crop, zoom/pan alignment, and aligned-frame persistence.
- Per-game calibration readiness in the Hub plus a single rootless
  open-game-and-Studio workflow for installed packages.
- Local desktop gaming hub with a per-user library, game status, resolution
  presets, hardware diagnostics, profile import, Play Store access, and safe
  daemon-owned background game launch with active-session status, daemon-backed
  Stop, and a PID-race-safe fallback for direct launches.
- Rootless per-user Linux desktop installation with an application-menu entry
  and scalable Wroid icon.
- Android ABI, Play Store, native-bridge, and per-game compatibility preflight
  in CLI and Hub, including safe early refusal for missing known packages.
- Explicit ARM setup handoff through an installed Waydroid Helper or a visible
  terminal package-install flow.
- Non-injecting keyboard/mouse binding preview inside Controls Studio.
- Strict validation of runtime-executable input/action combinations.
- Save-and-launch live profile testing from Controls Studio through the same
  production Waydroid session used by the Hub.
- Transactional `launch-v2` desktop-session restoration with a detached
  crash-recovery watchdog.
- Shared, atomically persisted keyboard, mouse, and Android render-size
  preferences across randomly addressed Hub and Controls Studio sessions.
- Persisted GameMode Auto/Off preference for normal Hub game sessions, with
  fixed-path root-owned wrapper validation, sanitized loader environment, and
  dependency-free direct fallback.
- Rootless desktop Waydroid startup from the Hub with package-manager readiness,
  installed-game refresh, and active-game lease protection.
- Automatic previous-save recovery for edited control profiles, exposed as a
  reversible unsaved revision in Controls Studio.
- Bounded production-path input self-test from the Hub, with no APK launch,
  live tracing, latency reporting, and normal lifecycle cleanup.
- Direct physical-key capture in Controls Studio for key, directional cluster,
  and mouse-aim toggle bindings, with runtime-compatible canonical names.
- Focus-aware, deduplicated Hub refresh after Store, Studio, game-session, and
  Waydroid handoffs without periodic gameplay polling.
- Private, bounded last-session outcomes with clean/stopped/failed Hero state
  and daemon-owned process reaping independent of Hub lifetime.
- Local Waydroid data-volume capacity preflight with separate full-library and
  critical free-space thresholds.
- Extraction-free Android artifact inspection for APK/XAPK/APKM/APKS/OBB,
  including manifest structure, embedded packages, encryption, native ABIs,
  and Waydroid ABI/ARM-translation compatibility before single-APK install.
- Terminal-free single-APK intake in Hub with authenticated streaming upload,
  private ticket state, explicit preflight confirmation, detached Waydroid
  installation, progress/status reporting, safe discard, and stale cleanup.

## Core Principles

- Rust workspace.
- Waydroid remains an external dependency.
- ADB and Waydroid shell are explicit backends.
- Profiles are stored as JSON.
- CLI orchestration stays in `wroid-cli`.
- Profile model and validation stay in `wroid-core`.
- Command wrappers stay in `wroid-adb` and `wroid-waydroid`.

## Out of Scope

- Native toolkit packaging and a settings UI beyond the current local hub.
- Live in-game overlay editing.
- Gamepad mapping.
- Macro execution.
- XAPK/APKM/OBB install flows.
- Anti-cheat bypasses or protection evasion.

## Acceptance Gates

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `node --check` for repository JavaScript and focused JavaScript model tests.
- Example profiles validate.
- `git diff --check` is clean.
- Existing CLI behavior remains compatible.
- New behavior has focused unit coverage.
