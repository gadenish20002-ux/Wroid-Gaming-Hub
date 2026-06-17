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
starts the container, verifies that Android lists `Wroid Gaming Touchscreen`,
and captures injected touch data with Android `getevent`. It then stops the
container and restores the original config.

The bridge grants the container access only to the dynamically created Wroid
event node. It does not bind the host `/dev/input` directory and does not expose
physical keyboards or mice.

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
sudo waydroid container stop
waydroid status
```

`waydroid status` should report the container as stopped.

## Run the end-to-end test

Use the detected Waydroid screen size. For the current test system:

```bash
sudo ./target/release/wroid-waydroid-input-smoke 1920 1050
```

Expected result:

```text
Created Wroid Gaming Touchscreen at /dev/input/eventN
Installed a temporary, reversible Waydroid LXC input bridge.
Android getevent capabilities include Wroid Gaming Touchscreen.
Captured Android input events:
...
Waydroid detected the virtual touchscreen and Android getevent received touch data.
The container was stopped and the temporary LXC bridge was removed.
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
grep -n 'config_wroid_input' /var/lib/waydroid/lxc/waydroid/config || true
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
