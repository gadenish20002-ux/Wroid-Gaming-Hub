# Daemon-Owned Bridge Helper Design

## Problem

Normal Hub launches are process-owned by the per-user `wroidd`, but the
daemon-supervised `launch-v2` worker still executes the setuid
`/usr/lib/wroid/wroid-helper` directly. The helper protocol is narrow and the
worker is unprivileged, yet helper activation, readiness, and cleanup are not
owned by the daemon control plane described by Architecture v2. A replaced CLI
binary can also keep talking to an older live daemon that still constructs the
old worker command.

The next production step is to make `wroidd` the sole owner of the privileged
helper without moving input capture, profile evaluation, or the latency-critical
dispatch loop into the daemon in the same change.

## Decision

For every managed game launch, `wroidd` creates a private Unix socket pair. It
keeps one endpoint in a per-session bridge broker and passes the other endpoint
to the worker as one inherited descriptor. The worker creates the persistent
uinput touchscreen as it does today, then uses a bounded, versioned bridge
protocol over that descriptor. The daemon-side broker validates each request,
starts and owns the installed helper, and forwards only the helper's existing
fixed readiness and cleanup operations.

The worker never receives a helper path and never executes the helper. The
helper never receives a profile, package, resolution, host device, shell text,
or daemon socket path. Physical input capture and all steady-state touch
dispatch remain in the desktop-user worker, so this change adds no IPC hop to
the gameplay hot path.

Ordinary `wroid launch-v2` invocations become daemon clients as well. The daemon
uses a hidden, fixed worker mode and inherited bridge descriptor when it starts
the actual transaction. Root-only diagnostic binaries and explicit recovery
commands remain direct diagnostic paths; they are not presented as production
game launches.

## Alternatives Considered

1. Keep direct worker/helper activation. This preserves the current code but
   leaves the privileged lifecycle outside the daemon boundary and does not
   satisfy the roadmap milestone.
2. Move all evdev capture, uinput, profile dispatch, Waydroid lifecycle, and
   telemetry directly into `wroidd`. This is the eventual simplification, but
   combining it with the privilege-boundary change would make crash recovery
   and latency regressions much harder to isolate.
3. Add bridge methods to the public daemon Unix socket. Rejected because
   readiness can block while Android starts, the public listener is currently
   single-threaded, and same-UID callers would require additional session-token
   authorization. A per-launch inherited socket pair is smaller and has no
   discoverable pathname.

## Process and Ownership Model

`ManagedProcesses` stores one managed session object rather than only a
`std::process::Child`. That object owns:

- the exact worker child;
- the daemon end of the private bridge channel through a broker thread;
- bridge state and the broker completion result;
- the existing session id used for stop and reap transitions.

The launch sequence is:

1. Validate the launch request, paired staged and installed helper, profile
   path, input paths, resolution, worker executable, worker protocol generation,
   and daemon release identity.
2. Create a close-on-exec Unix socket pair.
3. Start the broker with the daemon endpoint.
4. Spawn the fixed worker command, duplicating only the worker endpoint to a
   fixed descriptor in `pre_exec` and closing unrelated copies.
5. Mark the daemon session running only after spawn succeeds.
6. The worker adopts the descriptor, immediately restores close-on-exec, and
   creates the uinput touchscreen.
7. The worker opens, verifies, and finishes the bridge through the internal
   protocol. Gameplay events do not cross the channel.

If worker spawning fails, closing its endpoint makes the broker exit without
starting a helper. When the worker is reaped, the daemon joins the broker and
combines worker and bridge errors so cleanup failures are not hidden by an
earlier gameplay failure.

The worker is configured with Linux parent-death signalling. `pre_exec` sets
`PR_SET_PDEATHSIG` and verifies that the parent PID is still the daemon PID,
closing the fork-to-setup race. If `wroidd` dies, the worker receives `SIGTERM`,
the broker side disappears, the helper sees EOF, and the existing helper
fail-safe stops Waydroid and removes the managed bridge. The worker still owns
its normal signal-driven contact cancellation and evdev ungrab path.

## Internal Bridge Protocol

The bridge channel has its own protocol version, independent of the public
daemon protocol. Messages are newline-delimited JSON with a 4 KiB maximum and
exactly one request followed by one response. Frame writes have a three-second
timeout. The broker allows five seconds for the initial `open` and 120 seconds
from `opened` to `verify_android_input`; both are startup-only states. After
verification, gameplay duration is intentionally unbounded while the exact
worker child remains alive. Android verification itself retains the helper's
bounded retry count.

Client requests are a closed enum:

- `open { eventNode }`
- `verify_android_input`
- `finish { waydroidStopped }`

Server responses are a closed enum:

- `opened`
- `android_input_ready`
- `finished`
- `error { code, detail }`

Every frame includes the internal protocol version. The broker state machine
accepts only `open -> verify_android_input -> finish`. Duplicate, reordered,
oversized, incomplete, malformed, or version-mismatched messages fail closed.
Only the `open` request contains data. Its path must be an absolute
`/dev/input/eventN` path, and the existing root helper independently validates
the sysfs location, Wroid device name, virtual bus, vendor, and product before
changing LXC configuration.

`finish { waydroidStopped: true }` requests graceful helper cleanup. A false
value closes the helper protocol without sending its graceful command, retaining
the helper's forced Waydroid-stop and bridge-cleanup behavior.

## Failure and Cleanup Semantics

