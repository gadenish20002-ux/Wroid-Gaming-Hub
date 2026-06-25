# Native Keyboard Path

This is the first production-oriented gameplay smoke path. Unlike the legacy
`adb input` and `waydroid shell input` compatibility backends, this path keeps a
persistent Linux virtual touchscreen open and routes host keyboard events through
Wroid runtime state.

Pipeline:

```text
host evdev keyboard -> wroid-input -> TouchEngine / VirtualJoystick -> UinputTouchInjector -> Waydroid
```

No subprocess is spawned per input frame. Shell/Waydroid commands are used only
for setup, diagnostics, bridge installation, and cleanup.

## Binaries

Two binary names currently run the same native keyboard smoke path:

- `wroid-live-keyboard`: legacy diagnostic name retained for compatibility.
- `wroid-native-keyboard`: preferred name for the native production-path smoke test.

## Basic run

Replace `/dev/input/eventN` with the physical keyboard event node.

```bash
cargo build -p wroid-cli --bin wroid-native-keyboard
sudo target/debug/wroid-native-keyboard /dev/input/eventN 1920 1050
```

The command creates a temporary uinput touchscreen, installs a reversible
Waydroid LXC input bridge, starts Waydroid as the desktop user, and maps WASD to
one persistent Android touch contact. Press `Esc` to exit and trigger cleanup.

## Safer diagnostic run

Use this first when validating a keyboard device, because it avoids exclusive
keyboard grab and disables Android event tracing:

```bash
sudo target/debug/wroid-native-keyboard /dev/input/eventN 1920 1050 --no-grab --no-trace --ready-delay-ms 0
```

## Cleanup

If a previous run exits unexpectedly, remove the temporary bridge with:

```bash
sudo target/debug/wroid-native-keyboard --cleanup
```

## Notes

- This command must run as root because it creates a uinput device and edits the
  temporary Waydroid input bridge.
- Keep a second terminal open while testing exclusive grab.
- This is the path that should evolve toward daemon/helper ownership; the legacy
  `wroid input ... --backend waydroid-shell` commands remain compatibility and
  diagnostics only.
