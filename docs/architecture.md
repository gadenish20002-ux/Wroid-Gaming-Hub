# Architecture

Wroid Gaming Hub is a Rust workspace for a CLI-first Linux gaming layer on top of Waydroid. It loads JSON control profiles, launches Android packages, and translates terminal keyboard input into Android tap, swipe, keyevent, and virtual joystick shell input.

## Crates

- `wroid-core` owns the profile data model, JSON loading/saving, validation, and coordinate scaling.
- `wroid-adb` wraps the external `adb` command for device discovery, package management, activity inspection, display queries, and shell input.
- `wroid-waydroid` wraps the external `waydroid` command for status, app management, focused activity inspection, display queries, and shell input.
- `wroid-cli` owns command parsing, registry paths, backend selection, workflow orchestration, terminal input handling, and user-facing output.

## Command Flow

1. The CLI parses a command with `clap`.
2. Profile commands load and validate JSON through `wroid-core`.
3. Backend-aware commands choose `adb`, `waydroid-shell`, or `auto`.
4. `auto` prefers ADB only when `adb devices` reports at least one device with state `device`; otherwise it falls back to `waydroid-shell`.
5. Device and scaling commands query `wm size` and `wm density` through the selected backend.
6. Run commands launch the profile package unless `--no-launch` is set, wait for the launch delay, then start the terminal keymapper.

## Profile Registry

The registry is a user-owned directory:

- `$XDG_CONFIG_HOME/wroid/profiles` when `XDG_CONFIG_HOME` is set.
- `~/.config/wroid/profiles` otherwise.
- When running under `sudo`, registry resolution uses `SUDO_USER` and `SUDO_UID` so shell input can run as root while profiles still come from the desktop user.

Registry import validates profile JSON. Export, duplicate, and rename operate on profile files without reserializing them, preserving existing JSON formatting.

## Interactive Input

The interactive runner is terminal-focused. It listens to crossterm key events from the focused terminal, not global desktop input.

- `key` inputs trigger tap or swipe actions on press/repeat.
- `key_cluster` inputs track directional press/release state for virtual joystick actions.
- Virtual joystick diagonals are normalized so diagonal targets do not exceed the configured radius.
- Joystick ticks emit repeated Android `input swipe center target duration` commands while a direction is held.

This is not an evdev/uinput implementation. It does not capture keys globally and does not hide input from games or anti-cheat systems.

## Boundaries

Implemented CLI foundation:

- Profile creation, validation, editing, scaling, import/export, duplicate, rename, and remove.
- App list, launch, APK install, and current activity inspection.
- Device screen, density, and doctor diagnostics.
- Tap, swipe, keyevent, and terminal virtual joystick execution.

Not implemented yet:

- GUI or overlay editor.
- Global input capture.
- evdev/uinput.
- Gamepad mapping.
- Mouse aim behavior.
- Macro execution.
- XAPK/APKM/OBB install flows.
- Anti-cheat bypasses or protection evasion.
