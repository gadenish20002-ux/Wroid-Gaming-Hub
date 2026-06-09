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
cargo run -p wroid-cli -- profile add-tap /tmp/wroid-profile.json --name fire --key F --x 1640 --y 540
cargo run -p wroid-cli -- profile add-swipe /tmp/wroid-profile.json --name look_right --key D --from 960,540 --to 1260,540 --duration-ms 180
cargo run -p wroid-cli -- profile remove-binding /tmp/wroid-profile.json fire
cargo run -p wroid-cli -- input tap 500 400
cargo run -p wroid-cli -- input swipe 400 500 800 500 180
cargo run -p wroid-cli -- input keyevent 3
cargo run -p wroid-cli -- app list --backend waydroid-shell
cargo run -p wroid-cli -- app launch com.android.settings --backend waydroid-shell
cargo run -p wroid-cli -- app install-apk ./game.apk --backend waydroid-shell
cargo run -p wroid-cli -- app current --backend waydroid-shell
cargo run -p wroid-cli -- binding run profiles/examples/shooter-basic.json fire
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell
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
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell --launch-delay-ms 2500
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
```

For app management, `--backend waydroid-shell` uses `waydroid app list`, `waydroid app launch`, and `waydroid app install` where available. Current-app detection still uses `waydroid shell dumpsys activity activities` because Waydroid does not expose the focused Android activity through `waydroid app`.

On some systems, `waydroid shell ...` operations require root privileges. This affects shell-backed input commands and `app current`; `waydroid app launch` may work without sudo. On those systems, `wroid run` handles app launch as the original user when `SUDO_USER` and `SUDO_UID` are available, restores that user's DBus and Wayland session environment for the launch subprocess, then keeps the keymapper in the current sudo process for shell input. If `--backend waydroid-shell` fails with `Action "shell" needs root access`, run the CLI itself with `sudo`, for example:

```sh
sudo target/debug/wroid input tap 500 400 --backend waydroid-shell
sudo target/debug/wroid input keyevent 3 --backend waydroid-shell
sudo target/debug/wroid input keyevent 4 --backend waydroid-shell
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

## Development

```sh
cargo fmt
cargo test --workspace
```
