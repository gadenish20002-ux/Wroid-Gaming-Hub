# Roadmap

## Completed CLI Foundation

- Rust workspace with `wroid-core`, `wroid-adb`, `wroid-waydroid`, and `wroid-cli`.
- Profile schema, JSON loading/saving, validation, and example profiles.
- ADB and Waydroid command wrappers.
- `wroid binding run` for single tap/swipe bindings.
- `wroid play` terminal keymapper.
- `wroid run` and `wroid run-profile` app launch plus keymapper workflows.
- Device screen and density detection.
- Profile creation, editing, scaling, import/export, duplicate, rename, and remove.
- Android app list, launch, install APK, and current activity commands.
- Virtual joystick profile model, scaling, and terminal execution.
- Doctor diagnostics with backend recommendation and Waydroid UNKNOWN-IP warning.

## Current Limitations

- No GUI.
- No overlay editor.
- No global input capture.
- No evdev/uinput backend.
- No gamepad mapping.
- No mouse aim behavior.
- No macro execution.
- No XAPK/APKM/OBB install flow.
- No anti-cheat bypasses or protection evasion.

## Next Useful Milestones

1. Improve terminal joystick ergonomics.
   - Better handling for terminals without release events.
   - Optional on-screen debug output for active directions.
   - Clearer runtime warnings when release events are unavailable.

2. Add non-global input backends only when safe.
   - Investigate evdev/uinput with explicit user permissions.
   - Keep behavior transparent and avoid protection evasion.

3. Build a profile authoring workflow.
   - Inspect current app/package.
   - Capture screen size.
   - Create profile from current display.
   - Add bindings from CLI or a future editor.

4. GUI and overlay editor.
   - Not part of the current CLI foundation.
   - Should build on the existing profile schema and validation instead of replacing it.

5. Package install expansion.
   - APK install is implemented.
   - XAPK/APKM/OBB are intentionally out of scope until explicitly designed.
