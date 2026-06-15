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

Supported inputs:

- `key`: one terminal key, for example `{ "kind": "key", "key": "f" }`.
- `key_cluster`: four directional terminal keys, for example `w/a/s/d`.

`mouse_button` remains a schema variant but is not executed by the current terminal runner.

## Actions

Supported actions:

- `tap`: emits Android input tap at one point.
- `swipe`: emits Android input swipe from one point to another with a duration.
- `virtual_joystick`: tracks a directional key cluster and repeatedly emits a swipe from the joystick center to a computed target point.

Placeholder actions:

- `mouse_aim`
- `macro`

These placeholders intentionally fail normal profile validation until behavior is implemented.

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

## Coordinate Scaling

Profiles can be scaled to another Android surface resolution.

- Tap points scale by x/y axis.
- Swipe endpoints scale by x/y axis.
- Virtual joystick centers scale by x/y axis.
- Virtual joystick radius uses the average of the horizontal and vertical scale factors because it is a single scalar.

Use `--scale-to-current` on `play`, `run`, `run-profile`, or `binding run` when the current Waydroid surface differs from the profile's authored resolution.

## Current Limitations

- The terminal window must stay focused.
- Key tracking is not global desktop input capture.
- Release tracking depends on terminal support for enhanced keyboard events.
- There is no evdev/uinput backend yet.
- Mouse aim and macros are not implemented.
