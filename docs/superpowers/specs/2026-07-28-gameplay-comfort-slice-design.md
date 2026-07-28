# Gameplay Comfort Slice Design — `wroid play-v2`

Date: 2026-07-28
Status: approved by user (2026-07-28)

## 1. Goal

One command turns a profile v2 JSON into a playable Waydroid session with
low-latency persistent-input controls:

- WASD-driven virtual joystick(s) with dead zones and hold reaffirmation;
- toggle-key mouse aim with sensitivity scaling and automatic recentering;
- tap bindings on keyboard keys and mouse buttons;
- deterministic cleanup of every contact and device on any exit path.

First reference game: **Brawl Stars** (two virtual joysticks, no mouse aim,
runs in Waydroid without an integrity wall). Ready-made FPS profiles for
**Standoff 2** and **PUBG Mobile** ship in the same iteration. The user
installs and configures the games themselves.

## 2. Non-goals (explicit)

- GUI, overlay controls editor, game library (roadmap Phase 4).
- XAPK/APKM/OBB install flows, ARM translation, graphics/Vulkan tuning,
  audio tuning, gamepad, gyroscope (later iterations).
- Window focus tracking (needs compositor integration; daemon phase).
- Daemon/IPC/privileged helper (roadmap Phase 2-3; this slice's engine is
  designed so the daemon wraps it unchanged later).
- Anti-cheat bypasses, integrity spoofing, input-source hiding (project
  policy, non-negotiable). Games that refuse Waydroid at the integrity level
  are a user setup concern, not a Wroid feature.

## 3. Architecture and data flow

```text
wroid play-v2 <profile.json> [--keyboard <path>] [--mouse <path>]
              [--resolution <WxH>] [--managed-bridge]
  |
  |-- load + validate ProfileV2                     (wroid-core)
  |-- resolve resolution (`wm size` via adapter, else --resolution, else 1920x1080)
  |-- ensure Waydroid session running (start as desktop user if stopped; NOT hot path)
  |-- RuntimeControlPlan::from_profile_v2           (wroid-runtime)
  |-- create virtual touchscreen (UinputTouchInjector, persistent fd)
  |-- wait until the device is visible inside Android (getevent listing)
  |-- EVIOCGRAB keyboard + mouse (rootless: user in `input` group)
  |
  `-- event loop (single thread owns TouchEngine):
        evdev events -> mpsc -> normalized host events -> control handlers:
          key cluster (WASD) -> VirtualJoystick (dead zone, reaffirm)
          relative mouse     -> MouseAimController (toggle, sensitivity, recenter)  [NEW]
          keys / mouse btns  -> tap bindings (press = DOWN+UP, or held tap)
        -> TouchEngine.apply(frame) -> injector.inject(frame)
  |
  exit (exit key / Ctrl-C / injector error / panic):
        disable capture -> ungrab devices -> cancel_all contacts
        -> remove virtual device -> print session report
