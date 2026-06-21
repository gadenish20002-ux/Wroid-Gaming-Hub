# Host Relative Mouse Input

This implementation slice adds the first evdev relative-mouse capture path for the gaming runtime. It is a host-side foundation for future profile v2 `mouse_move` to `mouse_aim` execution; it does not yet move Android aim by itself.

## What is implemented

- `crates/wroid-input/src/mouse.rs` opens a selected Linux evdev mouse device.
- The device is validated for `REL_X` and `REL_Y` relative axes and a primary left button.
- Relative x/y motion, vertical/horizontal wheel deltas, and primary mouse buttons are normalized into stable Rust events.
- Optional exclusive grab is available, with automatic release when the reader is dropped.
- `wroid-mouse-capture` prints normalized events for hardware validation.

## Diagnostic usage

Build the workspace, then run the diagnostic binary against the mouse event node selected by the user:

```sh
cargo build --workspace
cargo run -p wroid-input --bin wroid-mouse-capture -- /dev/input/event7 --max-events 20
```

Use exclusive grab only when you intentionally want Wroid to own the selected mouse while testing:

```sh
cargo run -p wroid-input --bin wroid-mouse-capture -- --grab /dev/input/event7
```

On many Linux systems `/dev/input/event*` access requires the user to be in the `input` group, an ACL from the desktop session, or running the diagnostic with elevated privileges.

## Production direction

The next step is not to send shell `input swipe` commands. The production path should be:

```text
EvdevMouse
  -> normalized mouse motion
  -> profile v2 mouse_aim region + sensitivity
  -> runtime touch aim state machine
  -> TouchEngine frame
  -> persistent uinput Type-B injector
  -> Waydroid Android input stack
```

The daemon/helper boundary should own evdev leases and grabs. The GUI should only request focus ownership and configure profile bindings; it should not open physical input devices directly.

## Safety and compatibility

This feature is intended for transparent user-controlled input mapping. It does not attempt to hide automation, bypass integrity checks, or evade anti-cheat systems. Games with unsupported input or integrity requirements should be reported as limited compatibility rather than silently worked around.
