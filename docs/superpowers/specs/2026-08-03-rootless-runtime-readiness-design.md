# Rootless Runtime Readiness Design

## Problem

The production game worker now runs as the desktop user, but its startup path
still calls legacy diagnostics implemented with `waydroid shell`. Waydroid
requires root for that action, so a valid root-owned bridge helper can install
the touchscreen and start Android while the worker subsequently fails before
controls become live.

## Decision

Keep Android boot and render-size checks unprivileged. The desktop worker uses
Waydroid's user D-Bus API (`waydroid prop get`) and the existing fresh-log
readiness cursor. It must not call `waydroid shell` in the production path.

Extend the already-running bridge helper with one fixed protocol request that
waits until Android `getevent -pl` lists `Wroid Gaming Touchscreen`. The request
contains no path, command, property, or caller-controlled argument. The helper
runs only the absolute `/usr/bin/waydroid` executable with a cleared environment
and fixed arguments.

The alternatives are rejected:

- Trusting only the generated LXC configuration cannot prove Android InputReader
  discovered the mounted device.
- Exposing arbitrary or caller-parameterized Waydroid shell commands would turn
  the minimal setuid helper into a general privileged execution surface.

## Protocol and lifecycle

After bridge installation the helper emits its existing ready line. The worker
may then write exactly `VERIFY_ANDROID_INPUT\n`. The helper performs a bounded
fixed-device probe and replies with exactly `WROID_ANDROID_INPUT_READY 1\n`.
After that, the only accepted request is the existing `CLEANUP\n` command.

EOF, malformed commands, failed probes, or worker death trigger the existing
forced Waydroid stop and bridge cleanup. A graceful cleanup remains valid only
after the worker has stopped Waydroid. The worker retains the helper stdout
reader for the full session rather than discarding it after the initial ready
line.

## Production startup sequence

1. Create the persistent uinput touchscreen and start the verified helper.
2. Start Waydroid as the desktop user and wait for its fresh user-ready log
   marker.
3. Configure the render properties through the user D-Bus API. Restart once
   only if values changed, then wait for a new user-ready marker.
4. Confirm the persisted render properties through the same rootless API.
5. Ask the helper to verify the fixed touchscreen inside Android.
6. Open the UI/package when requested and start evdev capture.
7. Stop Waydroid, send graceful cleanup, and preserve combined errors.

## Verification

Unit tests must prove that the helper protocol accepts only the two fixed
messages in order, malformed input cannot invoke a command, the privileged
Waydroid command remains absolute with a cleared environment, and production
startup delegates readiness to the desktop session/helper rather than legacy
root-only functions. The live acceptance test is the bounded
`launch-v2 --no-launch --no-ui --no-grab` session, followed by checks that
Waydroid is stopped and no managed bridge/process remains.
