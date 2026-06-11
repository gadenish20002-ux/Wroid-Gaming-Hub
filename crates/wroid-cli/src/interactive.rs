use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use wroid_core::{Binding, BindingAction, BindingInput, ControlProfile};

use crate::backend::{execute_swipe, execute_tap, InputExecutor, SelectedInputBackend};

pub(crate) fn start_interactive_keymapper(
    input_executor: &impl InputExecutor,
    profile: &ControlProfile,
    selected_backend: SelectedInputBackend,
) -> Result<()> {
    println!("Backend: {}", selected_backend);
    println!();
    print_keyboard_bindings(profile);
    println!();
    println!("Press a mapped key to run its binding. Press Esc or Ctrl+C to exit.");
    io::stdout().flush().context("failed to flush stdout")?;

    let raw_mode = RawModeGuard::enable()?;

    loop {
        let event = event::read().context("failed to read terminal input")?;
        let Event::Key(key_event) = event else {
            continue;
        };

        if should_exit_key(key_event) {
            break;
        }

        let Some(key_name) = key_event_name(key_event) else {
            continue;
        };

        let Some(binding) = resolve_key_binding(&profile.bindings, &key_name) else {
            continue;
        };

        execute_interactive_binding(input_executor, selected_backend, binding)?;
        io::stdout().flush().context("failed to flush stdout")?;
    }

    drop(raw_mode);
    println!();
    Ok(())
}

fn print_keyboard_bindings(profile: &ControlProfile) {
    println!("Keyboard bindings:");
    let mut printed = false;

    for binding in keyboard_bindings(&profile.bindings) {
        match &binding.input {
            BindingInput::Key { key } => {
                println!("  {} -> {}", display_key(key), binding.name);
                printed = true;
            }
            BindingInput::KeyCluster {
                up,
                left,
                down,
                right,
            } => {
                println!(
                    "  {}/{}/{}/{} -> {}",
                    display_key(up),
                    display_key(left),
                    display_key(down),
                    display_key(right),
                    binding.name
                );
                printed = true;
            }
            BindingInput::MouseButton { .. } => {}
        }
    }

    if !printed {
        println!("  (none)");
    }
}

fn keyboard_bindings(bindings: &[Binding]) -> impl Iterator<Item = &Binding> {
    bindings.iter().filter(|binding| {
        matches!(
            binding.input,
            BindingInput::Key { .. } | BindingInput::KeyCluster { .. }
        )
    })
}

pub(crate) fn resolve_key_binding<'a>(bindings: &'a [Binding], key: &str) -> Option<&'a Binding> {
    let normalized_key = normalize_key(key);
    keyboard_bindings(bindings).find(|binding| match &binding.input {
        BindingInput::Key { key } => normalize_key(key) == normalized_key,
        BindingInput::KeyCluster {
            up,
            left,
            down,
            right,
        } => [up, left, down, right]
            .iter()
            .any(|key| normalize_key(key) == normalized_key),
        BindingInput::MouseButton { .. } => false,
    })
}

pub(crate) fn normalize_key(key: &str) -> String {
    key.trim().to_lowercase()
}

fn display_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.chars().count() == 1 {
        trimmed.to_uppercase()
    } else {
        trimmed.to_owned()
    }
}

fn key_event_name(key_event: KeyEvent) -> Option<String> {
    match key_event.code {
        KeyCode::Char(character) => Some(character.to_string()),
        KeyCode::Enter => Some("enter".to_owned()),
        KeyCode::Tab => Some("tab".to_owned()),
        KeyCode::BackTab => Some("backtab".to_owned()),
        KeyCode::Backspace => Some("backspace".to_owned()),
        KeyCode::Delete => Some("delete".to_owned()),
        KeyCode::Insert => Some("insert".to_owned()),
        KeyCode::Home => Some("home".to_owned()),
        KeyCode::End => Some("end".to_owned()),
        KeyCode::PageUp => Some("pageup".to_owned()),
        KeyCode::PageDown => Some("pagedown".to_owned()),
        KeyCode::Up => Some("up".to_owned()),
        KeyCode::Down => Some("down".to_owned()),
        KeyCode::Left => Some("left".to_owned()),
        KeyCode::Right => Some("right".to_owned()),
        KeyCode::F(number) => Some(format!("f{number}")),
        KeyCode::Esc
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => None,
    }
}