- Worker EOF before `open`: exit without privileged work.
- Worker EOF after `open`: drop the helper channel; the helper force-stops
  Waydroid and removes the bridge.
- Invalid broker request: return one bounded error when possible, then fail
  closed through the same helper cleanup path.
- Helper start/readiness failure: report it to the worker and keep the session
  outcome failed.
- Worker `SIGTERM`: cancel contacts and release grabs first, stop Waydroid, then
  request graceful bridge cleanup.
- Worker crash: uinput and evdev descriptors close in the kernel; broker EOF
  triggers helper recovery.
- Daemon crash: parent-death signalling terminates the worker and helper stdin
  closes when the broker disappears.
- Multiple failures: preserve worker, Waydroid-stop, and helper-cleanup details
  in the bounded last-session result and daemon session detail.

No error path may leave the bridge broker detached from both the managed worker
and the daemon. Daemon shutdown stops workers before joining brokers.

## Daemon Release Handoff

The public daemon envelope remains protocol v2, but `GameLaunchRequest` gains a
required worker protocol generation. Deserialization defaults a missing value
to zero solely so the daemon can reject an old client cleanly before spawn.
Changed worker launch semantics also require the running daemon executable to
match the release selected by the client. `ensure_running` must compare the
authenticated daemon PID from `Ping` with the exact desired `wroidd` executable
using followed file identity (device and inode), not pathname text.

If identities match, the existing daemon is reused. If they differ, the client
uses the compatible `List` request and may replace the daemon only when no
session has a managed process in Preparing, Running, or Stopping state. Stopped,
failed, and metadata-only prepared records do not hold runtime resources and do
not block replacement. The client sends `SIGTERM` through a pidfd opened for the
authenticated peer, waits for the private socket and lease to disappear, then
starts the current daemon. An active managed session makes launch refuse with an
explicit instruction to stop it first. Wroid never replaces a daemon during an
active game and never signals a PID that has not been authenticated through the
private socket and revalidated through pidfd.

This release check prevents an old daemon from constructing the legacy direct-
helper worker command after `wroid desktop install` publishes new binaries.
The worker-generation field prevents a new daemon from trying to start an old
client executable that cannot adopt the inherited bridge descriptor.

## Public CLI and Hub Behavior

The Hub API remains asynchronous: a successful daemon response means the
managed worker was spawned, not that Android is already ready. Existing active
session, Stop, log, outcome, and performance displays remain valid.

The public `launch-v2` command performs its existing profile, graphics,
compatibility, and input selection preflight, sends the typed launch request to
`wroidd`, and returns after managed spawn. Only the daemon-constructed hidden
worker invocation enters the desktop restoration and input session code. The
hidden mode requires a valid inherited bridge descriptor; invoking it manually
must fail before uinput or Waydroid mutation.

The bounded no-APK input self-test uses the same daemon-owned helper path, so it
remains the live acceptance workflow before games or a Google account are
installed.

## Security Invariants

- The Hub, CLI, daemon, and game worker remain unprivileged.
- Only `wroidd` executes the exact installed helper after its current ownership,
  mode, effective-root, and staged-release checks pass. The paired staged helper
  is derived from the daemon executable's release directory, opened without
  following a final symlink, validated as a non-writable executable owned by the
  desktop user, and compared byte-for-byte before activation.
- The worker receives one already-open channel, not a helper executable or a
  reusable filesystem credential.
- No arbitrary command, executable, environment entry, package, property, or
  profile data crosses the bridge protocol.
- The inherited descriptor is not exposed through the public daemon socket and
  is close-on-exec again immediately after worker startup.
- The helper retains its independent device identity validation and global
  crash-safe bridge lease.
- Existing public daemon socket permissions, peer UID checks, message bounds,
  fixed launch arguments, private logs, and one-active-session rule remain.

## Verification

Focused automated tests must prove:

- exact serialization and version rejection for every bridge frame;
- bounds, timeout handling, and strict state ordering;
- event-node path rejection before helper activation;
- worker command construction contains the fixed hidden mode and descriptor but
  no helper path;
- only the intended descriptor is inherited and the worker restores
  close-on-exec;
- spawn failure, EOF before/after open, SIGTERM, worker crash, daemon shutdown,
  and helper failure all converge on deterministic cleanup;
- combined worker and broker failures remain visible;
- ordinary `launch-v2` routes through `wroidd` while hidden mode cannot be used
  without an inherited channel;
- worker-generation zero and mismatches are rejected before spawn;
- matched daemons are reused, resource-idle stale daemons are replaced by
  authenticated pidfd signalling, and active stale daemons are never replaced;
- existing Hub launch/Stop and active-session behavior remains compatible.

The full workspace tests, Clippy with warnings denied, rustfmt, repository
JavaScript syntax/model tests, example validation, and `git diff --check` remain
required. Release acceptance rebuilds and installs `wroid`, `wroidd`, and the
helper, then runs the bounded no-APK production input self-test. Acceptance must
confirm the daemon owns the helper process while the bridge is active, ten
simultaneous virtual contacts remain supported, contacts and grabs are released,
Waydroid returns to its previous state, no helper/worker remains, and managed
LXC configuration is removed.

## Deferred Work

Moving profile evaluation and the input hot path from the worker into daemon-
native components remains a later migration. Gamepad support, split-package
installation, image-component management, and live calibration of installed
games are not part of this privilege-boundary change.
