# Daemon-Owned Game Sessions Design

## Problem

The Hub currently spawns `wroid launch-v2` itself and owns the child handle. The game keeps running if the Hub window closes, but process reaping, exit-state tracking, and launch ownership disappear with the Hub process. `wroidd` has authenticated IPC and session metadata, yet its `start` operation only changes an in-memory enum.

## Decision

Make `wroidd` the process owner for normal Hub launches. Add one typed `launch_profile_v2` IPC request containing a validated session id, the already parsed profile, canonical profile path, resolution, and optional discovered keyboard/mouse paths. The daemon derives the worker executable from the authenticated peer PID, constructs only the fixed `launch-v2` argument shape, opens the private game log itself, starts a new process group, records the PID, and reaps the child independently of the Hub.

The existing direct `launch-v2` command remains available for diagnostics and input self-test. Existing active-session state and the detached Waydroid restoration watchdog remain the low-level crash/stop safety net.

## Alternatives Considered

1. Register a Hub-spawned PID with `wroidd`. Rejected because the Hub would still own the child and its reaper.
2. Move all launch orchestration from `wroid-cli` into shared libraries. Architecturally pure, but it would combine a large module extraction with the ownership change and delay a testable product improvement.
3. Let IPC supply an arbitrary executable and arguments. Rejected because it weakens the typed protocol into a generic command runner.

## Protocol and State

- Increment the protocol version because the session snapshot gains process state.
- `launch_profile_v2` atomically prepares the control plan and starts the worker.
- A session snapshot exposes optional `processId` and bounded `detail` fields.
- Spawn success transitions `Preparing -> Running`.
- Spawn failure transitions `Preparing -> Failed` and returns a protocol error.
- A daemon reap transitions a successful child to `Stopped` and a non-zero/signalled child to `Failed`.
- Stop sends `SIGTERM` to the exact live child owned by `std::process::Child`, marks it `Stopping`, and lets the daemon reap it. Metadata-only prepared sessions retain the existing synchronous stop behavior.
- Only one managed game process may be preparing, running, or stopping at once.

## Security

- Keep the 0600 Unix socket, `SO_PEERCRED` UID check, 1 MiB message bound, and validated session ids.
- Read the peer PID from `SO_PEERCRED`; resolve `/proc/<pid>/exe`; require a current-user-owned regular executable with at least one execute bit and no group/other write bits.
- Never accept free-form arguments or an executable path from JSON.
- Require an absolute canonical regular profile file and resolutions in `1..=8192`.
- Optional input paths must be absolute `/dev/input/...` paths. The Hub continues to select them only from fresh discovery results.
- Game output goes to the existing private `0700` state directory and `0600` `game-session.log` with `O_NOFOLLOW`.

## Hub Behavior

The launch action keeps its current helper, graphics, package, and input preflight. It then ensures `wroidd` is running and sends the typed launch request. The response is immediate after a successful spawn, so the HTTP handler does not wait for Waydroid startup. Stop first asks `wroidd` to stop its active managed session; the pidfd-based legacy stop remains a fallback for direct or pre-upgrade launches.

## Verification

- Unit tests cover request serialization, validation, fixed argument construction, state transitions, single-active-session rejection, stop signalling, and reap outcomes.
- Hub tests assert that launch data maps to the typed request and that no free-form arguments cross IPC.
- Run daemon, CLI, Hub, workspace, Clippy, formatting, JavaScript syntax, and release installation checks.

