# Persistent Daemon Touchscreen Design

## Problem

Production game launches currently create a virtual touchscreen, install a
temporary Waydroid bridge, start Android, and tear all of it down again when the
game worker exits. The security boundary is daemon-owned, but the actual uinput
device and Waydroid user session are still worker-owned. This makes every test
or game visibly open and close Waydroid and prevents the next launch from
reusing a ready Android runtime.

The next production milestone is to keep the touchscreen bridge and Waydroid
session alive across consecutive games while retaining bounded cleanup, the
existing narrow root helper, and the sub-5 ms input contract.

## Decision

`wroidd` will own one lazy, persistent runtime platform for its whole process
lifetime. A dedicated daemon thread owns:

- one 10-slot `UinputTouchInjector` with a canonical 65536 by 65536 absolute
  range;
- the exact-release privileged bridge-helper session;
- the desktop Waydroid session and its current configured resolution;
- the currently attached worker's validated touch state.

Each game worker keeps profile evaluation and physical evdev capture, but
replaces its local uinput injector with a synchronous remote injector over one
inherited private `SOCK_SEQPACKET` socket. The daemon injects every accepted
frame into the persistent uinput device and acknowledges it only after the
kernel write succeeds. A worker exit ends only that input attachment: all
remaining contacts are cancelled and host grabs are released, while Waydroid,
the helper, LXC bridge, and virtual touchscreen remain ready for the next game.

The public daemon IPC remains a control plane. Touch frames never use the
pathname-based daemon socket, and no new public method accepts arbitrary input
events.

## Alternatives Considered

1. Keep the worker-owned uinput device and only stop tearing down Waydroid.
   Rejected because the bridged event node disappears with the worker and the
   next device normally receives a different `/dev/input/eventN` identity.
2. Let the setuid helper own uinput and receive gameplay frames. Rejected
   because it turns a small lifecycle helper into a long-lived root hot path.
3. Pass a duplicate uinput descriptor directly to each worker. This avoids an
   IPC hop, but the daemon cannot reliably validate or cancel the worker's
   logical contacts after a crash. It also couples the worker to evdev device
   internals instead of the existing `TouchInjector` contract.

The selected design keeps privileged code out of gameplay dispatch and gives
the daemon authoritative contact state and deterministic crash cleanup.

## Ownership and Launch Sequence

The persistent platform is lazy so merely starting `wroidd` never opens a UI or
changes Waydroid. The first managed game launch performs one initialization:

1. `wroidd` validates the typed launch request and creates the private worker
   socket before spawning the fixed hidden worker.
2. The platform thread creates the canonical uinput touchscreen and discovers
   its exact virtual `/dev/input/eventN` node.
3. If an unrelated pre-existing Waydroid session prevents bridge installation,
   the platform stops it through the desktop-user command path. This is the one
   controlled first-use restart; it is not repeated per game.
4. The daemon starts the exact paired helper. The helper validates the virtual
   device independently and holds the transactional LXC bridge.
5. The platform starts Waydroid, applies the requested display resolution,
   performs any required one-time restart, confirms Android readiness, and
   verifies that Android sees the Wroid touchscreen.
6. It shows the UI and launches the validated package according to the original
   typed request, then reports readiness to the worker.
7. The worker begins evdev capture only after readiness and submits touch frames
   over the inherited channel.

For later launches at the same resolution, steps 2 through 5 are skipped. The
daemon reuses the running Android session and bridge, optionally raises the UI,
launches the requested package, and attaches the new worker. A resolution
change restarts the persistent Waydroid session once but keeps the same uinput
device and helper bridge.

Stopping a game or closing the Hub cancels controls and detaches the worker but
does not stop Android. Orderly daemon shutdown first terminates any worker,
cancels contacts, stops Waydroid, sends the helper's graceful cleanup command,
and then drops uinput. If the daemon crashes, the private descriptors close;
the helper's existing EOF recovery force-stops Waydroid and removes the managed
bridge, while the kernel destroys uinput and any evdev grabs.

## Private Runtime Protocol

The inherited channel changes from a startup-only JSON stream to a versioned
fixed-size `SOCK_SEQPACKET` protocol. Worker generation increments so a new
daemon never starts an old worker with incompatible descriptor semantics.

The daemon supplies package, resolution, UI, and launch intent directly to the
platform thread from the already validated `GameLaunchRequest`; the worker
cannot override them through the channel. The wire protocol contains only:

- `ready` or a bounded startup error;
- `touch_frame { sequence, events[] }`;
- `touch_result { sequence, ok | bounded error }`;
- `finish` and `finished`.

Every packet has a magic value, protocol version, opcode, sequence, and exact
payload length. A frame contains 1 through 10 unique contacts. Each event is a
contact id, closed phase enum, and logical x/y coordinate. Packets larger than
the fixed maximum, truncated packets, unknown opcodes, unknown phases,
out-of-order sequences, coordinates outside the session resolution, duplicate
contacts, invalid contact transitions, or frames beyond 10 active contacts fail
closed.

