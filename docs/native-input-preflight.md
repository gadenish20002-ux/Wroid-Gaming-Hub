# Native input preflight

The native input runner must fail before creating a uinput device or installing a
Waydroid bridge when Waydroid is already in a split state such as:

```text
Session:   RUNNING
Container: FROZEN
```

Required behavior:

1. Check `waydroid status` before creating the virtual touchscreen.
2. If `Container: RUNNING`, stop with the existing message asking the user to stop Waydroid first.
3. If `Session: RUNNING` and `Container: FROZEN`, stop with an actionable recovery message:

```bash
sudo target/debug/wroid-native-keyboard --cleanup
waydroid session stop
sudo systemctl restart waydroid-container
```

This protects the host from partial setup where the temporary LXC bridge is
installed and the virtual touchscreen is created before the runner discovers that
Waydroid cannot transition cleanly into `Container: RUNNING`.
