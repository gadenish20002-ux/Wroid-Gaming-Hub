# Waydroid input bridge smoke test

This milestone verifies the complete low-latency input path:

```text
Wroid touch frame
  -> persistent uinput device
  -> host /dev/input/eventN
  -> narrow LXC bind mount
  -> Android EventHub/InputReader
  -> Android getevent
```

The smoke binary temporarily adds one managed include to Waydroid's LXC config,
starts a normal Waydroid session as the desktop user recorded by `sudo`, verifies
that Android lists `Wroid Gaming Touchscreen`, and captures injected touch data
with Android `getevent`. It then stops the user session and container and restores
the original config.

Waydroid mounts a fresh tmpfs at container `/dev`, so the bridge first creates an
empty container-local `/dev/input` mountpoint and then bind-mounts only the
dynamically created Wroid event node. It does not bind the host `/dev/input`
directory and does not expose physical keyboards or mice.

The bridge deliberately does not add `lxc.cgroup*.devices.allow` or
`lxc.cgroup*.devices.deny` rules. On cgroup v2, a specific allow rule creates an
allowlist program that blocks every device not listed, including Binder devices
required to boot Android. The smoke bridge preserves Waydroid's existing device
policy and narrows visibility through the mount namespace instead.

All privileged bridge workflows share one non-blocking kernel lease at
`/run/wroid/input-bridge.lock`. A second game, smoke test, benchmark, or recovery
command exits before stopping Waydroid or modifying LXC configuration and names
the current PID/owner. The kernel releases the lease even if its process crashes;
the lock file is only readable owner metadata, not the source of lock truth.
Desktop launches also hold a per-user lease at
`$XDG_RUNTIME_DIR/wroid/game-launch.lock` before inspecting or stopping the
desktop Waydroid session. This closes the earlier pre-sudo race where two
launchers could both plan restoration before the privileged bridge owner
existed. Hub and Controls Studio probe both leases before opening a second game
terminal.

## Production daemon ownership

Normal Hub launches, foreground `launch-v2`, and the bounded input self-test use
the same daemon-owned helper lifecycle. `wroidd` validates that the helper
staged beside its own executable is a protected current-user file and is
byte-identical to `/usr/lib/wroid/wroid-helper`. It then creates a private Unix
socket pair, starts the helper from a daemon-owned broker, and gives the worker
only fixed inherited descriptor 198.

The internal protocol is bounded, versioned, and ordered:
`open(eventN) -> verify_android_input -> finish`. The worker still creates the
virtual touchscreen and runs physical-input/profile dispatch, but it cannot
choose a helper executable or reuse bridge authority outside that one session.
Worker exit, Stop, daemon shutdown, protocol failure, and helper failure all
close the broker and converge on helper cleanup. A daemon release with an
active worker is not replaced. An idle stale daemon is bound to a pidfd at
socket authentication, frozen and rechecked for worker children, then
terminated; a detached watchdog resumes it if the upgrader disappears.

## Build

```bash
cargo build --locked --release \
  -p wroid-inject \
  --bin wroid-waydroid-input-smoke
```

## Prepare Waydroid

The container must be stopped before its LXC mount configuration can change:

```bash
waydroid session stop
waydroid status
```

If the container remains running after the user session stops:

```bash
sudo waydroid container stop
```

`waydroid status` should report the container as stopped.

## Run the end-to-end test

This test uses **one terminal only**. Run every command below sequentially in the
same terminal. Do not start `evtest`, do not start Waydroid manually in another
terminal, and do not press `Ctrl+C` while the smoke binary is running.

Run the binary with `sudo` from the active desktop user session. The tool uses
`SUDO_USER` and `SUDO_UID` to reconnect to that user's DBus and Wayland sockets;
do not launch it from a standalone root shell.

Use the Android surface size. For the current test system:

```bash
sudo ./target/release/wroid-waydroid-input-smoke 1920 1050
```

The binary performs the complete sequence itself:

1. creates `Wroid Gaming Touchscreen`;
2. installs the temporary LXC bridge;
3. starts the Waydroid session;
4. waits until Android sees the device;
5. captures a bounded number of Android `getevent` records;
6. injects one `down -> move -> up` sequence;
7. stops the Waydroid session and container;
8. removes the temporary bridge.

The production game-session workflow additionally applies the selected Android
render width and height after boot. The desktop worker waits for a fresh
user-ready event captured from its owned `waydroid session start` process,
forwards that process's stdout/stderr, verifies property readback through the
user D-Bus API, and restarts Waydroid once only when the size changed. The
installed helper then accepts one fixed, argument-free request to confirm that Android
`getevent -pl` lists `Wroid Gaming Touchscreen`; it does not expose a general
root shell to the worker.

Wait until the shell prompt returns. Expected result:

```text
Created Wroid Gaming Touchscreen at /dev/input/eventN
Installed a temporary, reversible Waydroid LXC input bridge.
Starting Waydroid session as desktop user USER on wayland-N...
Waydroid container is RUNNING.
Android getevent capabilities include Wroid Gaming Touchscreen.
Captured Android input events:
...
Waydroid detected the virtual touchscreen and Android getevent received touch data.
The user session and container were stopped, and the temporary LXC bridge was removed.
```

The tool uses the argument separator required when a shell command has options:

```bash
sudo waydroid shell -- getevent -pl
```

Without `--`, Waydroid's own argument parser may treat `-pl` as an option to the
`waydroid shell` command instead of passing it to Android `getevent`.

## Host-only smoke test uses two terminals

The separate `wroid-uinput-smoke` host test is the only test in this milestone
that requires two terminals:

- terminal 1 keeps `wroid-uinput-smoke` running;
- terminal 2 attaches `evtest` to the temporary event node;
- terminal 1 sends the sequence after `evtest` is attached.

Do not apply that two-terminal workflow to `wroid-waydroid-input-smoke`; the
Waydroid test captures Android events internally.

## Recovery

Bridge installation is transactional: if the managed input config is written
but the main LXC include cannot be updated, Wroid restores the previous managed
file before returning an error. Normal cleanup removes only Wroid's include and
preserves unrelated LXC configuration changes made during the session.

The binary restores the managed state on normal success and ordinary errors.
If gameplay and one or more cleanup stages fail together, the final error
reports the runtime, Waydroid shutdown, and bridge cleanup failures instead of
hiding the recovery problem.
After `Ctrl+C`, abrupt power loss, or `SIGKILL`, remove the managed include
explicitly:

```bash
sudo ./target/release/wroid-waydroid-input-smoke --cleanup
```

Then verify that no managed include remains:

```bash
sudo grep -n 'config_wroid_input' /var/lib/waydroid/lxc/waydroid/config \
  || echo 'Wroid bridge is clean'
```

The cleanup command only removes the Wroid-managed include and file. It does not
modify Waydroid images, application data, Android properties, or unrelated LXC
configuration. Recovery refuses to run while a live Wroid session owns the
bridge, preventing it from dismantling an active game's input path.

## Start the normal session again

```bash
waydroid session start
waydroid show-full-ui
```

The normal Waydroid session is started only after the smoke test has returned to
the shell and bridge cleanup has been verified.