```

Component placement:

| Component | Crate | Notes |
| --- | --- | --- |
| Profile v2 schema additions + validation | `wroid-core` | additive, optional fields only |
| `MouseAimController` | `wroid-runtime` | new, pure, unit-tested with recording fake injector |
| `TouchEngine`, `VirtualJoystick`, `RuntimeControlPlan` | `wroid-runtime` | reused as-is |
| `EvdevKeyboard`, `EvdevMouse` | `wroid-input` | reused as-is |
| `game_session` module (session runner) | `wroid-inject` | library module (not a bin); orchestrates capture -> engine -> injector; rootless-first device setup with managed-bridge fallback |
| `play-v2` subcommand | `wroid-cli` | thin: args, profile load, resolution, delegates to `wroid-inject` runner, prints report |

The existing `wroid-waydroid-game-session` binary is the prototype this slice
productizes; its proven pieces (session start, boot wait, device visibility
wait, capture loops, reaffirmation) are mined and moved into the library
module rather than duplicated.

## 4. Mouse aim controller (the core new mechanic)

Pure state machine in `wroid-runtime`. Owns its aim contact(s); never does I/O.

### Configuration (materialized from profile v2)

| Field | Type | Default | Meaning |
| --- | --- | --- | --- |
| `region` | pixel rect | from profile | camera area; contact is clamped to it |
| `sensitivity` | rational (num/den) | from profile | scales relative motion |
| `toggle_key` | key name, optional | none | when absent, aim is active for the whole session |
| `recenter_threshold` | 0.1-1.0 | 0.7 | fraction of min(region half-w, half-h) that triggers recenter |
| `recenter_gap_ms` | >= 0 | 0 | if > 0, use two-step recenter with a timed gap (for games sensitive to instant re-touch) |
| `ads_multiplier` | 0.1-1.0, optional | none | sensitivity multiplier while right mouse button is held |
| `reaffirm_ms` | optional | none | periodic tiny move to keep the contact alive in picky games |

### States

- `Inactive`: relative motion ignored.
- `Active { contact: ContactId, anchor: Point, pending_gap: Option<Instant> }`:
  motion drives the contact.

### Transitions

- Toggle key press while `Inactive` -> emit DOWN at region center -> `Active`.
- Toggle key press / exit key / session teardown while `Active` -> emit UP ->
  `Inactive`.
- Relative motion while `Active`:
  1. accumulate `(dx, dy)`, scale by sensitivity (and `ads_multiplier` if RMB
     held);
  2. `new_pos = clamp(pos + scaled)` to region;
  3. if `dist(new_pos, center) > recenter_threshold * min(half_w, half_h)`:
     **recenter** (below);
  4. else emit MOVE to `new_pos`.

### Recentering (slot swap, single frame)

The runtime invariant forbids one contact appearing twice in a frame, so the
recenter uses **two alternating ContactIds**:

- default path (`recenter_gap_ms = 0`): one frame contains
  `DOWN(new_contact, center)` + `UP(old_contact, old_pos)` — two distinct
  contacts, each appearing once. The game sees the old drag end and a new
  touch begin at the center with no motion between them, so the camera does
  not snap back. The controller continues from `new_contact`.
- gap path (`recenter_gap_ms > 0`): frame 1 = `UP(old_contact)`; after the
  gap, frame 2 = `DOWN(new_contact, center)`. Motion accumulated during the
  gap is buffered and applied after the DOWN.

A recenter counter feeds the session report.

### Coexistence

The aim contact lives in its own slot; tap contacts (LMB fire etc.) and the
WASD joystick contact use other slots of the same `TouchEngine`, so
move + aim + fire simultaneously is ordinary multitouch.

## 5. Profile v2 additions (additive)

`mouse_aim` action gains optional fields: `toggle_key`, `recenter_threshold`,
`recenter_gap_ms`, `ads_multiplier`, `reaffirm_ms`. Validation: threshold in
[0.1, 1.0], gap >= 0, multiplier in [0.1, 1.0], toggle key is a known key
name. Existing profiles validate unchanged; no migrations in this slice.

No other schema changes. `virtual_joystick` (hold/toggle, `reaffirm_ms`,
dead zone) is already sufficient.

## 6. Shipped game profiles

All in `profiles/examples/`, normalized coordinates, landscape, default HUD
assumed; README gets a binding table and instructions for adjusting coords.

- `brawlstars-v2.json`: WASD -> move joystick (hold, reaffirm 50 ms);
  arrow keys -> attack joystick (hold — releasing fires in that direction,
  matching the game's release-to-shoot); Space -> tap super; E -> tap gadget.
- `standoff2-v2.json`: WASD move; mouse aim over the right half (toggle
  Tab); LMB fire tap; R reload; Space jump; C crouch; 1/2 weapons; F action.
- `pubg-v2.json`: WASD move; mouse aim (toggle Tab); LMB fire; F loot;
  M map; C crouch; Z prone; Q/E lean taps.

## 7. Session lifecycle and cleanup

- **Rootless by default**: requires the user in the `input` group and
  `/dev/uinput` accessible (both true on the reference machine). Keyboard and
  mouse are auto-detected via `/dev/input/by-id` (capability-validated) with
  `--keyboard`/`--mouse` overrides.
- **Device visibility**: after creating the touchscreen, wait (timeout 10 s)
  for Android's getevent listing to show it. On timeout: cleanup, then an
  actionable error pointing at `--managed-bridge` (the existing root bridge
  path from `waydroid_bridge.rs`) — no automatic sudo.
- **Waydroid session**: if stopped, start as the desktop user and wait for
  boot (existing `DesktopWaydroidSession` pieces). Wroid never stops a
  session it found running; sessions it started are left running too
  (documented).
- **Exit paths** — exit key (default Esc), Ctrl-C (SIGINT), injector error,
  panic: all funnel into one teardown that disables capture, ungrabs,
  cancels every contact, removes the device, and prints the report. Drop
  implementations already guarantee device release; the runner adds a
  scopeguard for contact cancellation.
- **Session report**: frames submitted, peak simultaneous contacts, recenter
  count, host pipeline p50/p95 (same measurement pattern as
  `wroid-bench-host`).

## 8. Error handling

| Failure | Behaviour |
| --- | --- |
| profile v2 invalid | joined validation errors, exit 2 |
| keyboard/mouse path wrong or missing capabilities | list `/dev/input/by-id`, hint flags, exit 2 |
| `/dev/uinput` open denied | hint: `input` group / udev rule, exit 1 |
| waydroid missing or container broken | point at `wroid doctor`, exit 1 |
| device not visible in Android within timeout | cleanup, hint `--managed-bridge`, exit 1 |
| injector error mid-session | cancel_all, report, exit 1 |

## 9. Testing

CI (unit, no hardware):

- `MouseAimController`: scaling, clamping, threshold trigger, slot-swap frame
  contents (DOWN-new + UP-old in one frame, each contact once), residual
  motion carry, gap-path buffering, ADS multiplier, toggle on/off, deactivate
  on teardown.
- Coexistence: aim + joystick + tap contacts through `TouchEngine` into a
  recording fake injector.
- Profile v2: new-field validation accept/reject cases; old profiles still
  validate.
- Workspace gates stay green: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.

Manual gates on the reference machine (games installed by the user):

1. `wroid play-v2 profiles/examples/brawlstars-v2.json` is playable: WASD
   moves, arrows aim and shoot on release, Space fires super.
2. getevent trace shows 3+ simultaneous contacts (move + attack + super).
3. Aim stress (Standoff 2 profile): ten full-speed circles — no contact
   stuck at the edge, no camera snap-back, recenter counter increments.
4. Session report shows host pipeline p95 < 5 ms (performance budget).
5. Esc exit: contacts cancelled (getevent quiet), grabs released, report
   printed.

## 10. Risks

- Rootless-created uinput devices may not be visible inside the Waydroid
  container on some setups. Mitigation: visibility probe is part of the
  runner; `--managed-bridge` fallback exists and is documented; the probe is
  verified on the reference machine in the first implementation tasks.
- Game HUDs shift across resolutions and patches. Mitigation: normalized
  coordinates, per-profile adjustment documented; overlay editor later.
- PUBG Mobile / Standoff 2 integrity checks may block login or matchmaking
  in Waydroid regardless of Wroid. This is documented honestly in the
  profile README notes; no evasion work (project policy).