fn should_exit_key(key_event: KeyEvent) -> bool {
    key_event.code == KeyCode::Esc
        || (key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn execute_interactive_binding(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    binding: &Binding,
) -> Result<()> {
    match &binding.action {
        BindingAction::Tap { point } => {
            println!("\r{} -> tap {},{}", binding.name, point.x, point.y);
            execute_tap(input_executor, backend, point.x, point.y)
        }
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => {
            println!(
                "\r{} -> swipe {},{} to {},{} ({} ms)",
                binding.name, from.x, from.y, to.x, to.y, duration_ms
            );
            execute_swipe(
                input_executor,
                backend,
                from.x,
                from.y,
                to.x,
                to.y,
                *duration_ms,
            )
        }
        BindingAction::VirtualJoystick { .. } => {
            println!(
                "\r{} uses virtual_joystick; live hold execution is not implemented yet",
                binding.name
            );
            Ok(())
        }
        unsupported => {
            println!(
                "\r{} uses unsupported action kind: {}",
                binding.name,
                action_kind(unsupported)
            );
            Ok(())
        }
    }
}

pub(crate) fn action_kind(action: &BindingAction) -> &'static str {
    match action {
        BindingAction::Tap { .. } => "tap",
        BindingAction::Swipe { .. } => "swipe",
        BindingAction::VirtualJoystick { .. } => "virtual_joystick",
        BindingAction::MouseAim { .. } => "mouse_aim",
        BindingAction::Macro { .. } => "macro",
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable terminal raw mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod tests {
    use wroid_core::{Binding, BindingAction, BindingInput, ControlProfile, Point, Resolution};

    use super::*;

    #[test]
    fn resolves_key_f_to_fire_binding() {
        let profile = keyboard_test_profile();

        let binding = resolve_key_binding(&profile.bindings, "F").unwrap();

        assert_eq!(binding.name, "fire");
    }

    #[test]
    fn resolves_key_r_to_reload_binding() {
        let profile = keyboard_test_profile();

        let binding = resolve_key_binding(&profile.bindings, "R").unwrap();

        assert_eq!(binding.name, "reload");
    }

    #[test]
    fn unknown_key_returns_no_binding() {
        let profile = keyboard_test_profile();

        assert!(resolve_key_binding(&profile.bindings, "X").is_none());
    }

    #[test]
    fn non_keyboard_bindings_are_ignored_by_interactive_runner() {
        let profile = ControlProfile {
            name: "Mouse Profile".to_owned(),
            package_name: "com.example.mouse".to_owned(),
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            bindings: vec![Binding {
                name: "fire".to_owned(),
                input: BindingInput::MouseButton {
                    button: "f".to_owned(),
                },
                action: BindingAction::Tap {
                    point: Point { x: 100, y: 100 },
                },
            }],
        };

        assert!(resolve_key_binding(&profile.bindings, "F").is_none());
    }

    #[test]
    fn resolves_key_cluster_direction_to_joystick_binding() {
        let profile = ControlProfile {
            name: "Joystick Profile".to_owned(),
            package_name: "com.example.joystick".to_owned(),
            resolution: Resolution {
                width: 1280,
                height: 720,
            },
            bindings: vec![Binding {
                name: "movement".to_owned(),
                input: BindingInput::KeyCluster {
                    up: "w".to_owned(),
                    left: "a".to_owned(),
                    down: "s".to_owned(),
                    right: "d".to_owned(),
                },
                action: BindingAction::VirtualJoystick {
                    center: Point { x: 320, y: 640 },
                    radius: 120,
                    tick_ms: 80,
                    swipe_duration_ms: 70,
                },
            }],
        };

        let binding = resolve_key_binding(&profile.bindings, "W").unwrap();

        assert_eq!(binding.name, "movement");
    }

    fn keyboard_test_profile() -> ControlProfile {
        ControlProfile {
            name: "Keyboard Profile".to_owned(),
            package_name: "com.example.keyboard".to_owned(),
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            bindings: vec![
                Binding {
                    name: "fire".to_owned(),
                    input: BindingInput::Key {
                        key: "f".to_owned(),
                    },
                    action: BindingAction::Tap {
                        point: Point { x: 1640, y: 540 },
                    },
                },
                Binding {
                    name: "reload".to_owned(),
                    input: BindingInput::Key {
                        key: "r".to_owned(),
                    },
                    action: BindingAction::Tap {
                        point: Point { x: 1760, y: 900 },
                    },
                },
                Binding {
                    name: "look".to_owned(),
                    input: BindingInput::MouseButton {
                        button: "left".to_owned(),
                    },
                    action: BindingAction::Swipe {
                        from: Point { x: 960, y: 540 },
                        to: Point { x: 1260, y: 540 },
                        duration_ms: 180,
                    },
                },
            ],
        }
    }
}
