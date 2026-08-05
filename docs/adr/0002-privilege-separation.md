# ADR 0002: Separate UI, runtime, and privileged operations

- Status: Accepted
- Date: 2026-06-17

## Context

Waydroid system operations, evdev capture, and uinput creation may require
additional privileges. Running the complete launcher or controls editor through
`sudo` would expose a large attack surface and break normal Wayland and D-Bus
integration.

## Decision

The GUI and CLI remain unprivileged clients. A per-user daemon owns game session
state. A minimal system helper performs only allow-listed privileged operations
through a typed IPC protocol and policy-controlled authorization.

The helper must never accept arbitrary commands or shell fragments. Input grabs
and virtual devices are leased to a runtime session and automatically released
when the session ends or the client disappears.

The current implementation keeps evdev capture, uinput,
profile execution, focus handling, package lifecycle, and telemetry in the
desktop-user process. Its standalone short-lived `wroid-helper` accepts one validated
event node plus the fixed `CLEANUP` message; EOF triggers a fixed Waydroid stop
and bridge rollback. The desktop installer stages the helper, a typed installer
copies it root-owned to `/usr/lib/wroid`, and Hub verifies ownership, mode
`4750`, effective-root activation, a sanitized environment, and release
equality. The Hub bootstrap uses a detached leased process, desktop Polkit
authorization, fixed `/usr/bin/install` arguments, and a write-sealed memfd
instead of a mutable pathname or terminal. This removes per-launch `sudo`.
Protocol v1 of the per-user daemon now owns typed profile preparation and
session state; moving live capture and helper activation to it remains future
work.

## Consequences

- The desktop UI works in the normal user session.
- Privileged code remains small enough to audit.
- IPC types and protocol versioning become part of the public architecture.
- Packaging must install service and authorization policy files.
- Crash recovery is implemented at the service boundary rather than in the UI.
