# Wroid Gaming Hub Architecture v2

## Product boundary

Wroid Gaming Hub is a gaming-oriented Android environment for Linux built on a
containerized Android system. Waydroid provides the Android container and direct
hardware integration. Wroid owns the gaming runtime, low-latency input,
per-game configuration, lifecycle management, diagnostics, and user interface.

The project is not a full-machine emulator and must not introduce a video
capture/encode/decode path for normal local play.

## Non-negotiable invariants

1. No subprocess creation in the gaming input hot path.
2. The GUI never runs as root.
3. Privileged operations are exposed through a minimal typed API, never an
   arbitrary shell command.
4. Touch state is stateful and multitouch-capable: down, move, up, and cancel.
5. Runtime state changes only after the injection backend accepts a frame.
6. Every shutdown, focus-loss, and backend-failure path releases active contacts.
7. A software renderer is detected and reported as a blocking performance issue.
8. A compatibility backend may be slow, but it must be clearly labelled and must
   not be selected silently for gaming mode.

## Component model

```text
wroid-ui (unprivileged)
    |
    | typed IPC
    v
wroidd (per-user runtime daemon)
    |-- game/session state machine
    |-- profile evaluation
    |-- viewport transforms
    |-- input capture coordination
    |-- telemetry
    |
    +--> wroid-helper (minimal privileged service)
    |       |-- evdev access and grabs
    |       |-- uinput device creation
    |       `-- restricted Waydroid system operations
    |
    +--> persistent input injector
    |       |-- virtual multitouch touchscreen
    |       `-- virtual gamepad
    |
    `--> Waydroid session/container
            |-- Android package lifecycle
            |-- direct GPU rendering
            `-- Android input stack
```

The CLI becomes another client of `wroidd`. Direct ADB and Waydroid shell
wrappers remain available for diagnostics and compatibility mode.

The current production boundary is an incremental form of this target.
`wroidd` protocol v2 owns the managed worker and a private per-launch bridge
broker. The desktop-user worker owns evdev/uinput, profile evaluation, the input
hot path, telemetry, and Waydroid user-level lifecycle. The daemon alone starts
the root-owned typed helper, which owns only the validated LXC event-node
bridge, one fixed Android input-device readiness probe, and crash rollback.
The worker receives an inherited versioned socket rather than a helper path.
Boot and render-property readiness remain in the desktop worker through
Waydroid's user API. The helper must match the daemon's paired staged release
and prove effective root through a side-effect-free check.
Mode `4750` limits execution to the `input` group and avoids per-game password
prompts. Its Hub bootstrap uses a detached unprivileged installer,
graphical Polkit authorization, an interprocess lease, and a write-sealed memfd
source. The root-owned fixed `/usr/bin/install` process never reads a mutable
staging pathname; if the detached owner disappears before the source is opened,
installation fails instead of publishing a partial helper. Daemon reuse is
bound to authenticated peer credentials and exact executable identity; only an
idle stale release may be replaced, through a revalidated pidfd. Moving profile
evaluation, capture, injection, and lifecycle cleanup into daemon-native
components remains a later migration.

## Workspace direction

- `wroid-core`: serialized profile model, validation, migrations, viewport math.
- `wroid-android`: bounded package format, archive structure, and native ABI
  inspection before Android adapters receive an install request.
- `wroid-runtime`: session-independent input state and binding execution.
- `wroid-input`: keyboard, mouse, and controller capture.
- `wroid-inject`: uinput touchscreen/gamepad and compatibility injectors.
- `wroid-android`: packages, activities, APK metadata, and Android diagnostics.
- `wroid-waydroid`: Waydroid lifecycle and properties.
- `wroid-daemon`: per-user service and typed IPC.
- `wroid-helper`: smallest possible privileged boundary.
- `wroid-cli`: automation and recovery client.
- `wroid-ui`: launcher, settings, controls editor, and diagnostics.
- `wroid-bench`: input latency and frame-time regression tools.

