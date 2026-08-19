# Standoff 2 Playable Slice Design

Date: 2026-08-19
Status: approved by user

## Goal

Make Standoff 2 the first end-to-end gameplay acceptance target for Wroid.
The slice starts in Hub, validates the host and Android environment, guides the
user through calibration when needed, launches a managed game session, and
proves keyboard/mouse playability and deterministic cleanup on real hardware.

The same readiness model will later be reused for PUBG Mobile, Free Fire, and
Brawl Stars.

## Scope

The slice covers:

- Standoff 2 package `com.axlebolt.standoff2` installed through Google Play;
- the existing 1280x720 performance preset;
- explicit physical keyboard and relative-mouse selection;
- the existing Controls Studio live-window calibration flow;
- the existing daemon-owned managed launch and privileged input bridge;
- WASD movement, relative mouse aim, fire, ADS, reload, jump, crouch, weapon
  selection, and interaction from the shipped Standoff 2 profile;
- root-environment detection before game teardown or input capture;
- production-session latency and cleanup evidence.

Native toolkit replacement, macros, gamepad input, protection evasion, and
support for additional games are outside this slice.

## User flow

1. Hub refreshes one readiness snapshot containing the installed package,
   graphics, ARM translation, Play Store, helper, root state, input devices,
   render target, profile, and calibration state.
2. A hard blocker keeps Play unavailable and gives one concrete remediation.
   Warnings remain visible but do not prevent a known-safe launch.
3. If the Standoff 2 calibration frame is missing, the primary setup action
   opens the installed game and Controls Studio. The user aligns the live game
   viewport and saves the frame.
4. Input self-test runs the production input path for 20 seconds without
   launching an APK. It verifies capture, mapped controls, latency reporting,
   and normal bridge restoration.
5. Play launches Standoff 2 directly through the daemon-owned session. Input is
   captured only while the Waydroid game window owns focus. F12 releases or
   reacquires capture, and Ctrl+Esc stops the session.
6. Hub reports the bounded session outcome and input/kernel latency after the
   game exits.

## Root compatibility preflight

Standoff 2 refuses an Android environment with active root. The reference host
contained Magisk Delta installed by `waydroid-extras`; the active marker was
`/var/lib/waydroid/overlay/system/etc/init/magisk`, and the Android package was
`io.github.huskydg.magisk`. Removing Magisk allowed the game to enter a match.

Compatibility probing gains a root state with three outcomes:

- `detected`: an active system overlay marker or an installed known root-manager
  package is present;
- `not_detected`: all supported active probes completed and no marker exists;
- `unknown`: required evidence could not be read.

The first implementation recognizes the proven Magisk signals only:

- active overlay directory
  `/var/lib/waydroid/overlay/system/etc/init/magisk`;
- installed package `io.github.huskydg.magisk` or `com.topjohnwu.magisk`.

The probe deliberately ignores `waydroid.host_data_path/adbroot` and stale app
data such as `data/io.github.huskydg.magisk`. Those paths can remain after a
successful uninstall and do not prove that root is active.

`detected` is an action-required compatibility finding and blocks known-game
launch before Waydroid teardown. The message tells the user to remove the root
extension using the same trusted tool that installed it. Wroid does not hide
root, modify game files, spoof integrity, or execute a general-purpose removal
command as part of gameplay.

`unknown` is a warning because an unreadable optional marker must not become a
false claim that root exists. It remains visible in CLI and Hub diagnostics.

## Components and data flow

### Compatibility report

`wroid-cli::commands::compatibility` owns the root classification because it
already combines Waydroid properties, package inventory, and target-game
readiness. `CompatibilityReport` exposes `rootAccess` in JSON and a typed
launch guard for known games.

The pure classifier consumes marker-probe results and the optional installed
package list. Filesystem discovery is a thin adapter so the classification and
false-positive rules can be unit tested without privileged fixtures.

### Hub

Hub consumes the extended compatibility JSON in its existing readiness card.
An active-root finding uses the current action-required presentation and keeps
the selected game's launch action disabled. The detail is visible without a
terminal. No new polling is added; the existing focus-aware refresh updates the
state after the user returns from a setup tool.

### Managed launch

Both Hub and direct `launch-v2` use the same compatibility guard. The guard runs
before desktop Waydroid teardown, render-size changes, bridge activation, or
physical-device capture. This prevents a predictable game rejection from
disturbing the user's Android session.

### Existing gameplay path

The control plan, daemon/worker split, uinput touchscreen, focus protection,
telemetry, and cleanup remain unchanged unless real-hardware verification
reveals a reproducible defect. Any such defect receives a separate focused
root-cause investigation and test-first fix.

## Error handling

- Active Magisk: block known-game launch with the exact detected signal and
  removal guidance.
- Root state unknown: warn, preserve the evidence gap, and allow launch.
- Missing keyboard or mouse: disable self-test and Play until an explicit,
  capability-valid `/dev/input/by-id` device is selected.
- Missing calibration: offer the combined open-game-and-Studio flow; do not
  discard or overwrite the profile.
- Runtime failure: preserve the existing bounded last-session reason and
  restore the bridge and previous Waydroid desktop state.
- Focus loss or controlled stop: cancel all active contacts and release both
  physical grabs before returning to Hub.

## Testing

Automated tests cover:

- active Magisk overlay classification;
- active Magisk package classification;
- stale Magisk app-data and `adbroot` paths not being inputs to the classifier;
- clean and unknown root states;
- JSON `rootAccess` output and action-required health;
- known-game launch rejection when active root is detected;
- custom package behavior remaining unchanged;
- Hub readiness serialization and browser rendering of the root finding;
- existing compatibility, launch, Hub JavaScript, formatting, Clippy, and
  workspace test gates.

Manual verification on the reference host covers:

1. Select the Hexgears keyboard and Logitech G403 relative mouse.
2. Save a 1280x720 Standoff 2 calibration frame.
3. Run the 20-second production input self-test.
4. Launch Standoff 2 from Hub and play one 15-minute match.
5. Verify WASD, mouse aim, fire, ADS, reload, jump, crouch, weapon selection,
   interaction, F12 release/reacquire, and Ctrl+Esc stop.
6. Confirm no stuck touches or physical grabs after focus loss and exit.
7. Confirm reader-to-inject p95 is below 5 ms and review kernel-to-inject p95.

## Acceptance criteria

- Active Magisk is reported before a known game launch and is not masked.
- A clean Waydroid installation with stale root-related data is not falsely
  blocked.
- Standoff 2 launches from Hub into a playable match at 1280x720.
- All shipped essential FPS bindings work against the calibrated HUD.
- A 15-minute match completes without Wroid-caused stuck input or recurring
  stalls.
- Reader-to-inject p95 is below 5 ms on the reference host.
- F12, focus loss, Ctrl+Esc, normal exit, and failure paths release grabs,
  cancel contacts, and restore Waydroid deterministically.
