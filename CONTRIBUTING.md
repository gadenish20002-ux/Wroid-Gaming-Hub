# Contributing to Wroid Gaming Hub

Thanks for taking an interest in Wroid Gaming Hub. The project is still in active early-stage development, so bug reports, hardware compatibility results, documentation fixes, game profiles, and focused code contributions are especially useful.

## Before you start

Wroid currently targets Linux systems running Waydroid, with the primary development environment on Wayland. Some integration tests require real Linux input devices, `/dev/uinput`, and a working Waydroid installation, so not every test can run in a generic CI environment.

For Arch/CachyOS development, install the main dependencies with:

```sh
sudo pacman -S rust cargo adb waydroid gtk3 webkit2gtk-4.1
```

Package names differ between distributions.

## Build and validate

Before opening a pull request, run:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace
```

For latency-sensitive changes, also build a release binary and run the injection benchmark where supported:

```sh
cargo build --release --bin wroid-inject-latency
target/release/wroid-inject-latency --samples 20000
```

## Useful contribution areas

Contributions are welcome across the project, especially:

- testing on different Linux distributions;
- Intel, AMD, and NVIDIA GPU compatibility;
- Wayland compositor and desktop-environment compatibility;
- Waydroid lifecycle and rendering diagnostics;
- evdev/uinput input handling and latency work;
- game control profiles and calibration improvements;
- installer and packaging work;
- documentation, examples, and troubleshooting guides;
- reproducible bug reports.

See [ROADMAP.md](ROADMAP.md) and the [detailed engineering roadmap](docs/roadmap.md) for current priorities.

## Reporting bugs

Please use the bug-report issue template and include as much of the following as possible:

- Linux distribution and version;
- kernel version;
- desktop environment / compositor;
- GPU and driver;
- Waydroid version;
- Wroid commit or version;
- affected Android package/game;
- steps to reproduce;
- expected and actual behavior;
- relevant `wroid compatibility` or `wroid performance` output.

Do not post account credentials, tokens, private logs, or other secrets in an issue.

## Pull requests

Keep pull requests focused. Explain the problem being solved, the approach taken, and how the change was validated. If a change affects input capture, privilege boundaries, Waydroid lifecycle, or recovery behavior, describe the failure and cleanup paths explicitly.

Please avoid unrelated formatting or refactoring in the same pull request unless it is required by the change.

## Security-sensitive changes

Wroid includes a privileged helper and interacts with Linux input devices and Waydroid container configuration. Treat changes involving privilege boundaries, helper installation, device access, process identity, IPC, or cleanup as security-sensitive.

Do not open a public issue for a suspected vulnerability. Follow [SECURITY.md](SECURITY.md) instead.

## Scope and safety

Wroid is intended to provide transparent Linux/Waydroid gaming input and session tooling. Contributions that hide Android root, modify games to bypass integrity checks, evade anti-cheat systems, or otherwise implement protection bypasses are out of scope.

## License

By contributing, you agree that your contributions will be licensed under the project's [MIT License](LICENSE).
