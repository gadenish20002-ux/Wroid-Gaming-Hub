# Wroid Gaming Hub

Wroid Gaming Hub is a Linux gaming frontend for Waydroid. It combines a local
desktop launcher, a visual controls editor, and a low-latency evdev/uinput
runtime for profile-driven Android gaming.

## Workspace

- `wroid-core`: profile schema, JSON loading/saving, validation
- `wroid-adb`: thin ADB command wrapper
- `wroid-waydroid`: thin Waydroid command wrapper
- `wroid-cli`: `wroid` command-line interface

## Current Capabilities

- Validate, create, edit, scale, import, export, duplicate, rename, and remove JSON profiles.
- Run tap, swipe, keyevent, and virtual joystick actions through ADB or Waydroid shell input.
- Launch Android packages from profiles with managed background game sessions.
- List, launch, install APKs, and inspect current Android apps.
- Detect device screen size and density for profile creation and scaling.
- Diagnose ADB/Waydroid state with backend recommendations.
- Inspect the active GPU, DRM drivers, Waydroid EGL/Vulkan stack, compositor,
  resolution, refresh rate, and host-driven frame pacing with a
  launch-blocking software-renderer check.
- Request optional Feral GameMode host optimizations for normal Hub game
  sessions, with a persisted Auto/Off control and a direct-launch fallback.
- Inspect Android ABI, native ARM translation, GAPPS/Play Store availability,
  installed packages, and readiness for the four shipped games.
- Inspect the filesystem backing Waydroid game data and warn before the
  complete starter deck runs out of installation or resource-update space.
- Run profile v2 controls through evdev and a persistent multitouch uinput device with `play-v2`.
- Keep steady-state relative mouse-aim dispatch allocation-free after session
  preparation; inline touch frames and in-place state commit avoid profile,
  frame, and contact-map clones per MOVE.
- Preserve the sub-pixel remainder of scaled mouse motion, so a sensitivity or
  ADS multiplier below 1.0 keeps slow aim tracking proportional.
- Wait on the evdev descriptor rather than a fixed timer, removing the
  per-event reader poll delay from keyboard and mouse capture.
- Measure the injection hot path without root or Waydroid using
  `wroid-inject-latency`, which reports per-frame p50/p95/p99/max against the
  5 ms budget and verifies ten simultaneous contacts.
- Use shipped starter profiles for Brawl Stars, Standoff 2, PUBG Mobile, and
  Free Fire. FPS starters include mouse fire/ADS, reload, movement, camera aim,
  and editable game-specific actions.
- Open an unprivileged gaming hub with a persistent per-user profile library,
  game details, performance presets, hardware diagnostics, Play Store access,
  one-click controls editing, and managed background game launch.
Selecting an uninstalled game opens its exact Google Play listing instead of
leaving the user at the store home screen.

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

Install the optimized build for the current desktop user:

```sh
cargo build --release --workspace
target/release/wroid desktop install
wroid helper install
```

This installs `wroid` in `~/.local/bin`, adds **Wroid Gaming Hub** to the Linux
application menu, installs its scalable icon, and stages the standalone
`wroid-helper` as a read-only mode-`0555` release. The Hub's setup button opens
the desktop Polkit authorization dialog without a terminal. A detached installer
holds the exact helper bytes in a write-sealed Linux `memfd`, then asks the fixed
root `/usr/bin/install` command to publish that inode at
`/usr/lib/wroid/wroid-helper` as `root:input` mode `4750`. Closing the Hub cannot
truncate or replace the authorized source. `wroid helper install` uses the same
graphical path in a desktop session and retains a visible `sudo` fallback for
headless consoles. Subsequent game launches need no password. Readiness checks
verify ownership, exact permissions, effective-root `--check`, and release
contents before production play.
Inspect the installations without deleting profiles:

```sh
wroid desktop status
wroid helper status
wroid desktop uninstall
```

The development binary is `target/debug/wroid`. During development,
`cargo run -p wroid-cli -- <command>` is equivalent.

