# Wroid Gaming Hub SPEC

Wroid Gaming Hub is a Linux gaming frontend for Waydroid.

Goal:
Build a BlueStacks-like gaming layer for Linux focused on Android games.

MVP goal:
Load a control profile JSON and execute tap/swipe bindings through ADB.

Core principles:
- Rust core
- Waydroid is an external dependency
- ADB backend first
- evdev/uinput later
- profiles stored as JSON
- no GUI in MVP-0
- no overlay editor in MVP-0
- no macros in MVP-0
- no gamepad in MVP-0

Architecture:
- wroid-core: profile schema, validation, action execution
- wroid-adb: ADB command wrapper
- wroid-waydroid: Waydroid command wrapper
- wroid-cli: CLI interface

Acceptance criteria for MVP-0:
- cargo test passes
- example profile validates
- invalid coordinates fail validation
- duplicate binding names fail validation
- CLI can run a binding by name
