# Implementation Plan

This document is the authoritative, audited snapshot of the repository and the
phase-by-phase plan to reach the production architecture described in
[architecture-v2.md](architecture-v2.md). It is updated as phases land.

Last audited against the workspace on 2026-06-24. Baseline (`cargo fmt --all --
check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`) is green at the time of writing.

## Product boundary (unchanged, non-negotiable)

Wroid is a gaming runtime/frontend on top of Waydroid. Waydroid owns the Android
container and direct rendering. Wroid owns gaming runtime, controls, profiles,
diagnostics, package management, and UX. This is not an emulator and must not add
a video capture/encode path for normal local play.

## Current crate map

The workspace (`Cargo.toml`, resolver 2) contains seven crates. Dependency
direction is leaf → CLI; `wroid-core` and `wroid-runtime` are pure (no
subprocess, no backend, no toolkit).

| Crate | Responsibility | Subprocess? | Depends on |
| --- | --- | --- | --- |
| `wroid-core` | Profile data model. Legacy `ControlProfile` (pixel coords, `tap`/`swipe`/`virtual_joystick`/`mouse_aim`/`macro`), JSON load/save, validation, coordinate scaling (`scale_point`/`scale_action`/`scale_profile`). `profile_v2` prototype: normalized coords, `Orientation`, `mouse_move`, `mouse_aim` regions, `macro`, `JoystickMode` hold/toggle, `reaffirm_ms`, materialization to pixels. | No | serde, serde_json, thiserror |
| `wroid-runtime` | Pure runtime state machine. `ContactId`, `TouchPhase`, `TouchEvent`, `TouchFrame`, `TouchState`, `TouchInjector` trait, `TouchEngine` (validate-then-commit, atomic state only after backend accepts), `VirtualJoystick`, `DirectionalInput`. No I/O. | No | wroid-core, thiserror |
| `wroid-input` | Host evdev capture + normalization. `lib.rs`: `EvdevKeyboard`, `HostKey`/`HostKeyEvent`, WASD `DirectionalKeyState`. `mouse.rs` (now a `pub mod`): `EvdevMouse`, relative motion/wheel/button normalization. Bin `wroid-mouse-capture` is a host diagnostic. | No (evdev fd only) | evdev, thiserror, wroid-runtime |
| `wroid-inject` | Persistent Linux input injection + Waydroid integration harness. `UinputTouchInjector` (Type-B multitouch over a persistent uinput fd via `EvdevEventSink`; no subprocess per frame), slot state, translate, config, error. `waydroid_bridge`/`waydroid_session` spawn Waydroid for *session lifecycle/diagnostics only*. `live_keyboard` end-to-end host→Android smoke path. Many diagnostic/bench bins. | Setup/diagnostics only (`runuser`, `waydroid`, `timeout`) — never per-frame | evdev, thiserror, wroid-core, wroid-input, wroid-runtime |
| `wroid-adb` | Thin ADB adapter (`Adb` struct + free fns): devices, tap/swipe/keyevent, packages, launch (monkey), install, current activity, `wm size`/`density`. Activity/package parsing. | Yes — `adb` (compatibility/debug) | anyhow |
| `wroid-waydroid` | Thin Waydroid adapter (`Waydroid` struct + free fns): status, session start, show-full-ui, shell input tap/swipe/keyevent, app list/launch/install, launch-as-user (sudo env), current activity, `wm size`/`density`. Maps root-needed shell errors to actionable messages. | Yes — `waydroid`, `sudo` (compatibility/debug) | anyhow |
| `wroid-cli` | `wroid` binary. clap parsing (`cli.rs`), command dispatch (`commands/`), backend selection + `InputExecutor` trait (`backend.rs`), registry, scaling, device probes, terminal interactive keymapper (`interactive.rs`), doctor. Bin `wroid-live-keyboard` delegates to `wroid-inject`. | Via adapters only | anyhow, clap, crossterm, wroid-adb, wroid-core, wroid-inject, wroid-waydroid |

### Key existing abstraction

