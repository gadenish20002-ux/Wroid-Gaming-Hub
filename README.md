# Wroid Gaming Hub

Wroid Gaming Hub is a Linux gaming frontend for Waydroid. MVP-0 is a CLI-only skeleton that loads JSON control profiles and executes tap/swipe bindings through ADB or Waydroid shell input.

## Workspace

- `wroid-core`: profile schema, JSON loading/saving, validation
- `wroid-adb`: thin ADB command wrapper
- `wroid-waydroid`: thin Waydroid command wrapper
- `wroid-cli`: `wroid` command-line interface

## Usage

```sh
cargo run -p wroid-cli -- doctor
cargo run -p wroid-cli -- profile validate profiles/examples/shooter-basic.json
cargo run -p wroid-cli -- profile list-bindings profiles/examples/shooter-basic.json
cargo run -p wroid-cli -- profile example /tmp/wroid-profile.json
cargo run -p wroid-cli -- profile new /tmp/wroid-profile.json --name "My Game" --package com.example.game --width 1920 --height 1080
cargo run -p wroid-cli -- device info --backend waydroid-shell
cargo run -p wroid-cli -- profile new-current /tmp/settings.json --name "Android Settings" --package com.android.settings --backend waydroid-shell --force
cargo run -p wroid-cli -- profile registry-new-current --name "Android Settings" --package com.android.settings --backend waydroid-shell --force
cargo run -p wroid-cli -- profile scale profiles/examples/shooter-basic.json /tmp/shooter-1050.json --width 1920 --height 1050 --force
cargo run -p wroid-cli -- profile add-tap /tmp/wroid-profile.json --name fire --key F --x 1640 --y 540
cargo run -p wroid-cli -- profile add-swipe /tmp/wroid-profile.json --name look_right --key D --from 960,540 --to 1260,540 --duration-ms 180
cargo run -p wroid-cli -- profile remove-binding /tmp/wroid-profile.json fire
cargo run -p wroid-cli -- profile import /tmp/settings.json
cargo run -p wroid-cli -- profile list
cargo run -p wroid-cli -- profile show com.android.settings
cargo run -p wroid-cli -- input tap 500 400
cargo run -p wroid-cli -- input swipe 400 500 800 500 180
cargo run -p wroid-cli -- input keyevent 3
cargo run -p wroid-cli -- app list --backend waydroid-shell
cargo run -p wroid-cli -- app launch com.android.settings --backend waydroid-shell
cargo run -p wroid-cli -- app install-apk ./game.apk --backend waydroid-shell
cargo run -p wroid-cli -- app current --backend waydroid-shell
cargo run -p wroid-cli -- binding run profiles/examples/shooter-basic.json fire
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --scale-to-current
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell --no-launch
cargo run -p wroid-cli -- run-profile com.android.settings --backend waydroid-shell
cargo run -p wroid-cli -- run-profile com.android.settings --backend waydroid-shell --no-launch
```

Input commands default to `--backend auto`. Auto uses ADB when `adb devices` reports at least one connected device with status `device`; otherwise it falls back to `waydroid shell input`.

```sh
cargo run -p wroid-cli -- input tap 500 400 --backend auto
cargo run -p wroid-cli -- input tap 500 400 --backend adb
cargo run -p wroid-cli -- input tap 500 400 --backend waydroid-shell
cargo run -p wroid-cli -- input swipe 400 500 800 500 180 --backend waydroid-shell
cargo run -p wroid-cli -- input keyevent 3 --backend waydroid-shell
cargo run -p wroid-cli -- app list --backend waydroid-shell
cargo run -p wroid-cli -- app launch com.android.settings --backend waydroid-shell
cargo run -p wroid-cli -- app install-apk ./game.apk --backend waydroid-shell
cargo run -p wroid-cli -- app current --backend waydroid-shell
cargo run -p wroid-cli -- binding run profiles/examples/shooter-basic.json fire --backend auto
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --backend adb
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --backend waydroid-shell
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --backend waydroid-shell --scale-to-current
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell --launch-delay-ms 2500
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell --no-launch
```

`play` loads and validates a profile, prints the profile metadata and keyboard bindings, then listens for key presses until `Esc` or `Ctrl+C`:

```text
Profile: Shooter Basic
Package: com.example.shooter
Backend: adb

Keyboard bindings:
  F -> fire
  R -> reload
  D -> look_right
```

`run` loads the same profile, launches `package_name`, waits 1500 ms by default, then starts the same interactive keymapper used by `play`:

```sh
sudo target/debug/wroid run profiles/my-game.json --backend waydroid-shell
sudo target/debug/wroid run profiles/my-game.json --backend waydroid-shell --launch-delay-ms 2500
sudo target/debug/wroid run profiles/my-game.json --backend waydroid-shell --no-launch
```

