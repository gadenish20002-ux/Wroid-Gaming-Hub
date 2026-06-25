# Waydroid FROZEN recovery

During native input testing, Waydroid can report a split state:

```text
Session:   RUNNING
Container: FROZEN
IP address: UNKNOWN
```

In that state the compatibility `waydroid shell` backend may still answer some
root-only diagnostic commands, but the native keyboard path expects a clean
session start because it temporarily installs a uinput bridge into the Waydroid
container configuration before launching Android.

## Recovery sequence

Run this before starting `wroid-native-keyboard` when the container is FROZEN:

```bash
sudo target/debug/wroid-native-keyboard --cleanup
waydroid session stop
sudo systemctl restart waydroid-container
waydroid status
```

Then start the native path again with the real keyboard event node:

```bash
sudo target/debug/wroid-native-keyboard \
  /dev/input/by-id/usb-Homertech_Hexgears_Gaming_Keyboard-event-kbd \
  1920 1050 \
  --no-grab \
  --no-trace \
  --ready-delay-ms 0
```

Once the safe run works, test exclusive grab from a second terminal:

```bash
sudo target/debug/wroid-native-keyboard \
  /dev/input/by-id/usb-Homertech_Hexgears_Gaming_Keyboard-event-kbd \
  1920 1050 \
  --no-trace
```

## Confirmed working smoke path

A successful run should print:

```text
Created Wroid Gaming Touchscreen at /dev/input/eventX
Installed a temporary, reversible Waydroid LXC input bridge.
Waydroid container is RUNNING.
Android boot_completed=1.
Android detected Wroid Gaming Touchscreen.
Controls are live: W/A/S/D move one persistent Android touch contact ...
```

Pressing `W/A/S/D` should emit direction logs. Press `Esc` to release the active
contact, stop Waydroid, and remove the temporary input bridge.