Always benchmark and play with the release build. The release profile enables
fat LTO, a single codegen unit, and `panic = "abort"` so the reader, runtime,
and injector crates inline into one gameplay hot path; the debug build shows a
tail an order of magnitude worse.

### Injection latency benchmark

```sh
cargo build --release --bin wroid-inject-latency
target/release/wroid-inject-latency --samples 20000
```

This needs no root, no Waydroid session, and no device grab: it creates the
same virtual touchscreen a production session uses, walks one contact across
it, and reports per-frame mean/p50/p95/p99/max against the 5 ms budget. It then
holds all ten advertised slots at once and releases them, which fails loudly if
the kernel does not accept the full contact count. Baseline on the RX 6600 XT /
7.1.5-cachyos / KDE Wayland development host is p99 ≈ 1 µs over 20 000 frames.

## Gaming Hub

After installation, open **Wroid Gaming Hub** from the application menu or run:

```sh
wroid hub
```

On first launch, Wroid installs editable copies of the four starter profiles in
`~/.config/wroid/profiles-v2`. The hub detects Waydroid, keyboard, mouse, and
installed Android packages; lets you select the exact keyboard and mouse,
choose a 720p, 900p, or 1080p session resolution, open Controls Studio, and
import additional Profile V2 JSON files. Device choices persist locally and
are revalidated against `/dev/input/by-id` before every launch. Hub and
Controls Studio share these choices and the GameMode Auto/Off preference
through the atomically written private file
`~/.config/wroid/preferences.json`, so random localhost ports and browser
restarts do not reset the selected devices or session target.
Auto uses a protected system `gamemoderun` when installed; otherwise the game
starts normally. The daemon clears loader override variables and never accepts
a wrapper path from the Hub.
If Waydroid contains a verified PUBG regional edition, BGMI, or Free Fire MAX,
Hub clones the current controls from the matching canonical starter into a
new exact-package profile. This is atomic and no-overwrite: existing profiles
and user edits always win. Each edition keeps its own calibration reference.
The selected preset is also persisted as Waydroid's Android render size. Wroid
verifies the saved width and height, restarts Android once only when they
change, and confirms the live `wm size` before input capture or game launch.
When Android is stopped, **Start Waydroid & scan**, **Play Store**, and
**Open Waydroid UI** start the normal desktop session without sudo, wait for
the Android package manager, and then refresh installed-game status. These
desktop actions refuse to interfere with an active Wroid game session.
For each installed game, the Hub also reports whether a calibration reference
has been saved beside its profile. **Open & calibrate** starts the package in
the normal desktop Waydroid session and opens Controls Studio in one action;
returning to the Hub refreshes the map status automatically.
The compatibility deck also reports free space on Waydroid's real host data
volume. Less than 40 GiB is flagged for the complete four-game library, and
less than 8 GiB is marked critical before large packages or resource updates
can fail silently.

**Run input self-test** opens the selected production control map without
launching its Android package. After the one-time helper setup, focus Waydroid and
exercise WASD, mouse aim, and mapped buttons; the trace and latency report are
printed in the terminal. The diagnostic stops after 20 seconds of live input,
then restores the prior Waydroid state and bridge automatically. This permits
end-to-end validation before any game or Google account is installed.
When the browser regains focus after Play Store, Controls Studio, a game
session, or Waydroid, Hub refreshes package, profile, and lease state
automatically. Concurrent focus events share one request, and there is no
periodic background polling during gameplay.

