# Live keyboard CLI entrypoint

`wroid-live-keyboard` is the first user-facing entrypoint for the persistent Waydroid keyboard path. It reuses the same low-latency runtime as the development smoke test:

```text
physical keyboard evdev
  -> W/A/S/D normalization
  -> persistent VirtualJoystick
  -> TouchEngine
  -> Type-B uinput touchscreen
  -> temporary Waydroid LXC bridge
  -> Android InputReader
```

This keeps subprocess creation out of the input hot path. The process starts once, keeps the virtual touchscreen open, captures the selected keyboard through evdev, and submits persistent multitouch frames through uinput.

## Build

```bash
cargo build --locked --release -p wroid-cli --bin wroid-live-keyboard
```

The legacy smoke-test binary remains available and now calls the same reusable runtime:

```bash
cargo build --locked --release -p wroid-inject --bin wroid-waydroid-keyboard-smoke
```

## Run

Stop Waydroid before the tool changes the temporary LXC bridge:

```bash
waydroid session stop 2>/dev/null || true
sudo waydroid container stop 2>/dev/null || true
```

Start live control with the physical keyboard event node:

```bash
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050
```

The tool starts Waydroid as the original desktop user from the `sudo` environment, opens the full UI by default, waits until Android detects `Wroid Gaming Touchscreen`, then maps W/A/S/D onto one persistent Android touch contact. Press Esc to stop.

## Useful flags

```bash
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --no-grab
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --no-ui
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --no-trace
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --reaffirm-ms 33
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --no-reaffirm
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --hold-log-ms 500
sudo ./target/release/wroid-live-keyboard /dev/input/event7 1920 1050 --no-hold-log
```

`--no-grab` is diagnostic-only. The default exclusive grab prevents W/A/S/D from leaking into the desktop while Android control is live.

## Recovery

If the process is interrupted before normal cleanup, remove the managed bridge explicitly:

```bash
sudo ./target/release/wroid-live-keyboard --cleanup
```

This removes only the Wroid-managed temporary Waydroid include and bridge file. It does not modify Android images, application data, or unrelated Waydroid configuration.
