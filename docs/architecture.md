# Architecture

MVP-0 is a small Rust workspace with clear crate boundaries.

## Crates

- `wroid-core` owns the control profile JSON schema, loading/saving, and validation.
- `wroid-adb` wraps the external `adb` command for device discovery and touch input.
- `wroid-waydroid` wraps the external `waydroid` command for basic status/session operations and shell input.
- `wroid-cli` exposes the initial command surface and connects validated profile actions to the selected input backend.

## Data Flow

1. The CLI loads a JSON control profile through `wroid-core`.
2. `wroid-core` validates binding names, resolution, action support, and action coordinates.
3. For `binding run`, the CLI finds the requested binding by name.
4. Supported actions are executed through the selected input backend. `auto` uses ADB when `adb devices` reports at least one device in the `device` state, otherwise it uses `waydroid shell input`.

## MVP-0 Boundaries

The MVP deliberately excludes GUI, overlay editing, evdev/uinput, gamepad support, and macro execution. Placeholder action variants are present in the profile schema so future profile files have a stable direction, but they fail validation until the behavior exists.
