# ADR 0001: Persistent multitouch input transport

- Status: Accepted
- Date: 2026-06-17

## Context

The MVP executes `adb shell input` or `waydroid shell input` for taps and
swipes. Each action creates a process and performs a shell round trip. This is
useful for diagnostics but cannot provide stable gaming latency, continuous
mouse aim, or simultaneous movement, aiming, and firing.

## Decision

Gaming mode will use a persistent injection backend. The first implementation
target is a Linux `uinput` multitouch Type-B device visible to the Waydroid
container. The runtime represents input as synchronized logical touch frames
with explicit contact identifiers and down/move/up/cancel phases.

`wroid-runtime` owns backend-independent touch state. `wroid-inject` will own the
Linux-specific event translation and device lifecycle.

ADB and Waydroid shell input remain as an explicitly selected compatibility and
diagnostics backend. They are prohibited from the production gaming hot path.

## Consequences

- The runtime can support true multitouch and continuous contacts.
- Input latency becomes measurable without process-spawn noise.
- Device permissions and cleanup require a narrow privileged helper.
- The project needs integration tests against a running Waydroid instance.
- Compatibility mode remains functional on systems where uinput is unavailable.
