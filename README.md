# Wroid Gaming Hub

Wroid Gaming Hub is a Linux gaming frontend for Waydroid. It is currently a CLI-focused gaming layer that loads JSON control profiles and executes tap, swipe, and virtual joystick bindings through ADB or Waydroid shell input.

## Workspace

- `wroid-core`: profile schema, JSON loading/saving, validation
- `wroid-adb`: thin ADB command wrapper
- `wroid-waydroid`: thin Waydroid command wrapper
- `wroid-cli`: `wroid` command-line interface

## Current Capabilities

- Validate, create, edit, scale, import, export, duplicate, rename, and remove JSON profiles.
- Run tap, swipe, keyevent, and virtual joystick actions through ADB or Waydroid shell input.
- Launch Android packages from profiles and start the terminal keymapper.
- List, launch, install APKs, and inspect current Android apps.
- Detect device screen size and density for profile creation and scaling.
- Diagnose ADB/Waydroid state with backend recommendations.

## Tested Environment

The current local development environment is CachyOS / Arch-based Linux on Wayland with Waydroid available. On this system, `waydroid app launch` works as the normal desktop user, while `waydroid shell input ...` may require sudo. Waydroid can report `IP address: UNKNOWN`, which makes ADB unreliable even when the Waydroid session and container are running.

## Build and Install

Install Rust, ADB, and Waydroid with your distribution tooling. On Arch/CachyOS:

```sh
sudo pacman -S rust cargo adb waydroid
```

Build and test:

```sh
cargo build --workspace
cargo test --workspace
```

The CLI binary is `target/debug/wroid` after a debug build. During development, `cargo run -p wroid-cli -- <command>` is equivalent.

## Basic Workflow

```sh
cargo run -p wroid-cli -- doctor
sudo target/debug/wroid device info --backend waydroid-shell
sudo target/debug/wroid profile new-current /tmp/settings.json --name "Android Settings" --package com.android.settings --backend waydroid-shell --force
target/debug/wroid profile add-tap /tmp/settings.json --name home --key H --x 500 --y 900
target/debug/wroid profile add-joystick /tmp/settings.json --name movement --up W --left A --down S --right D --center 320,780 --radius 120
target/debug/wroid profile import /tmp/settings.json
wroid profile list
wroid profile show com.android.settings
target/debug/wroid app launch com.android.settings --backend waydroid-shell
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell --no-launch
```

For a new game profile, create a profile with the current Android surface size, add bindings, import it, then use `run-profile`. If the app is already launched or sudo app launch hangs, use `--no-launch`.

## More Docs

- [Architecture](docs/architecture.md)
- [Input model](docs/input-model.md)
- [Waydroid notes](docs/waydroid-notes.md)
- [Roadmap](docs/roadmap.md)

## Usage

```sh
cargo run -p wroid-cli -- doctor
cargo run -p wroid-cli -- doctor --backend waydroid-shell
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
cargo run -p wroid-cli -- profile add-joystick /tmp/wroid-profile.json --name movement --up W --left A --down S --right D --center 320,780 --radius 120
cargo run -p wroid-cli -- profile remove-binding /tmp/wroid-profile.json fire
cargo run -p wroid-cli -- profile import /tmp/settings.json
cargo run -p wroid-cli -- profile list
cargo run -p wroid-cli -- profile show com.android.settings
cargo run -p wroid-cli -- profile export com.android.settings /tmp/settings-export.json
cargo run -p wroid-cli -- profile duplicate com.android.settings com.android.settings-copy
cargo run -p wroid-cli -- profile rename com.android.settings-copy com.android.settings-backup
cargo run -p wroid-cli -- profile remove com.android.settings-backup
cargo run -p wroid-cli -- input tap 500 400
cargo run -p wroid-cli -- input swipe 400 500 800 500 180
cargo run -p wroid-cli -- input keyevent 3
cargo run -p wroid-cli -- app list --backend waydroid-shell
cargo run -p wroid-cli -- app launch com.android.settings --backend waydroid-shell
cargo run -p wroid-cli -- app install-apk ./game.apk --backend waydroid-shell
cargo run -p wroid-cli -- app install-apk ./downloaded-file.bin --backend waydroid-shell --allow-any-extension
cargo run -p wroid-cli -- app current --backend waydroid-shell
cargo run -p wroid-cli -- binding run profiles/examples/shooter-basic.json fire
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --scale-to-current
cargo run -p wroid-cli -- play profiles/examples/joystick-basic.json --scale-to-current
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
cargo run -p wroid-cli -- app install-apk ./downloaded-file.bin --backend waydroid-shell --allow-any-extension
cargo run -p wroid-cli -- app current --backend waydroid-shell
cargo run -p wroid-cli -- binding run profiles/examples/shooter-basic.json fire --backend auto
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --backend adb
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --backend waydroid-shell
cargo run -p wroid-cli -- play profiles/examples/shooter-basic.json --backend waydroid-shell --scale-to-current
cargo run -p wroid-cli -- play profiles/examples/joystick-basic.json --backend waydroid-shell --scale-to-current
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell --launch-delay-ms 2500
cargo run -p wroid-cli -- run profiles/examples/shooter-basic.json --backend waydroid-shell --no-launch
```

