# Wroid Gaming Hub

Wroid Gaming Hub is a Linux gaming frontend for Waydroid. MVP-0 is a CLI-only skeleton that loads JSON control profiles and executes tap/swipe bindings through ADB.

## Workspace

- `wroid-core`: profile schema, JSON loading/saving, validation
- `wroid-adb`: thin ADB command wrapper
- `wroid-waydroid`: thin Waydroid command wrapper
- `wroid-cli`: `wroid` command-line interface

## Usage

```sh
cargo run -p wroid-cli -- doctor
cargo run -p wroid-cli -- profile validate profiles/examples/shooter-basic.json
cargo run -p wroid-cli -- profile example /tmp/wroid-profile.json
cargo run -p wroid-cli -- input tap 500 400
cargo run -p wroid-cli -- input swipe 400 500 800 500 180
cargo run -p wroid-cli -- binding run profiles/examples/shooter-basic.json fire
```

## Profile Format

Profiles are JSON files with a target resolution and named bindings:

```json
{
  "name": "Shooter Basic",
  "resolution": { "width": 1920, "height": 1080 },
  "bindings": [
    {
      "name": "fire",
      "input": { "kind": "mouse_button", "button": "left" },
      "action": { "kind": "tap", "point": { "x": 1640, "y": 540 } }
    }
  ]
}
```

Supported MVP action kinds are `tap` and `swipe`. `virtual_joystick`, `mouse_aim`, and `macro` exist in the schema as placeholders and intentionally fail validation until implemented.

## Development

```sh
cargo fmt
cargo test --workspace
```
