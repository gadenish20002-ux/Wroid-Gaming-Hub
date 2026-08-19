## What changed

Describe the change and the problem it solves.

## Why

Explain why this belongs in Wroid and any design trade-offs.

## Validation

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-features`
- [ ] `cargo build --workspace`
- [ ] Hardware / Waydroid validation performed when the change requires it

Add the exact commands, environment, and observed results below:

```text

```

## Safety and lifecycle

If this change touches input capture, the privileged helper, daemon IPC, Waydroid lifecycle, process ownership, or temporary files, describe:

- privilege boundary changes;
- failure behavior;
- cleanup / rollback behavior;
- how stale state or crashed processes are handled.

If not applicable, write `N/A`.

## Compatibility impact

Note any Linux distribution, compositor, GPU/driver, Waydroid version, or Android package behavior that may be affected.

## Checklist

- [ ] The PR is focused and avoids unrelated refactoring.
- [ ] User-facing behavior is documented where necessary.
- [ ] No credentials, tokens, private logs, or personal data are included.
- [ ] The change does not add anti-cheat, integrity-check, or root-detection bypass behavior.
