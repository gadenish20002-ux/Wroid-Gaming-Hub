# Profile v2 Prototype

Profile v2 is the normalized-coordinate profile format intended for the future
visual controls editor and daemon runtime.

The existing legacy profile schema remains supported. This prototype adds a
validator and example without replacing the current CLI profile commands.

## Why normalized coordinates

Legacy profiles store pixel coordinates for one target resolution. That works for
simple CLI tests, but a BlueStacks-like controls editor needs coordinates that
survive resolution, density, orientation, and window-size changes.

Profile v2 stores touch locations and aim regions in normalized viewport space:

```text
x/y/w/h range: 0.0..=1.0
0.0,0.0      : top-left of the Android surface
1.0,1.0      : bottom-right of the Android surface
```

The runtime can materialize the same profile to `1920x1080`, `1600x900`, or a
cropped/aspect-aware viewport later.

## Current prototype fields

```json
{
  "schema_version": 2,
  "name": "Shooter v2 Example",
  "package_name": "com.example.shooter",
  "orientation": "landscape",
  "bindings": []
}
```

Supported input kinds:

- `key`
- `key_cluster`
- `mouse_button`
- `mouse_move`

Supported action kinds:

- `tap`
- `virtual_joystick`
- `mouse_aim`
- `macro`

## Example

See:

```text
profiles/examples/shooter-v2.json
```

Validate it:

```sh
cargo run -p wroid-core --bin wroid-profile-v2-validate -- profiles/examples/shooter-v2.json
```

Validate and materialize normalized coordinates to a concrete surface:

```sh
cargo run -p wroid-core --bin wroid-profile-v2-validate -- profiles/examples/shooter-v2.json --materialize 1920 1080
```

## Migration direction

Profile v2 is not yet wired into `wroid-cli run` or `wroid-live-keyboard`. The
intended order is:

1. keep the validator strict and stable;
2. add a library module for profile v2 once the schema stops changing daily;
3. add a legacy -> v2 migration command;
4. make `wroidd` evaluate profile v2 bindings;
5. make the visual controls editor read/write only profile v2.

## Validation rules now enforced

- `schema_version` must be `2`;
- profile and package names must be non-empty;
- binding names must be non-empty and unique;
- all normalized points must be finite and within `0.0..=1.0`;
- aim regions must be finite, positive-size, and inside the viewport;
- joystick radius must be finite and within `0.0..=1.0`;
- optional `reaffirm_ms` must be greater than zero;
- mouse sensitivity must be finite and greater than zero;
- macros must contain at least one step.
