# Wroid Gaming Hub SPEC

Wroid Gaming Hub is a Linux gaming frontend for Android games running through Waydroid.

## Goal

Build a BlueStacks-like gaming layer for Linux focused on Android games, with profile-driven controls and Waydroid-friendly workflows.

## Current Scope

The project is currently a CLI-first foundation. It supports:

- JSON control profiles.
- Tap and swipe bindings.
- Terminal keymapper execution.
- Virtual joystick profile bindings and terminal execution.
- ADB and Waydroid shell input backends.
- App list, launch, install APK, and current activity commands.
- Device screen and density detection.
- Local profile registry management.
- Profile coordinate scaling.
- Doctor diagnostics and backend recommendation.

## Core Principles

- Rust workspace.
- Waydroid remains an external dependency.
- ADB and Waydroid shell are explicit backends.
- Profiles are stored as JSON.
- CLI orchestration stays in `wroid-cli`.
- Profile model and validation stay in `wroid-core`.
- Command wrappers stay in `wroid-adb` and `wroid-waydroid`.

## Out of Scope

- GUI.
- Overlay editor.
- Global input capture.
- evdev/uinput.
- Gamepad mapping.
- Mouse aim.
- Macro execution.
- XAPK/APKM/OBB install flows.
- Anti-cheat bypasses or protection evasion.

## Acceptance Gates

- `cargo fmt`
- `cargo test --workspace`
- Example profiles validate.
- Existing CLI behavior remains compatible.
- New behavior has focused unit coverage.
