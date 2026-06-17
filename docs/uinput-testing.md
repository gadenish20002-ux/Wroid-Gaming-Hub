# uinput touchscreen testing

The `wroid-inject` crate creates one persistent Linux multitouch Type-B device.
It does not spawn `adb`, `waydroid shell`, or another process for each input
frame. The initial integration target is host-side protocol correctness and
latency; exposing the resulting event device inside Waydroid is the next gate.

## Prerequisites

- Linux with the `uinput` kernel module available.
- Permission to open `/dev/uinput`.
- `evtest` or `libinput debug-events` for host-side observation.
- A running Waydroid session for later container visibility checks.

Load the module when needed:

```bash
sudo modprobe uinput
ls -l /dev/uinput
```

Do not make `/dev/uinput` world-writable. During development, run only the
smoke tool with elevated privileges or use a narrowly scoped local udev rule.
The desktop UI and profile parser must remain unprivileged.

## Build and host smoke test

```bash
cargo build --locked --release -p wroid-inject --bin wroid-uinput-smoke
sudo ./target/release/wroid-uinput-smoke 1920 1050
```

The smoke tool keeps one virtual device open, waits for Enter, submits one
synchronized `down -> move -> up` sequence, and waits again before destroying
the device. `evdev::VirtualDevice::emit` appends one `SYN_REPORT` to each
submitted batch.

While the smoke tool is waiting, locate and inspect the device in another terminal:

```bash
sudo evtest
```

Select `Wroid Gaming Touchscreen`. The expected capabilities include:

- `INPUT_PROP_DIRECT`
- `BTN_TOUCH`
- `ABS_X` and `ABS_Y`
- `ABS_MT_SLOT` with ten slots by default
- `ABS_MT_TRACKING_ID`
- `ABS_MT_POSITION_X` and `ABS_MT_POSITION_Y`

## Waydroid visibility gate

The host device being correct does not prove Android can see it. With Waydroid
running, inspect Android input devices:

```bash
sudo waydroid shell getevent -pl
sudo waydroid shell dumpsys input
```

The current milestone is complete only for host-side injection. A later change
will lease or forward the event device into the Waydroid container and verify
that Android InputReader classifies it as a direct multitouch touchscreen.