Starting a game runs a managed session in the background; no terminal is
opened. The per-user `wroidd` process owns and reaps the `launch-v2` worker, so
closing the Hub does not orphan process supervision. The Hub shows the active
PID/profile and turns its primary action into **Stop game**. That action first
signals the exact child owned by `wroidd`; direct and pre-upgrade launches fall
back to Linux pidfd validation using the recorded UID, process start time,
executable, and typed command line. `Ctrl+Esc` remains the in-game stop shortcut. Session
output is kept privately in `~/.local/state/wroid/game-session.log`. When the
game ends, `launch-v2` atomically writes a bounded mode-`0600` result to
`~/.local/state/wroid/last-game-session.json`. The selected game shows a clean,
stopped, or failed status in its Hero; failures expand to the recorded reason,
including hard-crash state captured by the daemon process reaper. Clean
sessions expose a compact performance readout with input/kernel p95 latency,
submitted touch frames, and peak simultaneous contacts; input p95 above the
5 ms target is highlighted.

Wroid stops the current desktop Waydroid session and keeps input capture,
mapping, uinput, telemetry, and package lifecycle in the desktop-user process.
The installed root-owned, release-matched helper validates and mounts the Wroid
virtual touchscreen without another password prompt. Wroid restores the bridge
configuration and previous desktop Waydroid session when play ends. A detached
per-launch watchdog performs the same restoration if the game process crashes.
If Waydroid was already stopped, Wroid leaves it stopped. The Hub and Controls
Studio never run as root.

The same safe launch workflow is available without the UI:

```sh
target/release/wroid launch-v2 ~/.config/wroid/profiles-v2/pubg-mobile.json
```

Inspect the graphics path directly (or consume its JSON in diagnostics tooling):

```sh
wroid performance
wroid performance --json
wroid performance --setup-gpu
wroid compatibility
wroid compatibility --json
wroid compatibility --setup
```

`launch-v2` and the Hub rerun this preflight before starting an installed game.
An identified CPU/software renderer blocks launch; incomplete probes are shown
as warnings so missing optional diagnostic utilities do not prevent play.
On multi-GPU systems, the preflight also compares Waydroid's
`gralloc.gbm.device` with the GPU used by the desktop renderer. The setup action
opens a visible terminal for system authorization, stores Waydroid's supported
`drm_device` setting atomically, keeps a `.wroid-backup`, runs the offline
config upgrade, and rolls back on failure. If desktop Waydroid is running,
Wroid stops it before regeneration, restores it afterward even when
authorization is cancelled, and verifies the live DRM-property readback before
reporting success.
While Waydroid is running, the report exposes its compositor-driven refresh
target and `wp_presentation` feedback state. Wroid does not write undocumented
FPS properties: Waydroid's hardware composer follows the active Wayland output
mode automatically, and disabled presentation feedback is reported as a
frame-pacing warning.
For known games, launch also stops before Waydroid teardown when the package is
known to be absent. On x86_64, the compatibility report highlights a confirmed
missing native bridge before opening Google Play, because ARM-only APKs cannot
run without an ARM translation component. Offline probes reuse saved Waydroid
properties and keep unavailable evidence explicitly unknown.

See [Game compatibility setup](docs/game-compatibility.md) before installing the
four starter games.

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

## Low-latency game session (`play-v2`)

Build the release binary, then use `launch-v2` for the complete stop/elevate/run
workflow. Google Play login and game installation stay under the user's control.

```sh
cargo build --release --workspace
target/release/wroid launch-v2 profiles/examples/brawlstars-v2.json
target/release/wroid launch-v2 profiles/examples/pubg-v2.json \
  --no-launch --trace-input --exit-after-seconds 20
```

Before launching, open the visual controls editor as the normal desktop user:

```sh
target/release/wroid profile edit-v2 profiles/examples/brawlstars-v2.json
```

