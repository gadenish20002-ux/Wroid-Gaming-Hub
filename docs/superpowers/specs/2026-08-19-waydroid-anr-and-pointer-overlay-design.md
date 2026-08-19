# Waydroid ANR and Pointer Overlay Design

## Problem

Managed game startup currently opens the complete Waydroid desktop before it
launches the selected package. That puts Trebuchet on the interactive startup
path even though a game does not need the Android launcher. The captured ANR
from 2026-08-19 shows Trebuchet missing a five-second `MotionEvent` deadline
while its main thread waits in `folio_wait_bit_common` during a cold Android
boot.

Android's `show_touches` and `pointer_location` system settings are also both
persisted as `1`. They draw click markers, pointer trails, and coordinates for
every mouse event. These diagnostics are unwanted in a gaming product and add
work to the input/render path.

The Waydroid data directory is on Btrfs without the NOCOW flag. Copy-on-write
storage can amplify Android's write-heavy workload, but converting an existing
Android data tree in place is a destructive operation and is outside this fix.

## Decision

### Direct managed game launch

When a package launch is requested, the managed session launches that package
directly and does not call `waydroid show-full-ui` first. The full desktop is
opened only when UI was requested without a package launch. Explicit Hub
actions such as "Open Waydroid UI" retain their existing behavior.

This removes Trebuchet from the normal game startup path rather than trying to
recover it after an ANR.

### Fixed Android input-diagnostic cleanup

The already-running, release-matched privileged bridge helper disables the two
Android system settings after it has verified that Android sees the Wroid
touchscreen:

```text
settings put system show_touches 0
settings put system pointer_location 0
```

Both commands are constructed from absolute executables and literal arguments,
through the existing cleared-environment privileged command builder. No setting
name, namespace, value, executable, or shell fragment comes from the Hub,
daemon, worker, profile, or user input.

The existing `VERIFY_ANDROID_INPUT` request remains the only readiness request.
Its success reply now means both that the virtual touchscreen is visible and
that pointer diagnostics are disabled. A command failure aborts managed startup
instead of silently leaving an expensive debug overlay enabled.

Because Android persists these settings, one successful managed game launch
also removes the overlays from later explicit desktop-UI sessions. Reinstalling
the updated paired helper is required before the new behavior can run.

### Btrfs CoW warning

The storage report detects whether the Waydroid data directory is on Btrfs and
whether its directory inode has the NOCOW flag. Btrfs with CoW enabled produces
a warning that explains the potential Android I/O latency. Capacity warnings
remain higher priority than the filesystem warning.

The report is diagnostic only. It must not mutate attributes, copy, delete, or
migrate the existing Android data directory.

## Components and data flow

1. The game worker creates the virtual touchscreen and activates the verified
   privileged bridge helper.
2. Android starts and reports user readiness.
3. The worker requests fixed Android input verification.
4. The helper waits for `Wroid Gaming Touchscreen`, disables `show_touches` and
   `pointer_location`, and then emits the existing ready reply.
5. The worker launches the requested game package directly. It opens Trebuchet
   only for a UI-only session.
6. Hub storage diagnostics report Btrfs CoW risk without changing storage.

## Error handling and security

- Failure of either Android settings command is a startup failure with bounded
  command output in the error.
- Malformed or duplicate helper protocol requests keep their current rejection
  behavior and cleanup semantics.
- The privileged helper accepts no new caller-controlled fields or general
  command execution.
- UI-only and no-launch diagnostic sessions retain the full-desktop behavior
  when `show_ui` is true.
- Storage probing failures produce `unknown` diagnostics and do not block game
  launch.

## Testing

- A helper unit test asserts the exact absolute executable and literal argument
  vectors for both settings commands.
- A helper protocol test proves the ready reply is withheld when diagnostic
  cleanup fails and emitted only after cleanup succeeds.
- A game-session workflow test proves package launch excludes full-desktop
  launch, while a UI-only session still opens the full desktop.
- Storage tests cover capacity precedence, non-Btrfs storage, Btrfs with NOCOW,
  and Btrfs with CoW enabled.
- Focused crate tests, the complete workspace test suite, formatting, and a
  release build verify the final change.

## Acceptance criteria

- Managed game launches do not start Trebuchet before the game.
- Red click markers, pointer trails, and top-edge coordinate logging are absent
  after the first managed launch with the updated helper.
- The helper remains a fixed, typed privileged boundary.
- Btrfs CoW risk is visible but Wroid never modifies existing Android data.
- Existing no-launch, no-UI, cleanup, and helper-authentication behavior remains
  covered and passing.
