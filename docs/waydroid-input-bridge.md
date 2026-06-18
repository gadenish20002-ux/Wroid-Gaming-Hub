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

The binary restores the original config on normal success and ordinary errors.
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
configuration.

## Start the normal session again

```bash
waydroid session start
waydroid show-full-ui
```

The normal Waydroid session is started only after the smoke test has returned to
the shell and bridge cleanup has been verified.
