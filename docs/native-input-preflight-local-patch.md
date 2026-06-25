# Local patch: native input preflight guard

Apply this patch if the GitHub connector cannot update `waydroid_session.rs` directly.

```bash
python3 - <<'PY'
from pathlib import Path

path = Path('crates/wroid-inject/src/waydroid_session.rs')
text = path.read_text()

old = '''pub fn ensure_container_stopped() -> io::Result<()> {
    let status = waydroid_status()?;
    if container_state(&status) == Some("RUNNING") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid container is running. Stop it first with: waydroid session stop",
        ));
    }
    Ok(())
}
'''
new = '''pub fn ensure_container_stopped() -> io::Result<()> {
    let status = waydroid_status()?;
    if container_state(&status) == Some("RUNNING") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid container is running. Stop it first with: waydroid session stop",
        ));
    }
    if session_state(&status) == Some("RUNNING") && container_state(&status) == Some("FROZEN") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid session is already running but the container is FROZEN. Recover with:\n  sudo target/debug/wroid-native-keyboard --cleanup\n  waydroid session stop\n  sudo systemctl restart waydroid-container",
        ));
    }
    Ok(())
}
'''
if old not in text:
    raise SystemExit('ensure_container_stopped pattern not found')
text = text.replace(old, new, 1)

old = '''fn container_state(status: &str) -> Option<&str> {
    status_field(status, "Container")
}
'''
new = '''fn session_state(status: &str) -> Option<&str> {
    status_field(status, "Session")
}

fn container_state(status: &str) -> Option<&str> {
    status_field(status, "Container")
}
'''
if old not in text:
    raise SystemExit('container_state pattern not found')
text = text.replace(old, new, 1)

old = '''        Some("RUNNING") => false,
        _ => status_field(status, "Session") == Some("STOPPED"),
'''
new = '''        Some("RUNNING") => false,
        _ => session_state(status) == Some("STOPPED"),
'''
if old not in text:
    raise SystemExit('waydroid_is_stopped pattern not found')
text = text.replace(old, new, 1)

path.write_text(text)
PY
```

Validation:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```