`wroid-cli/src/backend.rs` already defines the `InputExecutor` trait
(package/app/display/activity/shell-input operations) with a real
`CommandInputExecutor` and a test `FakeInputExecutor`. `select_input_backend`
implements deterministic `auto`/`adb`/`waydroid-shell` selection: `auto` picks
ADB only when `adb devices` lists a device in state `device`, else
`waydroid-shell`. This trait is the seam Phase 1 builds typed interfaces around —
it is currently one wide CLI-private trait, not split per concern.

## Current implemented capabilities

- Profiles: validate, example, new, new-from-current-screen, scale, scale-to-
  current, add tap/swipe/joystick, remove binding, list bindings, import/export,
  duplicate, rename, remove, registry list/path/show. Legacy schema only.
- Profile v2: prototype schema + `wroid-profile-v2-validate` bin. **Not** the
  default runtime format.
- Input (compatibility): `input tap/swipe/keyevent`, `binding run`.
- App: list, launch (incl. sudo launch-as-desktop-user), extraction-free
  package format/ABI inspection, install-apk preflight, current activity.
- Device: screen, density, info; `doctor` with backend recommendation and the
  Waydroid `IP UNKNOWN` warning.
- Gameplay (compatibility): `play`/`run`/`run-profile` load + validate a profile,
  optionally launch, then run a terminal keymapper that emits tap/swipe and a
  tick-based virtual joystick through the selected shell backend.
- Production runtime primitives (not yet wired to CLI gameplay): pure
  `TouchEngine`/`VirtualJoystick`; persistent `UinputTouchInjector` (Type-B);
  evdev keyboard + relative-mouse capture; `live_keyboard` host→Android smoke
  path with hold reaffirmation; Waydroid bridge install/session harness.

## Gameplay paths that still use ADB / shell / terminal / subprocess

All are **legacy/compatibility** and permitted to remain as compat/debug; none
are the intended production gaming hot path.

1. **Interactive keymapper hot loop** (`interactive.rs`): per tap, per swipe, and
   per joystick tick calls `execute_tap`/`execute_swipe` → `InputExecutor` →
   `wroid_adb`/`wroid_waydroid` which `Command::new` a shell `input` per event.
   This is process-per-event and violates the production input budget; it is the
   compatibility path, not to be promoted.
2. **Terminal focus dependency** (`interactive.rs` + `RawModeGuard`): input comes
   from crossterm/the focused terminal, not global capture.
3. **App launch under sudo** (`run.rs`): spawns `waydroid app launch` as the
   desktop user (lifecycle, not hot path).
4. **wroid-inject Waydroid session/bridge** (`waydroid_session.rs`,
   `waydroid_bridge.rs`) and diagnostic bins: spawn `runuser`/`waydroid`/`timeout`
   for setup, boot wait, and getevent tracing — never per input frame.

The persistent `UinputTouchInjector` already satisfies "no subprocess per frame";
`play-v2` and `launch-v2` now use it with direct evdev capture. The desktop-user
process owns the hot path. A standalone root-owned typed helper validates and
mounts only the Wroid virtual event node, then accepts only a fixed Android
input readiness probe, cleanup, or EOF recovery. Boot readiness is captured
from the owned desktop-user Waydroid session process and render-property
readiness uses its D-Bus API; no root shell is exposed to the worker. Hub also
requires the helper's permissions and contents
to match the staged release. Exact `root:input` mode `4750`, a side-effect-free
effective-root check, absolute subprocess paths, and environment clearing
remove per-launch sudo without widening the helper protocol. Hub bootstrap is
terminal-free:
Polkit runs only fixed `/usr/bin/install` arguments against a write-sealed
memfd held by a detached, leased user process, and the installed bytes are
verified before readiness is published.

## Duplicated / overlapping logic to consolidate

- **Android activity + package parsing** duplicated almost verbatim in
  `wroid-adb` and `wroid-waydroid` (`parse_current_activity`,
  `component_from_token`, `is_package_name`, package-list parsing). Candidate for
  a shared `wroid-android` parsing module.
