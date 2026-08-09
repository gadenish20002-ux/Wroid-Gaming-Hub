# Profile v2

Profile v2 is Wroid's normalized-coordinate production format for Controls
Studio and low-latency Waydroid sessions. Legacy pixel-coordinate profiles and
their CLI commands remain separate and supported.

## Coordinates and shape

Points and rectangles use normalized Android viewport coordinates:

```text
x/y/w/h range: 0.0..=1.0
0.0,0.0      : top-left of the Android surface
1.0,1.0      : bottom-right of the Android surface
```

The same profile can therefore materialize to `1280x720`, `1600x900`, or
`1920x1080`. A layered profile looks like this:

```json
{
  "schema_version": 2,
  "name": "Standoff 2 — layered",
  "package_name": "com.axlebolt.standoff2",
  "orientation": "landscape",
  "layers": [
    { "name": "grenades", "activation": { "kind": "hold", "key": "g" } }
  ],
  "bindings": [
    {
      "name": "primary_weapon",
      "input": { "kind": "key", "key": "1" },
      "action": { "kind": "tap", "point": { "x": 0.89, "y": 0.18 } }
    },
    {
      "name": "frag",
      "layer": "grenades",
      "input": { "kind": "key", "key": "1" },
      "action": { "kind": "tap", "point": { "x": 0.70, "y": 0.30 } }
    },
    {
      "name": "fire_mode",
      "modifier": "shift",
      "input": { "kind": "key", "key": "r" },
      "action": { "kind": "tap", "point": { "x": 0.93, "y": 0.60 } }
    }
  ]
}
```

Top-level `layers` is optional and defaults to an empty list. A binding without
`layer` belongs to the always-active implicit `base` layer. A binding without
`modifier` has no chord requirement; absent binding fields are omitted again
when saved, so predecessor profiles remain structurally compatible.

Layer activation kinds are:

- `hold`: active only while its activation key is held;
- `toggle`: flips on each activation-key press and remains in that state after
  the key is released.

A profile can declare at most 64 named layers in addition to Base. Layer names
are resolved once when a runtime plan is prepared.

Supported input kinds are `key`, `key_cluster`, `mouse_button`, and
`mouse_move`. Supported action kinds are `tap`, `hold`, `virtual_joystick`,
`mouse_aim`, and schema-only `macro`; macro execution is not implemented.

## Why the schema version remains 2

Layers, `layer`, and `modifier` are additive fields with Serde defaults. Keeping
`schema_version: 2` lets existing profiles load without migration and avoids
invalidating every installed profile before migration machinery exists. Old
readers also ignore the additive fields, although rewriting a layered document
with an old binary would discard them. A future incompatible shape change must
use a migration and may then justify a version bump.

## Validation rules

General profile and input rules:

- `schema_version` is exactly `2`; profile name and package name are non-empty;
- binding names are non-empty and unique;
- key names are from the supported canonical key set; key clusters have four
  non-empty supported keys;
- mouse buttons are one of `left`, `right`, `middle`, `side`, or `extra`;
- `key`/`mouse_button` pair only with `tap` or `hold`, `key_cluster` pairs with
  `virtual_joystick`, and `mouse_move` pairs with `mouse_aim`;
- normalized points are finite and inside `0.0..=1.0`; aim rectangles have
  positive size and stay inside the viewport;
- joystick radius is finite in `(0.0, 1.0]`; dead zone is finite in
  `[0.0, 1.0)` and smaller than the radius;
- optional joystick and mouse-aim `reaffirm_ms` values are greater than zero;
- mouse sensitivity is finite and positive, while `recenter_threshold` and an
  optional ADS multiplier are finite in `0.1..=1.0`; a mouse-aim toggle uses a
  supported key;
- a macro has at least one step, and every nested step is validated recursively.

Layer rules:

- layer names are non-empty and unique; `base` is reserved case-insensitively;
- no more than 64 named layers may be declared;
- every activation key is supported and unique across layers;
- an activation key cannot be an unmodified Base binding key;
- an activation key cannot be used by a binding inside the layer it activates.

Binding scope and modifier rules:

- `layer` names a declared layer; unknown names are rejected;
- `modifier` is a supported key and differs from a single input key or every
  constituent key of a key cluster;
- `mouse_move` cannot have a modifier; mouse aim is deliberately always live;
- `ctrl+esc` and `ctrl+c` are rejected because the session owns those exit
  hotkeys;
- within the same `(layer, modifier)` scope, a physical key or mouse button can
  drive only one binding; duplicates across different layers or modifiers are
  legal;
- repeated keys within one key cluster are counted once for scope collision
  reporting, but the normal input/action validation still applies.

Controls Studio mirrors these rules before save, and the Rust validator remains
the authoritative boundary.

## Validate and materialize

```sh
cargo run -p wroid-core --bin wroid-profile-v2-validate -- profiles/examples/shooter-v2.json
cargo run -p wroid-core --bin wroid-profile-v2-validate -- \
  profiles/examples/shooter-v2.json --materialize 1920 1080
```

Materialized output names every binding's resolved layer (`base` or a declared
name) and modifier (`none` or a key) beside its concrete Android coordinates.

Schema migration tooling and the remaining daemon-owned gameplay wiring are
separate roadmap items; the layer/modifier schema, editor, materialization, and
direct production dispatch are implemented.