`doctor` reports ADB availability, ADB device states, Waydroid availability/status, screen and density probes for the selected backend, and a backend recommendation. If Waydroid reports `IP address: UNKNOWN` while the session/container are running, ADB may not connect; use `--backend waydroid-shell` for shell input on systems where Waydroid shell works.

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

For `virtual_joystick` bindings, `play` tracks the directional `key_cluster`, computes normalized diagonal movement, and repeatedly emits `input swipe center target duration` while a direction is held. Terminal input must remain focused; this is not global input capture yet. Key release tracking depends on terminal support for enhanced keyboard events, so terminals without release events may not provide reliable hold/release behavior.

```sh
sudo target/debug/wroid play profiles/examples/joystick-basic.json --backend waydroid-shell --scale-to-current
```

`run` loads the same profile, launches `package_name`, waits 1500 ms by default, then starts the same interactive keymapper used by `play`:

```sh
sudo target/debug/wroid run profiles/my-game.json --backend waydroid-shell
sudo target/debug/wroid run profiles/my-game.json --backend waydroid-shell --launch-delay-ms 2500
sudo target/debug/wroid run profiles/my-game.json --backend waydroid-shell --no-launch
```

With `--no-launch`, `run` and `run-profile` load and validate the profile, skip launching `package_name`, skip the launch delay, and start the interactive keymapper immediately.

For app management, `--backend waydroid-shell` uses `waydroid app list`, `waydroid app launch`, and `waydroid app install` where available. Current-app detection still uses `waydroid shell dumpsys activity activities` because Waydroid does not expose the focused Android activity through `waydroid app`.

`app install-apk` checks that the path exists, is a file, and ends in `.apk` before dispatching to the selected backend. It prints the resolved absolute path for install attempts. Pass `--allow-any-extension` only when the file is known to be an APK despite its name.

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

Supported action kinds are `tap`, `swipe`, and `virtual_joystick`. Virtual joystick bindings use a `key_cluster` input and store a center point, radius, tick interval, and swipe duration:

```json
{
  "name": "movement",
  "input": {
    "kind": "key_cluster",
    "up": "w",
    "left": "a",
    "down": "s",
    "right": "d"
  },
  "action": {
    "kind": "virtual_joystick",
    "center": { "x": 320, "y": 780 },
    "radius": 120,
    "tick_ms": 80,
    "swipe_duration_ms": 70
  }
}
```

`virtual_joystick` validates, lists, saves, scales, and runs in terminal `play` mode. `mouse_aim` and `macro` remain schema placeholders and intentionally fail normal validation until implemented.

Profiles can also be edited from the CLI:

```sh
cargo run -p wroid-cli -- profile new profiles/local/my-game.json --name "My Game" --package com.example.game --width 1920 --height 1080
cargo run -p wroid-cli -- profile add-tap profiles/local/my-game.json --name fire --key F --x 1640 --y 540
cargo run -p wroid-cli -- profile add-swipe profiles/local/my-game.json --name look_right --key D --from 960,540 --to 1260,540 --duration-ms 180
cargo run -p wroid-cli -- profile add-joystick profiles/local/my-game.json --name movement --up W --left A --down S --right D --center 320,780 --radius 120 --tick-ms 80 --swipe-duration-ms 70
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

Scaling also updates virtual joystick centers. Joystick radius is scaled with the average of the horizontal and vertical scale factors because radius is a single scalar while screens can change by different x/y ratios.

Profiles can be imported into the user-owned local registry at `$XDG_CONFIG_HOME/wroid/profiles/`, or `~/.config/wroid/profiles/` when `XDG_CONFIG_HOME` is not set. By default, the registry ID is the profile's `package_name`:

```sh
wroid profile import /tmp/settings.json
wroid profile list
wroid profile show com.android.settings
wroid profile export com.android.settings /tmp/settings-export.json
wroid profile duplicate com.android.settings com.android.settings-copy
wroid profile rename com.android.settings-copy com.android.settings-backup
wroid profile remove com.android.settings-backup
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell
sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell --no-launch
```

When `run-profile` is run via `sudo` for `--backend waydroid-shell`, Wroid resolves the registry against the original desktop user from `SUDO_USER` and `SUDO_UID`, so `sudo target/debug/wroid run-profile com.android.settings --backend waydroid-shell` still loads `/home/<user>/.config/wroid/profiles/com.android.settings.json` unless `XDG_CONFIG_HOME` is explicitly set.

## Development

```sh
cargo fmt
cargo test --workspace
```
