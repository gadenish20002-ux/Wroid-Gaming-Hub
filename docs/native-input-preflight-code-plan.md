# Native input preflight code plan

Implementation target: `crates/wroid-inject/src/waydroid_session.rs`.

Change `ensure_container_stopped()` so it rejects two non-clean states before the native input runner creates a uinput device or installs the temporary Waydroid bridge:

- `Container: RUNNING` -> existing stop-Waydroid error.
- `Session: RUNNING` + `Container: FROZEN` -> explicit recovery error.

Suggested helper:

```rust
fn session_state(status: &str) -> Option<&str> {
    status_field(status, "Session")
}
```

Suggested recovery text:

```text
Waydroid session is already running but the container is FROZEN.
Recover with:
  sudo target/debug/wroid-native-keyboard --cleanup
  waydroid session stop
  sudo systemctl restart waydroid-container
```

Add unit coverage for status parsing/rejection so this regression does not return.
