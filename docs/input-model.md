# Input Model

Wroid profiles describe Android actions independently from the backend used to execute them.

## Profile Shape

Each profile contains:

- `name`: human-readable profile name.
- `package_name`: Android package launched by `run` and `run-profile`.
- `resolution`: Android surface size used when coordinates were authored.
- `bindings`: named input/action pairs.

Binding names must be non-empty and unique within a profile.

## Inputs

Legacy profile inputs:

- `key`: one keyboard key, for example `{ "kind": "key", "key": "f" }`.
- `key_cluster`: four directional keys, for example `w/a/s/d`.
- `mouse_button`: stored by the schema but not executed by the legacy terminal runner.

Profile v2 prototype inputs add:

- `mouse_move`: relative host pointer motion for future `mouse_aim` bindings.

Host input capture is split from profile execution:

- `EvdevKeyboard` normalizes physical keyboard events into profile-visible key events.
- `EvdevMouse` normalizes relative x/y motion, wheel deltas, and mouse buttons.
- The production daemon should own focus, leases, and grabs before forwarding host events into the runtime.

## Actions

Supported legacy actions:

- `tap`: emits Android input tap at one point.
- `swipe`: emits Android input swipe from one point to another with a duration.
- `virtual_joystick`: tracks a directional key cluster and repeatedly emits a swipe from the joystick center to a computed target point.

Profile v2 prototype actions:

- `tap`
- `virtual_joystick`
- `mouse_aim`
- `macro`

`mouse_aim` and `macro` remain prototype-only until runtime behavior is wired into the daemon-managed session.

## Virtual Joystick

A virtual joystick binding stores:

- `center`: Android coordinate for the joystick center.
- `radius`: maximum movement distance.
- `tick_ms`: how often to emit movement while held.
- `swipe_duration_ms`: duration sent to Android `input swipe`.

Direction vector rules:

- Up moves toward negative y.
- Down moves toward positive y.
- Left moves toward negative x.
- Right moves toward positive x.
- Opposite directions cancel on the same axis.
- Diagonals are normalized so the target remains on the joystick radius instead of exceeding it.

Example:

```json
{
  "name": "movement",
  "input": {
    "kind": "key_cluster",
    "up": "w",
    "left": "a",
    "down": "s",
    "right": "d"
  },
  "action": {
    "kind": "virtual_joystick",
    "center": { "x": 320, "y": 640 },
    "radius": 120,
    "tick_ms": 80,
    "swipe_duration_ms": 70
  }
}
```

## Relative Mouse Capture

The current relative mouse slice is host-side capture and normalization only. It is validated with:

```sh
cargo run -p wroid-input --bin wroid-mouse-capture -- /dev/input/event7 --max-events 20
```

Normalized events include:

- relative motion: `dx`, `dy`;
- wheel deltas: vertical and horizontal;
- mouse buttons: left, right, middle, side, extra.

The future runtime path should map `mouse_move` to `mouse_aim` by applying profile v2 sensitivity and aim-region constraints, then emitting stateful touch frames through `TouchEngine` and the persistent Type-B injector. The shell compatibility backend should not be used for gaming mouse aim.

## Coordinate Scaling

Profiles can be scaled to another Android surface resolution.

- Tap points scale by x/y axis.
- Swipe endpoints scale by x/y axis.
- Virtual joystick centers scale by x/y axis.
- Virtual joystick radius uses the average of the horizontal and vertical scale factors because it is a single scalar.

Use `--scale-to-current` on `play`, `run`, `run-profile`, or `binding run` when the current Waydroid surface differs from the profile's authored resolution.

## Current Limitations

- The legacy terminal runner still requires terminal focus.
- Production global input capture is not daemon-managed yet.
- Relative mouse events are captured and normalized, but Android mouse aim is not wired yet.
- Profile v2 is validated by a prototype binary and is not the default runtime profile format yet.
- Macros are not implemented in the runtime.