- **`AndroidActivity` struct** defined in both adapters and re-mapped to
  `CurrentAndroidActivity` in the CLI — three near-identical types.
- **Two virtual-joystick implementations**: pure `wroid-runtime::VirtualJoystick`
  (production, touch-frame based) vs the CLI `interactive.rs` `JoystickRuntime`
  (swipe-tick based, legacy). Diagonal normalization logic is reimplemented.
- **Two profile schemas + two validators** (`ControlProfile` vs `ProfileV2`) with
  parallel input/action enums and validation. Convergence/migration is a Phase 3
  concern; keep both until v2 is the runtime default.
- **Backend recommendation** logic lives in `doctor.rs` separately from
  `select_input_backend` in `backend.rs` (consistent today; keep them in sync).

## Production architecture target (from architecture-v2)

Unprivileged `wroid-ui` → typed IPC → per-user `wroidd` (session/profile/viewport/
capture/telemetry) → minimal privileged `wroid-helper` (evdev grabs, uinput
creation, restricted Waydroid ops) + persistent injector → Waydroid. CLI becomes
another `wroidd` client. ADB/Waydroid-shell wrappers stay as diagnostics/compat.
Invariants: no subprocess in the input hot path; GUI never root; privileged ops
are a typed allow-list, not arbitrary shell; stateful multitouch; runtime commits
only after injection accepts; every shutdown/focus-loss/failure releases
contacts; software renderer is a blocking perf issue; compatibility backend is
explicit and never silently selected for gaming.

Current status: `launch-v2` already keeps evdev, uinput, profile execution,
focus control, and telemetry unprivileged. Its typed helper is limited to the
temporary LXC bridge and fixed crash recovery. Per-user `wroidd` protocol v1
now owns typed profile preparation and state over a private Unix socket; live
runtime migration and policy-controlled helper activation remain.

## Phase-by-phase implementation order

- **Phase 0 — Audit, baseline, plan (this document).** Done: baseline fixed
  (see below), responsibilities mapped, compat paths identified, plan written.
- **Phase 1 — Stable typed interfaces & backend boundaries.** Split the wide CLI
  `InputExecutor` into focused typed traits with structured errors: Android
  package ops, APK install, display size/density, focused-app inspection,
  Waydroid lifecycle/status, input injection abstraction, diagnostics collection.
  Place them in a clean crate boundary (not the CLI). Keep `wroid-adb`/
  `wroid-waydroid` as adapters implementing them. Make compatibility/degraded
  backend selection explicit in user-facing output. Preserve every CLI command
  and deterministic `auto`/`adb`/`waydroid-shell` behavior. Add tests for ADB
  unavailable, Waydroid unavailable, fallback, deterministic recommendation, and
  structured error messages.
- **Phase 2 — Persistent injector productionization.** Stable virtual-device
  discovery + bridge reconciliation; focus-loss/crash-safe `cancel_all`;
  validate ≥10 simultaneous contacts on a real session; measure capture-to-inject
  p50/p95/p99. Wire host capture → `TouchEngine` → `UinputTouchInjector` behind an
  injection interface, keeping shell input strictly as the explicit compat backend.
- **Phase 3 — Runtime daemon & privilege boundary.** Introduce `wroidd` + typed
  IPC and `wroid-helper` (leased device access); move CLI execution onto the
  daemon API; production session lifecycle, focus ownership, config rollback.
- **Phase 4 — Profile v2 as runtime format & gaming controls.** Migrations,
  layers/modifiers/hold-toggle/dead-zones, relative mouse aim wired into touch
  frames, gamepad support; converge the two joystick implementations.
- **Phase 5 — Desktop hub, compatibility & release.** Library/diagnostics UI,
  visual controls editor, APK ABI/format inspection, split-APK/data packages,
  multi-GPU/compositor testing, signed beta packaging.

(Phase numbering here follows the delivery dependency order; it maps onto the
roadmap's Phase 0–5 but leads with interface boundaries as Phase 1 per the
current project prompt.)

## Risky areas

