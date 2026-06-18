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

Run the binary with `sudo` from the active desktop user session. The tool uses
`SUDO_USER` and `SUDO_UID` to reconnect to that user's DBus and Wayland sockets;
do not launch it from a standalone root shell.

Use the detected Waydroid screen size. For the current test system:

```bash
sudo ./target/release/wroid-waydroid-input-smoke 1920 1050
```

Expected result:

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

## Recovery

The binary restores the original config on normal success and ordinary errors.
After an abrupt power loss or `SIGKILL`, remove the managed include explicitly:

```bash
sudo ./target/release/wroid-waydroid-input-smoke --cleanup
```

Then verify that no managed include remains:

```bash
sudo grep -n 'config_wroid_input' /var/lib/waydroid/lxc/waydroid/config || true
```

The cleanup command only removes the Wroid-managed include and file. It does not
modify Waydroid images, application data, Android properties, or unrelated LXC
configuration.

## Start the normal session again

```bash
waydroid session start
waydroid show-full-ui
```

This smoke test is an integration tool, not the final production privilege
model. The production runtime will move the same allow-listed operation behind
a minimal privileged helper while the GUI and profile engine remain
unprivileged.
