# GameMode integration design

## Goal

Give normal Hub-launched game sessions the host performance optimizations provided by Feral GameMode when it is safely available, without making GameMode a dependency or weakening the daemon launch boundary.

## Chosen approach

Wroid exposes a persisted `Auto / Off` performance mode. `Auto` is the default and asks the per-user daemon to launch the existing Wroid game worker through a trusted system `gamemoderun`. `Off` launches the worker directly. Input self-tests and other tools remain direct launches.

Alternatives considered:

- Always enable GameMode: simplest, but gives users no escape from machine-specific GameMode configuration or custom scripts.
- Link to `libgamemode` directly: avoids the wrapper, but adds an ABI/runtime dependency and more lifecycle code.
- `Auto / Off`: preserves the optional dependency, has a safe default, and remains understandable in the Hub.

## Security boundary

The Hub sends only a boolean `gameMode` field in the typed daemon request. It never sends an executable or wrapper path.

The daemon searches a fixed list of absolute system paths and accepts a wrapper only when it is a canonical regular file owned by root, executable, and not writable by group or others. Before spawning it removes `GAMEMODERUNEXEC` and `LD_PRELOAD`, because the installed helper script consumes both variables. The daemon then supplies the already authenticated peer executable as the first fixed argument, followed by the existing fixed `launch-v2` argument plan.

If no trusted wrapper exists, `Auto` falls back to the direct worker. A wrapper that was selected and subsequently fails is reported as a normal managed-session failure; Wroid does not secretly retry a game launch because that could create two Android launch attempts.

## Data and UI

Preferences schema v1 gains a backward-compatible `gameMode` boolean with a default of `true`. The Hub renders a compact toggle beside the resolution presets and passes its current value on normal launch. The daemon request uses the same camelCase field with a default of `false` for older clients.

The launch response states whether GameMode was requested or the direct path was selected. Availability remains opportunistic: the toggle describes `Auto when installed`, so a missing optional package is not shown as a blocker.

## Verification

Tests cover preference migration and persistence, request serialization, fixed command planning, trusted-wrapper validation, disabled and unavailable fallbacks, and environment sanitization. Final checks include all workspace tests, formatting, Clippy, JavaScript syntax, release installation hashes, and a stopped-runtime smoke test.