Controls Studio opens locally in the default browser. Load or drop a game
screenshot, or use **Live align** and select the running Waydroid game window.
The live surface remains beneath the editable map; zoom and horizontal/vertical
crop controls remove window borders or letterboxing before **Save aligned
frame** stores an aspect-correct calibration image. Drag
tap/hold/joystick/mouse-aim controls over the matching HUD elements, adjust keys
and sensitivity in the inspector, then enable **Test inputs**. The inspector
only offers input sources that the production runtime can execute. Keyboard keys,
mouse buttons, and relative mouse movement highlight every matching control
without sending events to Android. Use **Save & Close** when the preview is
correct. Calibration images are kept in a local `.wroid-assets` directory
beside the profile and reopen automatically. The editor binds only to
`127.0.0.1`, uses a per-session token, validates the profile, and atomically
writes both profile data and backgrounds. Each changed save retains the
previous valid map in `.wroid-assets`; **Previous save** loads it as an unsaved
revision that can be reviewed, saved, or discarded with Undo. Repeated saves
without changes do not overwrite that recovery point. The editor never needs
root.
Key, directional-cluster, and mouse-aim toggle cells capture the next physical
key directly, including arrows, Tab, Escape, and modifiers. Backspace clears an
optional aim toggle. Unsupported keys are rejected before they can enter the
profile, and F12 remains reserved for releasing captured devices during play.
Use the layer rail to add Base overrides or named Hold/Toggle maps, select the
layer before placing controls, and capture an optional modifier in the binding
inspector. The list, overlay, validation, and **Test inputs** preview follow the
selected layer. For example, `G` can expose grenade slots while `Shift+R`
selects a separate fire-mode action:

```json
{
  "schema_version": 2,
  "name": "Standoff 2 — layered",
  "package_name": "com.axlebolt.standoff2",
  "orientation": "landscape",
  "layers": [
    { "name": "grenades", "activation": { "kind": "hold", "key": "g" } }
  ],
  "bindings": [
    {
      "name": "primary_weapon",
      "input": { "kind": "key", "key": "1" },
      "action": { "kind": "tap", "point": { "x": 0.89, "y": 0.18 } }
    },
    {
      "name": "frag",
      "layer": "grenades",
      "input": { "kind": "key", "key": "1" },
      "action": { "kind": "tap", "point": { "x": 0.70, "y": 0.30 } }
    },
    {
      "name": "fire_mode",
      "modifier": "shift",
      "input": { "kind": "key", "key": "r" },
      "action": { "kind": "tap", "point": { "x": 0.93, "y": 0.60 } }
    }
  ]
}
```

Use **Run map in Waydroid** to save the current map and open the same guarded
game session used by the Hub, so bindings can be verified in the installed game
before returning to the editor. Its **Session target** selector applies the same
720p, 900p, or 1080p render-size presets as the Hub.

Wroid auto-detects `/dev/input/by-id/*-event-kbd` and, when the profile needs
it, `*-event-mouse`. Select devices explicitly when necessary:

```sh
target/release/wroid play-v2 profiles/examples/standoff2-v2.json \
  --keyboard /dev/input/by-id/your-keyboard-event-kbd \
  --mouse /dev/input/by-id/your-mouse-event-mouse \
  --width 1920 --height 1080
```

`F12` releases or reacquires the captured keyboard and mouse, so desktop
shortcuts such as `Alt+Tab` remain available. `Ctrl+Esc` stops the session and
cancels every active touch; plain `Esc` remains available for profile bindings.
FPS profiles use `Tab` to enable/disable mouse aim.
Stopping from the Hub, pressing `Ctrl+Esc`, or terminating the session runs the
same contact, device-grab, bridge, and Waydroid cleanup.
Only one Wroid game session can own the Waydroid input bridge. Hub, Controls
Studio, CLI launches, diagnostics, and recovery all detect the active PID/owner
before touching Waydroid; the kernel releases this lease automatically if the
owner crashes. A second per-user launcher lease begins before Waydroid teardown,
so simultaneous launch clicks cannot race desktop-session restoration.
Background Hub launches also publish a private, crash-cleaned active-session
record under `XDG_RUNTIME_DIR`; only an identity-matched process can be stopped
from the Hub.
On exit, the session reports reader-to-inject and evdev kernel-to-inject
p50/p95/p99/max latency for batches that produced Android touch frames. The
bounded private last-session record carries these measurements into the Hub
without parsing console output.
On KDE Plasma 6, `launch-v2` also tracks the active KWin window: switching away
from Waydroid immediately cancels touches and releases the physical keyboard
and mouse, then safely reacquires them when Waydroid is focused again. The Hub
shows whether this automatic focus guard is available; other desktops use the
visible Ctrl+Esc/shutdown fallback.
The starter coordinates assume the default
landscape HUD; adjust normalized `x`/`y` values in the JSON after moving a
game's HUD controls. Available profiles:

