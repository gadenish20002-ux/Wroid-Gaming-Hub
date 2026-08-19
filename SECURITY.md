# Security Policy

Wroid Gaming Hub interacts with Linux input devices, Waydroid container configuration, local IPC, and a narrowly scoped privileged helper. Security reports are taken seriously, especially when they involve privilege escalation, device access, process identity, helper installation, IPC authentication, filesystem permissions, or cleanup/recovery behavior.

## Reporting a vulnerability

Please do **not** open a public GitHub issue for a suspected security vulnerability.

When GitHub private vulnerability reporting is enabled for this repository, use the repository's **Security → Report a vulnerability** flow. Until then, contact the maintainer privately through the GitHub profile associated with this repository and provide only enough information to establish a private reporting channel.

A useful report includes:

- affected commit/version;
- Linux distribution and kernel;
- Waydroid version when relevant;
- a clear description of the security boundary that can be crossed;
- reproducible steps or a minimal proof of concept;
- expected versus observed permissions/behavior;
- any suggested mitigation, if known.

Please avoid including unrelated credentials, personal data, or third-party secrets.

## Security boundaries

Wroid aims to keep the Hub, Controls Studio, daemon, and gameplay worker unprivileged. Privileged operations are intentionally restricted to the installed helper and fixed, typed operations required for the temporary Waydroid input bridge.

Security-sensitive areas include:

- privileged helper installation and release matching;
- Polkit authorization;
- `/dev/input`, evdev, and uinput access;
- temporary Waydroid/LXC bridge configuration;
- daemon/client authentication and Unix-socket permissions;
- process identity and session ownership;
- temporary files, logs, uploaded APKs, and state permissions;
- rollback and cleanup after crashes or interrupted launches.

## Out of scope

Wroid does not aim to bypass anti-cheat, device-integrity checks, application protections, or Android root detection. Reports or feature requests whose goal is protection evasion are outside the project's scope.

## Supported versions

Wroid is currently in active early-stage development. Security fixes are applied to the current development branch rather than maintained across a long-term stable release series.
