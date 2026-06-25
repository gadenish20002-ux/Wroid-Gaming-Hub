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

## Tap bindings

Add repeatable tap bindings with `--tap KEY:X,Y`. The point is validated against
the selected Android surface resolution.

Example:

```bash
sudo target/debug/wroid-native-keyboard \
  /dev/input/by-id/usb-Homertech_Hexgears_Gaming_Keyboard-event-kbd \
  1920 1050 \
  --no-grab \
  --no-trace \
  --ready-delay-ms 0 \
  --tap F:1600,820 \
  --tap R:1700,220
```

When live, pressing `F` injects a short Android tap at `1600,820`; pressing `R`
injects a short Android tap at `1700,220`.

## Safer diagnostic run

Use this first when validating a keyboard device, because it avoids exclusive
keyboard grab and disables Android event tracing:

```bash
sudo target/debug/wroid-native-keyboard /dev/input/eventN 1920 1050 --no-grab --no-trace --ready-delay-ms 0
```

## Exit and cleanup

Use `Esc` for normal shutdown. During the live control loop, `Ctrl+C` also
requests graceful shutdown; the loop exits, releases the active touch contact,
stops Waydroid, and removes the temporary input bridge.

If the process is interrupted during early startup, before the live loop is
active, still run the cleanup command and stop the Waydroid session manually:

```bash
sudo target/debug/wroid-native-keyboard --cleanup
waydroid session stop
waydroid status
```

If the status remains stuck or reports `Container: FROZEN`, restart the container
service before the next native input run:

```bash
sudo systemctl restart waydroid-container
```

## Cleanup only

If a previous run exits unexpectedly, remove the temporary bridge with:

```bash
sudo target/debug/wroid-native-keyboard --cleanup
```

## Notes

- This command must run as root because it creates a uinput device and edits the
  temporary Waydroid input bridge.
- Keep a second terminal open while testing exclusive grab.
- `Ctrl+C` is handled gracefully only after the live control loop is active; use
  manual cleanup if startup is interrupted earlier.
- This is the path that should evolve toward daemon/helper ownership; the legacy
  `wroid input ... --backend waydroid-shell` commands remain compatibility and
  diagnostics only.
