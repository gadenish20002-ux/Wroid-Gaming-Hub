# Input Model

Wroid separates host input capture, profile matching, Android actions, and the
persistent Type-B touch injector. Profile v2 is used by Controls Studio and
production `play-v2`/`launch-v2` sessions; legacy pixel-coordinate profiles keep
their terminal runner and CLI commands.

## Inputs and actions

Profile v2 inputs are:

- `key`: one canonical keyboard key;
- `key_cluster`: four directional keys, normally W/A/S/D;
- `mouse_button`: left, right, middle, side, or extra;
- `mouse_move`: relative pointer motion for mouse aim.

The executable compatibility matrix is strict:

- `key` or `mouse_button` -> `tap` or sustained `hold`;
- `key_cluster` -> `virtual_joystick`;
- `mouse_move` -> `mouse_aim`.

`macro` remains a validated schema action but is not executable. Invalid pairs
are rejected when the profile is saved or loaded instead of being ignored at
runtime.

`EvdevKeyboard` and `EvdevMouse` normalize physical events. The game-session
runtime resolves them against a pre-materialized control plan and submits
synchronized touch frames through the persistent injector. The shell backend is
not used for the gaming hot path.

## Layers and modifiers

Base is implicit and always available. Named layers use either a held activation
key or a press-to-toggle activation key. Bindings may also require one modifier
key. Layer IDs, modifier keys, physical inputs, and modifier-sibling relations
are resolved before gameplay, so dispatch does not allocate or compare strings.

When several active named layers bind the same physical key or mouse button,
the highest `LayerId` wins: this is the later layer in profile declaration
order. If it has no binding for that input, precedence falls through to the next
active layer and finally Base. Only one layer owns a physical input at a time;
overlapping layers never double-fire it.

Inside the selected layer, an available modifier binding suppresses its
unmodified sibling for that same physical key or mouse button. For example,
holding Shift makes `Shift+R` win over plain `R` in the same layer. Key-cluster
suppression is evaluated per constituent key, so a chord for Shift+W does not
silence A/S/D movement.

Layer activation keys are consumed before normal binding dispatch. Modifier
state is updated before reconciliation, but a modifier key may still be a normal
binding on another physical-input path when validation permits it.

## Press, release, and continuous reconciliation

Presses are gated by the layer, modifier, sibling, and selected-layer condition.
Releases are not gated by the current condition. The runtime records active
owners and releases a contact whenever its action key goes up or its modifier or
layer becomes unavailable. This prevents stuck touches when events arrive in
the order `Shift down`, `W down`, `Shift up`, `W up`.

For continuous `hold` and hold-mode `virtual_joystick` actions, every relevant
key, modifier, or layer state change computes the complete desired state. Stale
contacts and joystick directions are released or neutralized first; replacement
Base/lower-layer actions start afterward. Consequently a key already held can
move cleanly between modifier or layer owners without simultaneous contacts.
Toggle joysticks change only on a physical press edge and neutralize when their
binding becomes unavailable.

`tap` is intentionally a zero-latency edge action. A modifier must already be
held when the action key is pressed for the chorded tap to fire. Wroid does not
delay an unmodified tap waiting for a possible chord, and it does not refire a
tap merely because a modifier or layer changes while the action key remains
held. Release the action key and press it again under the desired scope.

On focus loss, F12 release, stop, or failure, suspension best-effort cancels all
contacts, neutralizes joysticks, and clears held keys, modifiers, toggled layers,
and binding ownership before capture can resume.

## Mouse aim

Mouse aim is deliberately always live regardless of selected layer or held
modifier. Its own optional `toggle_key` controls enablement, and right mouse ADS
scaling remains independent of profile layers. Validation therefore rejects
`modifier` on `mouse_move`; layer selection never gates motion dispatch.

Relative motion applies sensitivity, ADS scaling, sub-pixel accumulation,
aim-region constraints, and recentering before submitting stateful touch frames.
Normalized mouse events include x/y motion, horizontal/vertical wheel deltas,
and all five supported mouse buttons.

## Virtual joystick

A profile v2 joystick stores normalized center/radius geometry, a dead zone,
Hold or Toggle mode, and optional reaffirm interval. Opposite directions cancel
on each axis, and diagonals are normalized so the target remains on the joystick
radius. Up is negative y; down is positive y; left is negative x; right is
positive x.

## Legacy coordinate scaling

Legacy profiles store an authored pixel resolution. Tap/swipe endpoints and
joystick centers scale by axis; joystick radius uses the average horizontal and
vertical scale because it is a scalar. `--scale-to-current` remains available
for the legacy `play`, `run`, `run-profile`, and `binding run` paths.

## Remaining limitations

- The legacy terminal runner still requires terminal focus.
- Remaining direct CLI execution paths are not all daemon-owned.
- Macro execution and physical/virtual gamepad mapping are not implemented.