The daemon scales logical coordinates to `0..=65535` with endpoint-preserving
integer rounding immediately before injection. Initial platform readiness is
bounded to 300 seconds so one Android boot plus a required resolution restart
can finish. Gameplay acknowledgments and all writes are bounded to two seconds.
The daemon polls the socket at 250 ms while idle so it can detect helper death
without imposing a total gameplay or idle timeout. Normal gameplay uses
preallocated packet and event buffers and performs no process spawn, shell call,
JSON parse, or heap allocation per frame.

The worker waits for the matching acknowledgment before committing its local
`TouchEngine` state. The daemon also validates and commits its independent
state only after uinput succeeds. This retains atomic failure semantics across
the IPC boundary.

## Waydroid Lifecycle

`DesktopWaydroidSession` becomes daemon-owned in production. The platform
thread exposes internal typed operations for start-or-adopt, resolution
configuration, Android readiness, show UI, package launch, and final stop.
Production workers no longer call `ensure_container_stopped`, create uinput,
start/stop Waydroid, or open/finish the root bridge.

Root-only diagnostic binaries keep the current in-process path so recovery and
low-level bridge diagnosis do not depend on `wroidd`. Their temporary lifecycle
is explicitly diagnostic and does not define normal Hub behavior.

The first implementation guarantees persistence across game sessions within
one daemon lifetime. Live hot-plug reconciliation across an abrupt daemon
replacement is a follow-up: orderly replacement remains safe and may require a
single controlled Waydroid restart before the next game, never one restart per
game.

## Failure Semantics

- Worker spawn failure: close the unused attachment without changing a ready
  platform; if initialization was already requested, it may finish and remain
  ready.
- Worker EOF/crash: cancel every daemon-tracked contact in one synchronized
  frame and preserve the platform for the next launch.
- Invalid or failed touch frame: return a bounded error, cancel contacts, close
  the attachment, and mark the worker outcome failed.
- Helper initialization failure: stop any newly started Waydroid session,
  destroy the new uinput device, return the startup error, and remain eligible
  for a clean retry.
- Waydroid startup or verification failure: preserve the primary error, attempt
  helper/Waydroid rollback, and report cleanup errors as additional details.
- Helper death after readiness: fail the current attachment, cancel contacts,
  tear down the platform, and require full initialization on the next launch.
- Daemon shutdown/crash: retain the existing parent-death, helper EOF recovery,
  private socket, LXC lease, and release-matched helper invariants.

No failure may leave a worker marked successful when its final contact cleanup
or runtime-channel result failed.

## Security Invariants

- Hub, CLI, daemon, worker, evdev capture, and touch injection stay
  unprivileged; only the fixed bridge helper is setuid root.
- The helper still accepts only an independently authenticated Wroid virtual
  event node and fixed readiness/cleanup commands. It never receives a touch
  frame, profile, package, display property, shell fragment, or host input
  device.
- The runtime channel is an unnamed inherited socket with close-on-exec restored
  in the worker. It is unavailable to same-UID clients through the public
  daemon socket.
- The daemon derives package and launch policy from the validated request. The
  private protocol cannot select commands or paths.
- One-active-managed-session remains enforced. Sequence and state validation
  prevent replay or cross-session contact reuse.
- Exact daemon/worker/helper release pairing and authenticated stale-daemon
  handoff remain required.

## Verification

Automated tests must prove:

- exact packet encoding, size/version/opcode/sequence rejection, partial packet
  rejection, and I/O deadlines;
- coordinate endpoint and midpoint scaling for landscape and portrait
  resolutions;
- independent worker and daemon state commit only after a successful ACK/uinput
  write;
- 10 simultaneous contacts, one synchronized cleanup frame, and no contact leak
  after worker EOF, injected failure, SIGTERM, or daemon drop;
- first launch initializes once, second launch reuses the same event node,
  helper, and Waydroid owner, and stopping a worker does not stop Waydroid;
- resolution changes restart only Waydroid while preserving the touchscreen;
- helper/Waydroid failures roll back and allow a later retry;
- old worker generations are rejected before spawn and only the intended
  runtime descriptor is inherited;
- direct root diagnostics retain their existing behavior.

The release gate includes full workspace tests, strict Clippy, rustfmt,
JavaScript model/syntax tests, example validation, and `git diff --check`. A
headless host benchmark must submit at least 20,000 acknowledged frames with
p99 reader-to-inject latency below 5 ms and verify 10/10 contact release.

One final live acceptance may visibly touch Waydroid. It must be announced
before execution and run only after all non-GUI gates pass. The acceptance
starts two bounded no-APK sessions against one daemon and proves that the first
initializes the bridge, the first worker exit leaves Waydroid ready, the second
reuses the same bridge without closing the UI, and final daemon shutdown leaves
no helper, worker, bridge include, uinput node, or held contact.

## Deferred Work

- Live LXC hot-plug reconciliation that avoids the one controlled restart after
  an abrupt daemon replacement.
- Moving physical keyboard/mouse capture and profile evaluation into `wroidd`.
- Gamepad input, macros, split APK/OBB installation, image management, and
  cross-vendor hardware acceptance.