Crates are introduced incrementally. The current `wroid-adb` and
`wroid-waydroid` crates remain valid adapters while responsibilities are split.

## Input data path

```text
physical input
  -> capture backend
  -> normalized host event
  -> profile/binding engine
  -> logical touch frame
  -> TouchEngine validation
  -> persistent injector
  -> Linux evdev/uinput device
  -> Android EventHub/InputReader/InputDispatcher
```

A logical frame may contain transitions for several contacts, but one contact
must not appear more than once in the same frame. This keeps backend translation
deterministic and maps directly to one synchronization boundary.

The first production injector target is a Linux multitouch Type-B virtual device
using slots and tracking IDs. A persistent Android-side agent remains a fallback
research option, not the default architecture.

## Runtime lifecycle

```text
Stopped -> Preparing -> Running -> Stopping -> Stopped
              |            |
              v            v
            Failed <-------+
```

`Preparing` validates the profile, verifies renderer and device capabilities,
creates virtual devices, and starts input capture. `Running` owns all active
contact state. `Stopping` disables capture first, cancels all contacts, restores
per-game Waydroid properties, and then removes virtual devices.

A watchdog must perform the same contact cleanup when the UI disconnects or the
runtime crashes.

The user-side `launch-v2` transaction separately records whether a desktop
Waydroid session was running before privileged setup. It restores that state
after success or failure. A detached, token-scoped watchdog monitors the parent
PID and performs the same restore if the launcher process disappears before it
can disarm the recovery ticket. A Hub launch detaches this transaction from the
browser and writes a mode-`0600` active-session record under the user's
mode-`0700` runtime directory. Stop revalidates UID, Linux start ticks,
executable, and the `launch-v2` command, then signals the opened process through
pidfd so numeric PID reuse cannot affect another process. Session output goes to
the user's private state log instead of a terminal. On return, `launch-v2`
atomically publishes a bounded clean/failed outcome in the private user state
directory. The Hub child reaper writes only a missing, launch-correlated
fallback after hard process death, preserving any report already committed by
the session itself.

## Privilege boundary

The privileged helper may perform only allow-listed operations with validated
arguments. Candidate operations are:

- open an explicitly selected input device;
- create or destroy a named virtual input device;
- apply an input grab with an automatic lease timeout;
- execute a fixed Waydroid system lifecycle operation;
- install or switch a verified Android image component.

Profiles, package names, APK parsing, UI rendering, network access, and telemetry
remain outside the privileged process.

The current bridge helper narrows this list further: it accepts only a virtual
input node whose sysfs location, device name, virtual bus, vendor, and product
identify the Wroid touchscreen. Its runtime protocol can request only a fixed
`getevent -pl` readiness probe for that name and cleanup; it never receives a
profile, Android command, package, property, device name, or shell text.

## Graphics policy

Normal local play uses Waydroid's direct display path. Wroid may manage Android
resolution, density, orientation, and fullscreen state. Frame pacing follows
Waydroid's compositor-advertised refresh automatically; Wroid observes that
target and presentation-feedback state but does not write undocumented FPS
properties. Wroid must not capture and re-stream frames unless the user
explicitly enters a remote or recording mode.

Diagnostics must record host GPU, kernel driver, EGL/Vulkan renderer, Android
resolution, compositor, refresh rate, and software-renderer detection.

## Compatibility policy

Game compatibility is evaluated across independent dimensions:

- Android API and package format;
- native ABI and optional ARM translation;
- graphics API and driver;
- input model;
- Play services requirements;
- integrity or anti-cheat requirements.

Wroid reports these dimensions separately instead of presenting one opaque
"compatible" flag.

The current preflight reads the guest ABI list, Android version, native-bridge
property, Play Store/package inventory, and exposes a separate state for PUBG
Mobile, Free Fire, Brawl Stars, and Standoff 2. A missing known package is
rejected before `launch-v2` tears down the desktop Waydroid session.
