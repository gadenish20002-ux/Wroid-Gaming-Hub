# Physical keyboard to persistent joystick smoke test

This milestone verifies the host-side path:

```text
physical keyboard evdev
  -> W/A/S/D normalization
  -> persistent VirtualJoystick
  -> TouchEngine
  -> Type-B uinput touchscreen
  -> host /dev/input/eventN
```

It does not start Waydroid or modify LXC configuration. The next integration
step will run the same captured keyboard state through the production session
lifecycle.

## Safety model

The keyboard event node is selected explicitly. Exclusive `EVIOCGRAB` is opt-in
through `--grab`; without it, the compositor and terminal continue receiving the
same key events. When enabled, the selected keyboard stops delivering events to
other clients until the process exits or releases the device.

Use a dedicated keyboard event node, not a mouse-button or consumer-control node.
The tool validates that the device reports W, A, S, D, and Esc before starting.
Esc always requests a controlled shutdown.

## Find the keyboard event node

List stable input symlinks:

```bash
ls -l /dev/input/by-id/
```

Or inspect devices interactively:

```bash
sudo evtest
```

Many gaming keyboards expose several event nodes. Select the node whose
capabilities include normal letter keys.

## Build

```bash
cargo build --locked --release \
  -p wroid-inject \
  --bin wroid-keyboard-joystick-smoke
```

## Run with two terminals

This host smoke test uses two terminals.

### Terminal 1

Start without exclusive grab first:

```bash
sudo ./target/release/wroid-keyboard-joystick-smoke \
  /dev/input/event7 1920 1050
```

The tool prints the temporary virtual touchscreen node, for example:

```text
Virtual touchscreen: /dev/input/event30
```

### Terminal 2

Attach `evtest` to the printed virtual touchscreen node:

```bash
sudo evtest /dev/input/event30
```

### Terminal 1 again

Press and hold W, A, S, and D. Expected behavior:

- first non-neutral direction emits touch `Down`;
- direction changes emit `Move` on the same tracking ID;
- releasing the final movement key emits `Up`;
- key auto-repeat does not create duplicate touch frames;
- Esc releases any active contact and exits.

After the non-exclusive test succeeds, repeat with `--grab`:

```bash
sudo ./target/release/wroid-keyboard-joystick-smoke \
  /dev/input/event7 1920 1050 --grab
```

With exclusive grab enabled, use Esc to exit. Closing the process also closes the
input file descriptor, which releases the kernel grab.

## Expected virtual touchscreen events

A direction press should include:

```text
BTN_TOUCH 1
ABS_MT_TRACKING_ID <positive id>
ABS_MT_POSITION_X <coordinate>
ABS_MT_POSITION_Y <coordinate>
SYN_REPORT
```

Changing direction should update coordinates without allocating a new tracking
ID. Releasing all movement keys should include:

```text
BTN_TOUCH 0
ABS_MT_TRACKING_ID -1
SYN_REPORT
```

## Troubleshooting

Permission errors require root for this development smoke tool or a narrow local
udev/logind policy. Do not make `/dev/input/event*` or `/dev/uinput`
world-writable.

If the tool reports missing W/A/S/D/Esc capabilities, choose another event node
for the same physical keyboard.