With `--no-launch`, `run` and `run-profile` load and validate the profile, skip launching `package_name`, skip the launch delay, and start the interactive keymapper immediately.

For app management, `--backend waydroid-shell` uses `waydroid app list`, `waydroid app launch`, and `waydroid app install` where available. Current-app detection still uses `waydroid shell dumpsys activity activities` because Waydroid does not expose the focused Android activity through `waydroid app`.

On some systems, `waydroid shell ...` operations require root privileges. This affects shell-backed input commands and `app current`; `waydroid app launch` may work without sudo. On those systems, `wroid run` handles app launch as the original user when `SUDO_USER` and `SUDO_UID` are available, restores that user's DBus and Wayland session environment for the launch subprocess, then keeps the keymapper in the current sudo process for shell input. If `--backend waydroid-shell` fails with `Action "shell" needs root access`, run the CLI itself with `sudo`, for example:

```sh
sudo target/debug/wroid input tap 500 400 --backend waydroid-shell
sudo target/debug/wroid input keyevent 3 --backend waydroid-shell
sudo target/debug/wroid input keyevent 4 --backend waydroid-shell
sudo target/debug/wroid device info --backend waydroid-shell
```

On some systems, launching apps through sudo user/session restoration may hang. In that case, launch the Android app as the normal user first, then start Wroid under sudo with `--no-launch`:

```sh
target/debug/wroid app launch com.android.settings --backend waydroid-shell
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell --no-launch
```

## Profile Format

Profiles are JSON files with a target resolution and named bindings:

```json
{
  "name": "Shooter Basic",
  "package_name": "com.example.shooter",
  "resolution": { "width": 1920, "height": 1080 },
  "bindings": [
    {
      "name": "fire",
      "input": { "kind": "key", "key": "f" },
      "action": { "kind": "tap", "point": { "x": 1640, "y": 540 } }
    }
  ]
}
```

Supported MVP action kinds are `tap` and `swipe`. `virtual_joystick`, `mouse_aim`, and `macro` exist in the schema as placeholders and intentionally fail normal validation until implemented. The interactive `play` runner tolerates those placeholder actions so it can print a clear unsupported-action message and continue running.

Profiles can also be edited from the CLI:

```sh
cargo run -p wroid-cli -- profile new profiles/local/my-game.json --name "My Game" --package com.example.game --width 1920 --height 1080
cargo run -p wroid-cli -- profile add-tap profiles/local/my-game.json --name fire --key F --x 1640 --y 540
cargo run -p wroid-cli -- profile add-swipe profiles/local/my-game.json --name look_right --key D --from 960,540 --to 1260,540 --duration-ms 180
cargo run -p wroid-cli -- profile list-bindings profiles/local/my-game.json
cargo run -p wroid-cli -- profile remove-binding profiles/local/my-game.json fire
```

To create a profile using the current Android surface resolution reported by `wm size`, use `new-current`. This avoids guessing host-window dimensions such as `1920x1080` when Waydroid reports a different Android surface such as `1920x1050`:

```sh
sudo target/debug/wroid device info --backend waydroid-shell
sudo target/debug/wroid profile new-current /tmp/settings.json --name "Android Settings" --package com.android.settings --backend waydroid-shell --force
sudo target/debug/wroid profile registry-new-current --name "Android Settings" --package com.android.settings --backend waydroid-shell --force
sudo target/debug/wroid profile scale-current profiles/examples/shooter-basic.json /tmp/shooter-current.json --backend waydroid-shell --force
```

Existing profiles can be scaled to another Android surface resolution without changing binding names or inputs:

```sh
wroid profile scale profiles/examples/shooter-basic.json /tmp/shooter-1050.json --width 1920 --height 1050 --force
sudo target/debug/wroid play /tmp/shooter-1050.json --backend waydroid-shell
sudo target/debug/wroid play profiles/examples/shooter-basic.json --backend waydroid-shell --scale-to-current
```

Profiles can be imported into the user-owned local registry at `$XDG_CONFIG_HOME/wroid/profiles/`, or `~/.config/wroid/profiles/` when `XDG_CONFIG_HOME` is not set. By default, the registry ID is the profile's `package_name`:

```sh
wroid profile import /tmp/settings.json
wroid profile list
wroid profile show com.android.settings
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell --no-launch
```

When `run-profile` is run via `sudo` for `--backend waydroid-shell`, Wroid resolves the registry against the original desktop user from `SUDO_USER` and `SUDO_UID`, so `sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell` still loads `/home/<user>/.config/wroid/profiles/com.android.settings.json` unless `XDG_CONFIG_HOME` is explicitly set.

## Development

```sh
cargo fmt
cargo test --workspace
```
