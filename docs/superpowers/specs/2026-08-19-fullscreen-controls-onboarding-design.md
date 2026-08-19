# Fullscreen Gameplay And Controls Onboarding Design

## Goal

Make a Hub-launched Waydroid game open as a real fullscreen gaming surface at
the selected render preset, and make the shipped keyboard/mouse map usable
without reading project documentation.

## Evidence And Root Cause

The live Standoff 2 window is `waydroid.com.axlebolt.standoff2` with a fixed
1280x720 surface on a 1920x1080 output. KWin accepts maximize/fullscreen state,
but Waydroid keeps the Android surface at 1280x720; the compositor therefore
moves it to `(0, 0)` instead of scaling it. Wroid's current KWin focus script
also matches only the exact class `waydroid`, so package windows are reported
as unfocused and gameplay input can remain released.

## Chosen Approach

Normal game launches use nested Gamescope when `/usr/bin/gamescope` is
available. Android keeps rendering at the selected 1280x720, 1600x900, or
1920x1080 preset. Gamescope presents that surface fullscreen and scales with
`fit` plus FSR, preserving aspect ratio and the Speed preset's GPU benefit.
Gamescope exposes a Wayland socket to the child `waydroid session start` and
forces the Android child surface to its nested display.

If Gamescope is not installed, Wroid retains the existing direct Waydroid
session and reports that fullscreen scaling is unavailable. Calibration and
non-game Android actions stay direct and windowed so Controls Studio can
capture the game window.

Alternatives rejected for this slice:

- Force Android to 1920x1080 and set KWin fullscreen: simple, but turns the
  Speed preset into Quality and wastes GPU time.
- Set KWin geometry on a low-resolution Waydroid window: verified ineffective
  because the Waydroid surface has a fixed size.

## Session And Focus Behavior

`DesktopWaydroidSession` gains an explicit presentation mode. Fullscreen mode
starts Gamescope with the selected render width/height as its nested size,
fullscreen output, Wayland client support, forced child fullscreen, aspect-fit
scaling, and FSR. Direct mode preserves the existing command path.

The KWin relay recognizes both `waydroid` and `waydroid.<package>` identities,
plus the Gamescope presentation window owned by a Wroid session. It keeps the
existing rule: keyboard and mouse are captured only while the game surface is
focused. F12 releases or reacquires them; Ctrl+Esc stops the session.

The Hub launch response and session log state whether Gamescope fullscreen or
direct fallback was selected. A missing Gamescope binary is not a launch
blocker.

## Controls Onboarding

Controls Studio adds a compact setup strip above the Android surface:

1. **Capture game window** — starts the existing live alignment flow.
2. **Place controls** — tells the user to drag shipped markers onto the HUD and
   select a marker to bind a key in the inspector.
3. **Test bindings** — runs the existing local input preview without sending
   Android events.
4. **Save and play** — saves the profile and closes the studio.

The strip derives completion from existing state and never invents a second
profile format. It remains visible but compact after calibration, so the flow
can be repeated. Existing advanced layers and inspector controls remain
available.

For the selected game, Hub shows a concise default-map summary and in-game
escape keys beside the launch action. Standoff 2 starts with WASD, mouse aim,
LMB fire, RMB aim, R reload, Space jump, C crouch, 1/2 weapons, F action, Tab
mouse-aim toggle, F12 capture release, and Ctrl+Esc stop.

## Error Handling

- Missing Gamescope: use direct Waydroid and expose the fallback in the result.
- Gamescope starts but exits before Android becomes ready: fail the launch with
  its captured output; do not silently restart into a different presentation.
- Unsupported desktop focus tracking: keep the existing manual F12 fallback.
- Live capture denied: keep the profile unchanged and leave step one pending.
- Invalid profile edits: retain existing validation and prevent save/play.

## Testing

- Rust unit tests cover exact Gamescope arguments, direct fallback selection,
  and package-specific Waydroid focus matching.
- JavaScript tests cover setup-step derivation and default control summary.
- Existing workspace tests, formatting, Clippy, and JS tests remain green.
- Live verification launches Standoff at 1280x720 on the 1920x1080 DP-2 output,
  confirms a 1920x1080 fullscreen Gamescope window, verifies focus transitions,
  and opens Controls Studio to check the setup strip.

## Scope

This slice does not add gamepad support, change anti-cheat compatibility,
modify Android root state, or redesign the profile schema.
