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
    |       |-- authenticated Wroid event-node bridge install/cleanup
    |       |-- fixed Android input-device readiness probe
    |       `-- bridge crash rollback
    |
    +--> persistent input platform
    |       |-- canonical 10-slot uinput touchscreen
    |       |-- private runtime touch channel
    |       `-- daemon-side TouchEngine cleanup
    |
    `--> Waydroid session/container
            |-- Android package lifecycle
            |-- direct GPU rendering
            `-- Android input stack
```

The CLI becomes another client of `wroidd`. Direct ADB and Waydroid shell
wrappers remain available for diagnostics and compatibility mode.

The current production boundary matches the persistent-daemon touchscreen
milestone. `wroidd` protocol v2 owns the managed worker and one lazy platform
thread for the daemon lifetime. The worker keeps unprivileged evdev capture,
profile evaluation, focus handling, and telemetry, but it receives only inherited
runtime descriptor `198` and submits fixed binary touch frames over the private
`SOCK_SEQPACKET` channel. The daemon owns the canonical uinput touchscreen, the
exact-release helper lifecycle, Waydroid user-level lifecycle, coordinate
scaling, ACK-after-uinput-write semantics, and final contact cleanup.

The first managed launch may perform one controlled Waydroid restart to install
or reconcile the bridge. Later same-resolution launches in the same daemon
lifetime reuse the same uinput event node, helper bridge, and Waydroid owner.
Worker exit, Stop, or Hub closure detaches that attachment, cancels any active
daemon-side contacts, and releases host grabs; it does not stop Waydroid or drop
the bridge per game. The helper must match the daemon's paired staged release
and prove effective root through a side-effect-free check.
Mode `4750` limits execution to the `input` group and avoids per-game password
prompts. Its Hub bootstrap uses a detached unprivileged installer,
graphical Polkit authorization, an interprocess lease, and a write-sealed memfd
source. The root-owned fixed `/usr/bin/install` process never reads a mutable
staging pathname; if the detached owner disappears before the source is opened,
installation fails instead of publishing a partial helper. Daemon reuse is
bound to authenticated peer credentials, a pidfd opened at authentication, and
exact executable identity. Only an idle stale release may be frozen, checked
again for worker children, and replaced; a detached watchdog guarantees resume
if the upgrader dies. If `wroidd` crashes instead of shutting down cleanly, the
private descriptors close, the helper's EOF recovery force-stops Waydroid and
removes the managed bridge include, and the kernel destroys the daemon-owned
uinput device. Live LXC hot-plug reconciliation after abrupt daemon replacement
is deferred; the next managed launch may still need the one controlled restart.
Moving profile evaluation and physical capture into daemon-native components
remains a later migration.

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
  -> worker TouchEngine validation
  -> inherited runtime channel generation 2
  -> daemon TouchEngine validation and canonical scaling
  -> persistent uinput injector
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
attaches to the daemon-owned platform, and starts input capture only after the
runtime channel reports ready. `Running` has mirrored contact state: the worker
commits local state only after the daemon ACK, and the daemon commits only after
the uinput write succeeds. `Stopping` disables capture first, sends `finish`
when possible, cancels any remaining daemon contacts, and releases host input
grabs. It does not tear down Waydroid, the helper bridge, or the uinput device
for ordinary game exits.

Orderly daemon shutdown performs the broader cleanup: terminate/reap workers,
finish their runtime attachments, cancel contacts, stop Waydroid, ask the helper
to remove the managed bridge, and drop uinput last. A watchdog or helper EOF
path performs equivalent bridge cleanup if the daemon/runtime crashes.

The user-side `launch-v2` worker is now the daemon-supervised input/profile
executor, not the owner of Waydroid or the bridge. A Hub launch writes a
mode-`0600` active-session record under the user's mode-`0700` runtime
directory. Stop revalidates UID, Linux start ticks, executable, and the
`launch-v2` command, then signals the opened process through pidfd so numeric
PID reuse cannot affect another process. Session output goes to the user's
private state log instead of a terminal. On return, `launch-v2` atomically
publishes a bounded clean/failed outcome in the private user state directory.
The Hub child reaper writes only a missing, launch-correlated fallback after
hard process death, preserving any report already committed by the session
itself. Platform cleanup belongs to `wroidd`, which keeps the ready platform
alive after ordinary worker exit and tears it down only on daemon shutdown or a
poisoned platform failure.

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

The current production helper narrows this list further: it accepts only a
daemon-created virtual input node whose sysfs location, device name, virtual
bus, vendor, and product identify the Wroid touchscreen. Its runtime protocol
can request only bridge open, one fixed `getevent -pl` readiness probe for that
name, health observation, and cleanup. It never receives a touch frame, profile,
package, display property, host physical input path, arbitrary device name, or
shell text. Root-only diagnostic binaries keep the temporary in-process bridge
path for recovery and low-level smoke testing; that exception is not the normal
Hub/CLI gameplay lifecycle.

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