- `brawlstars-v2.json`: WASD movement, arrows attack, Space super, E gadget.
- `standoff2-v2.json`: WASD, Tab mouse aim, LMB fire, R/Space/C/1/2/F.
- `pubg-v2.json`: WASD, Tab mouse aim, LMB/RMB, F/M/C/Z/Q/E/Space.
- `freefire-v2.json`: WASD, Tab mouse aim, LMB/RMB, R/F/Space/C/Z.

The managed input bridge needs one `sudo` authorization during helper
installation only. The separately installed typed helper runs as root while the
gameplay runtime remains unprivileged; launches themselves need no password.
`launch-v2` handles the stopped Waydroid precondition and restores
the LXC configuration, desktop session, and input grabs on normal exit. If
privileged bridge setup itself was interrupted, recover with:

```sh
sudo target/release/wroid-waydroid-game-session --cleanup
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
cargo run -p wroid-cli -- app inspect ./game.apk
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
cargo run -p wroid-cli -- app inspect ./game.xapk --json
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

`app inspect` reads bounded ZIP central-directory metadata without extracting or
executing the package. It distinguishes APK, XAPK, APKM, APKS, and OBB; reports
manifest/DEX/resources, embedded packages, encryption, and native libraries
under `lib/<abi>/*.so`; then compares those ABIs with Waydroid and its ARM
translation state.

`app install-apk` always runs that preflight before dispatch. Non-APK bundles,
encrypted entries, and malformed archives are rejected. A confirmed ABI
mismatch requires explicit `--force-incompatible`; unknown ABI evidence remains
advisory. `--allow-any-extension` changes only the filename check and never
bypasses content inspection.

The Hub exposes the same single-APK path without a terminal through **Sideload
APK**. It streams files up to 4 GiB into private per-user state instead of
buffering them in memory, shows format/native-ABI compatibility before an
explicit install action, and runs the Waydroid install in a detached ticketed
worker. Upload, status, discard, and worker state are localhost-token protected;
staged packages and bounded status files use private permissions and expire
after 24 hours.

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

## Per-user runtime daemon

`wroidd` is an unprivileged per-user process with protocol v2 over a
mode-`0600` Unix socket in `XDG_RUNTIME_DIR`. It validates peer UID through
`SO_PEERCRED`, bounds every JSON message, owns session state, and rejects a
second daemon through a kernel lease. Desktop installation stages the matching
daemon beside the other user runtime components.

`session prepare-v2` starts `wroidd` on demand, sends the profile through typed
IPC, and leaves the materialized control plan owned by the daemon. Normal Hub
launches use an atomic typed request: the daemon resolves the authenticated
client executable, constructs only fixed `launch-v2` arguments, owns the child
and private log, handles Stop, and reaps exit state. Direct `launch-v2` remains
available for diagnostics; input capture and Waydroid cleanup still run inside
that supervised desktop-user worker.

```sh
wroid daemon start
wroid daemon status
wroid session prepare-v2 profiles/examples/movement-v2.json --width 1920 --height 1080 --session-id shooter
wroid daemon sessions
```

A supported profile is retained in the daemon:

```text
Prepared session: shooter
State: Preparing
Package: com.example.movement
Launch package: true
Controls: 3
```

Unsupported actions such as `macro` fail clearly and name the offending binding.
Production `play-v2` sessions support tap, hold, virtual joystick, and relative
mouse aim controls.

```sh
wroid session prepare-v2 profiles/examples/shooter-v2.json --width 1920 --height 1080
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