- **uinput/evdev require explicit device permissions and real hardware.**
  `UinputTouchInjector::open` and grabs are untestable in CI without
  `/dev/uinput` and a device; keep them behind the `EventSink` trait so logic is
  unit-tested with a fake sink. On the validated host, the desktop user has the
  required `input` group access and rootless uinput creation is smoke-tested.
- **Exclusive evdev grab** can lock the desktop out of the device; only grab a
  validated device, and guarantee release on every exit path (Drop already does
  this for keyboard/mouse).
- **Privilege boundary**: the helper must stay a typed allow-list. Any "run shell
  as root" shortcut is a regression against architecture-v2.
- **Two schemas / two joysticks**: divergence risk until convergence; do not let
  v2 silently become the runtime format before migrations exist.
- **Backend determinism**: `auto` selection and doctor recommendation must remain
  consistent and never silently pick the compat backend for gaming mode.
- **sudo/session env launch** is environment-specific (DBus/Wayland) and fragile;
  keep the documented split workflow (launch as user, run keymapper with
  `--no-launch`).

## Manual Waydroid / uinput / evdev test notes

These require a real Linux host with Waydroid; they are not run in CI. See also
`docs/uinput-testing.md`, `docs/keyboard-input-testing.md`,
`docs/waydroid-keyboard-testing.md`, `docs/waydroid-input-bridge.md`,
`docs/host-mouse-input.md`, `docs/live-keyboard-cli.md`,
`docs/runtime-benchmarks.md`.

- **Doctor first**: `cargo run -p wroid-cli --bin wroid -- doctor` and
  `... doctor --backend waydroid-shell`. Expect the `IP UNKNOWN` warning when
  Waydroid runs but ADB cannot connect.
- **uinput smoke**: `sudo .../wroid-uinput-smoke` then verify the virtual device
  in Android via `getevent`; confirm contacts appear and release.
- **evdev capture**: `cargo run -p wroid-input --bin wroid-mouse-capture --
  /dev/input/eventN --max-events 20`; keyboard dump via `wroid-evdev-dump`.
- **Live keyboard → Android**: `wroid-live-keyboard` / `wroid-waydroid-keyboard-
  smoke` to drive WASD into a temporary managed Waydroid session with hold
  reaffirmation.
- **Compat gameplay**: `sudo .../wroid play <profile> --backend waydroid-shell
  --scale-to-current` (terminal must stay focused).
- **Latency/contacts (Phase 2 gates)**: `wroid-bench-host`,
  `wroid-waydroid-touch-bench`; record median/p95/p99/max and ≥10 contacts.

## What must NOT be done

- No anti-cheat bypasses, integrity/attestation spoofing, or stealth/hiding of
  the input source. Input remains transparent.
- No subprocess creation in the gaming input hot path. ADB and `waydroid shell`
  input may remain only as explicit compatibility/debug backends.
- No project rewrite from scratch; evolve crates incrementally.
- No removal of working CLI behavior.
- No GUI running as root; no arbitrary-shell privileged API.
- No video capture/encode/decode path for normal local play.
- No cosmetic-only UI work substituting for runtime/architecture work.

## Baseline issues found and fixed in Phase 0

- `cargo fmt --all -- --check` failed (formatting drift in
  `wroid-core/src/profile_v2.rs` and several `wroid-inject` bins). Fixed by
  `cargo fmt --all`.
- `cargo clippy --workspace --all-targets -- -D warnings` failed with `dead_code`
  in `wroid-input` because `mouse.rs` was compiled only via
  `#[path = "../mouse.rs"] mod mouse;` inside the `wroid-mouse-capture` bin, so
  parts of its public API (`is_zero`, `add_saturating`, `scaled`, `set_nonblocking`,
  `ungrab`, the `Ungrab`/`Configure` error variants, `scale_axis`) looked unused.
  Fixed by promoting `mouse.rs` to a real library module (`pub mod mouse;` in
  `wroid-input/src/lib.rs`) and having the bin import `wroid_input::mouse::...`.
  No behavior change; the mouse unit tests now run as part of the library suite.
- After the fix all three checks pass and `cargo test --workspace` is green.
