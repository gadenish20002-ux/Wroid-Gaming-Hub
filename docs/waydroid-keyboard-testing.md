# Live physical keyboard control inside Waydroid

This development smoke test verifies the complete interactive path:

```text
physical keyboard evdev
  -> W/A/S/D normalization
  -> persistent VirtualJoystick
  -> TouchEngine
  -> Type-B uinput touchscreen
  -> temporary Waydroid LXC bridge
  -> Android InputReader
```

The binary owns the complete temporary session lifecycle. It creates the virtual
touchscreen, installs the bridge, starts Waydroid as the desktop user recorded by
`sudo`, waits until Android sees the device, optionally opens the full UI, and
then reads the physical keyboard until Esc is pressed.

Held directions are game-compatible by default: while any W/A/S/D direction is
active, the tool reaffirms the current joystick position with a periodic
`Move` every 50 ms and prints a hold heartbeat every second. This makes a held
keyboard key visible both in Android `getevent` and in the terminal without
allocating a new tracking ID.

## Build

```bash
cargo build --locked --release \
  -p wroid-inject \
  --bin wroid-waydroid-keyboard-smoke
```

## Prepare

Use the physical keyboard event node verified with `evtest`. On the current test
system it is:

```text
/dev/input/event7
```

Stop Waydroid before changing the temporary LXC mount configuration:

```bash
waydroid session stop 2>/dev/null || true
sudo waydroid container stop 2>/dev/null || true
waydroid status
```

The expected status is `Session: STOPPED`. Some Waydroid versions omit a
`Container:` line when stopped; this is supported.

## Run in one terminal

This test normally uses **one terminal**. Do not start Waydroid or `evtest`
separately. Start without exclusive keyboard grab first:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 \
  1920 \
  1050
```

The tool performs these steps automatically:

1. validates that the selected keyboard reports W, A, S, D, and Esc;
2. creates `Wroid Gaming Touchscreen`;
3. installs the temporary LXC bridge;
4. starts the Waydroid user session;
5. waits until Android lists the virtual touchscreen;
6. opens the Waydroid full UI;
7. starts Android `getevent -lt` tracing for the virtual touchscreen;
8. maps W/A/S/D onto one persistent Android touch contact;
9. reaffirms a held direction periodically until release;
10. releases the contact, stops Waydroid, and removes the bridge after Esc.

Expected readiness output:

```text
Android detected Wroid Gaming Touchscreen.
Opened the Waydroid full UI.
Android getevent tracing is active.
Controls are live: W/A/S/D move one persistent Android touch contact; Esc exits. Exclusive grab: disabled. Reaffirm: 50ms. Hold log: 1000ms.
```

Press and hold W/A/S/D. Android trace lines should show one tracking ID while the
direction changes. During a long hold, Android should continue receiving
position `Move` samples at the reaffirm interval, and the terminal should print a
heartbeat similar to:

```text
holding up=true left=false down=false right=false for 1000ms
holding up=true left=false down=false right=false for 2000ms
```

Releasing the final direction should emit `BTN_TOUCH 0` and
`ABS_MT_TRACKING_ID -1`.

Press **Esc** to finish. Wait for both final messages:

```text
Keyboard capture stopped and the persistent contact was released.
Waydroid stopped and the temporary LXC bridge was removed.
```

## Exclusive grab

After the non-exclusive run succeeds, repeat with:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 \
  1920 \
  1050 \
  --grab
```

`--grab` applies `EVIOCGRAB` only after Android and the optional UI are ready.
The selected physical keyboard stops delivering events to the compositor and
other applications until Esc exits or the process closes the device descriptor.

## Optional flags

Tune the Android reaffirm interval:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 1920 1050 --reaffirm-ms 33
```

Disable reaffirming for comparison with the minimal event path:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 1920 1050 --no-reaffirm
```

Tune or disable terminal hold heartbeat logs:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 1920 1050 --hold-log-ms 500
```

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 1920 1050 --no-hold-log
```

Disable UI launch while retaining Android event tracing:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 1920 1050 --no-ui
```

Disable Android trace output while retaining live control:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke \
  /dev/input/event7 1920 1050 --no-trace
```

## Recovery

Use Esc for normal shutdown. `Ctrl+C`, `SIGKILL`, power loss, and kernel failure
can terminate the process before Rust destructors complete. Remove a stale
managed include explicitly before the next run:

```bash
sudo ./target/release/wroid-waydroid-keyboard-smoke --cleanup
```

Then verify:

```bash
sudo grep -n 'config_wroid_input' /var/lib/waydroid/lxc/waydroid/config \
  || echo 'Temporary Wroid bridge removed successfully'
```

The cleanup operation removes only the Wroid-managed include and bridge file. It
does not modify Android images, application data, or unrelated Waydroid config.
