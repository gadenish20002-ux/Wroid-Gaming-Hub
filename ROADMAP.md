# Wroid Gaming Hub Roadmap

Wroid is being developed performance-first: reliable low-latency input, safe Waydroid lifecycle management, and clear privilege boundaries come before broader packaging and polish.

This file is the contributor-facing overview. The detailed engineering checklist lives in [`docs/roadmap.md`](docs/roadmap.md).

## Current focus

- Stabilize the persistent evdev/uinput input path and Waydroid bridge lifecycle.
- Continue moving orchestration behind the per-user daemon and typed helper boundary.
- Improve Controls Studio and profile-v2 authoring workflows.
- Expand hardware, graphics, ABI, Waydroid, and game compatibility diagnostics.
- Validate production latency and simultaneous-contact behavior on more real hardware.

## Next

- Test across more Linux distributions and Wayland compositors.
- Expand Intel, AMD, and NVIDIA coverage.
- Improve installation, recovery, and first-run setup.
- Add more community-testable game profiles and calibration workflows.
- Improve contributor documentation and reproducible compatibility reports.
- Support additional Android package formats where the install path can remain explicit and safe.

## Later

- Signed beta releases and packages for major Linux distribution families.
- Community-maintained control profiles.
- Broader gamepad support.
- More daemon-native input/session ownership.
- Better compatibility reporting and automated diagnostics.

## Non-goals

Wroid is not intended to hide root, modify games to bypass integrity checks, evade anti-cheat systems, or implement protection bypasses.

For implementation-level status and completed milestones, see the [full engineering roadmap](docs/roadmap.md).
