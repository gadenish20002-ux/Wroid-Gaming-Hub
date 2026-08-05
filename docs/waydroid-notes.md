# Waydroid Notes

Wroid treats Waydroid as an external dependency. It does not manage Waydroid installation or lifecycle beyond calling documented `waydroid` commands.

## Tested Local Environment

Current local development has been tested on:

- CachyOS / Arch-based Linux.
- Wayland session.
- Waydroid backend available.
- `waydroid app launch <package>` working as the normal desktop user.
- `waydroid shell input ...` working when run with the privileges required by the local Waydroid setup.

On this system, `waydroid status` can report:

```text
Session: RUNNING
Container: RUNNING
IP address: UNKNOWN
```

When the IP address is `UNKNOWN`, ADB may not connect even though Waydroid itself is running. The `waydroid-shell` backend can still work because it uses `waydroid shell input` instead of ADB networking.

## Recommended Arch/CachyOS Packages

Package names vary by repository setup, but a typical development environment needs:

```sh
sudo pacman -S rust cargo adb waydroid
```

Depending on the system, Waydroid itself may require distribution-specific setup, kernel modules, container service configuration, and a vendor image. Follow the distro Waydroid documentation for installation.

## Backend Choice

Use `wroid doctor` first:

```sh
cargo run -p wroid-cli -- doctor
cargo run -p wroid-cli -- doctor --backend waydroid-shell
```

Backend behavior:

- `adb`: uses `adb shell input ...`; requires a connected ADB device in state `device`.
- `waydroid-shell`: uses `waydroid shell input ...`; may require root privileges.
- `auto`: chooses `adb` when a connected ADB device exists, otherwise falls back to `waydroid-shell`.

## Sudo Workflow

On systems where `waydroid shell` requires root, shell-backed input commands may need `sudo`:

```sh
sudo target/debug/wroid input tap 500 400 --backend waydroid-shell
sudo target/debug/wroid device info --backend waydroid-shell
```

Launching an app through `waydroid app launch` often works as the normal desktop user. If launching through a sudo-restored user session hangs, use a split workflow:

```sh
target/debug/wroid app launch com.android.settings --backend waydroid-shell
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell --no-launch
```

`run-profile` resolves the profile registry against the original desktop user when `SUDO_USER` and `SUDO_UID` are present, so root shell input can still use the user's profile registry.

## Current Limitations

- ADB depends on Waydroid networking.
- `waydroid-shell` may require sudo.
- Current input capture is terminal-focused.
- There is no global input capture yet.
- XAPK/APKM/OBB installation is not implemented.
- `wroid app inspect` recognizes those containers and reports embedded APK/OBB
  files, but deliberately does not extract or partially install them.
