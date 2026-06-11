use std::collections::HashMap;
use std::io::{self, Write};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal;
use wroid_core::{Binding, BindingAction, BindingInput, ControlProfile};

use crate::backend::{execute_swipe, execute_tap, InputExecutor, SelectedInputBackend};

const IDLE_EVENT_POLL: Duration = Duration::from_millis(50);

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
    println!("Terminal input must stay focused; this is not global input capture yet.");
    io::stdout().flush().context("failed to flush stdout")?;

    let raw_mode = RawModeGuard::enable()?;
    let mut joysticks = JoystickRuntime::from_bindings(&profile.bindings);

    loop {
        let now = Instant::now();
        let poll_timeout = joysticks.poll_timeout(now, IDLE_EVENT_POLL);

        if event::poll(poll_timeout).context("failed to poll terminal input")? {
            let event = event::read().context("failed to read terminal input")?;
            if let Event::Key(key_event) = event {
                if should_exit_key(key_event) {
                    break;
                }

                if let Some(key_name) = key_event_name(key_event) {
                    let now = Instant::now();
                    let joystick_handled =
                        joysticks.handle_key_event(&key_name, key_event.kind, now);

                    if !joystick_handled && key_event.kind != KeyEventKind::Release {
                        if let Some(binding) = resolve_key_binding(&profile.bindings, &key_name) {
                            execute_interactive_binding(input_executor, selected_backend, binding)?;
                            io::stdout().flush().context("failed to flush stdout")?;
                        }
                    }
                }
            }
        }

        for swipe in joysticks.due_swipes(Instant::now()) {
            println!(
                "\r{} -> virtual joystick {},{} to {},{} ({} ms)",
                swipe.binding_name,
                swipe.from.x,
                swipe.from.y,
                swipe.to.x,
                swipe.to.y,
                swipe.duration_ms
            );
            execute_swipe(
                input_executor,
                selected_backend,
                swipe.from.x,
                swipe.from.y,
                swipe.to.x,
                swipe.to.y,
                swipe.duration_ms,
            )?;
            io::stdout().flush().context("failed to flush stdout")?;
        }
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
        BindingAction::VirtualJoystick { .. } => Ok(()),
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

struct RawModeGuard {
    keyboard_enhancement_pushed: bool,
}

impl RawModeGuard {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable terminal raw mode")?;
        let keyboard_enhancement_pushed = execute!(
            io::stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )
        .is_ok();
        Ok(Self {
            keyboard_enhancement_pushed,
        })
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.keyboard_enhancement_pushed {
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = terminal::disable_raw_mode();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JoystickSwipe {
    binding_name: String,
    from: wroid_core::Point,
    to: wroid_core::Point,
    duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JoystickDirection {
    Up,
    Left,
    Down,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct JoystickKeyState {
    up: bool,
    left: bool,
    down: bool,
    right: bool,
}

impl JoystickKeyState {
    fn set(&mut self, direction: JoystickDirection, is_pressed: bool) {
        match direction {
            JoystickDirection::Up => self.up = is_pressed,
            JoystickDirection::Left => self.left = is_pressed,
            JoystickDirection::Down => self.down = is_pressed,
            JoystickDirection::Right => self.right = is_pressed,
        }
    }

    fn is_active(&self) -> bool {
        self.up || self.left || self.down || self.right
    }
}

#[derive(Debug)]
struct JoystickRuntime {
    bindings: Vec<JoystickBindingRuntime>,
    key_map: HashMap<String, Vec<(usize, JoystickDirection)>>,
}

impl JoystickRuntime {
    fn from_bindings(bindings: &[Binding]) -> Self {
        let mut runtime = Self {
            bindings: Vec::new(),
            key_map: HashMap::new(),
        };

        for binding in bindings {
            let BindingInput::KeyCluster {
                up,
                left,
                down,
                right,
            } = &binding.input
            else {
                continue;
            };

            let BindingAction::VirtualJoystick {
                center,
                radius,
                tick_ms,
                swipe_duration_ms,
            } = &binding.action
            else {
                continue;
            };

            let index = runtime.bindings.len();
            runtime.bindings.push(JoystickBindingRuntime {
                name: binding.name.clone(),
                center: *center,
                radius: *radius,
                tick_ms: *tick_ms,
                swipe_duration_ms: *swipe_duration_ms,
                keys: JoystickKeyState::default(),
                next_tick: None,
            });

            runtime.add_key(up, index, JoystickDirection::Up);
            runtime.add_key(left, index, JoystickDirection::Left);
            runtime.add_key(down, index, JoystickDirection::Down);
            runtime.add_key(right, index, JoystickDirection::Right);
        }

        runtime
    }

    fn add_key(&mut self, key: &str, binding_index: usize, direction: JoystickDirection) {
        self.key_map
            .entry(normalize_key(key))
            .or_default()
            .push((binding_index, direction));
    }

    fn handle_key_event(&mut self, key: &str, kind: KeyEventKind, now: Instant) -> bool {
        let Some(mapped) = self.key_map.get(&normalize_key(key)) else {
            return false;
        };

        for (binding_index, direction) in mapped {
            let binding = &mut self.bindings[*binding_index];
            let was_active = binding.keys.is_active();
            match kind {
                KeyEventKind::Press | KeyEventKind::Repeat => binding.keys.set(*direction, true),
                KeyEventKind::Release => binding.keys.set(*direction, false),
            }
            let is_active = binding.keys.is_active();

            if is_active && !was_active {
                binding.next_tick = Some(now);
            } else if !is_active {
                binding.next_tick = None;
            }
        }

        true
    }

    fn poll_timeout(&self, now: Instant, idle_timeout: Duration) -> Duration {
        self.bindings
            .iter()
            .filter_map(|binding| binding.next_tick)
            .map(|next_tick| next_tick.saturating_duration_since(now))
            .min()
            .map(|duration| duration.min(idle_timeout))
            .unwrap_or(idle_timeout)
    }

    fn due_swipes(&mut self, now: Instant) -> Vec<JoystickSwipe> {
        let mut swipes = Vec::new();

        for binding in &mut self.bindings {
            let Some(next_tick) = binding.next_tick else {
                continue;
            };
            if next_tick > now {
                continue;
            }

            if let Some(to) = joystick_target_point(binding.center, binding.radius, binding.keys) {
                swipes.push(JoystickSwipe {
                    binding_name: binding.name.clone(),
                    from: binding.center,
                    to,
                    duration_ms: binding.swipe_duration_ms,
                });
                binding.next_tick = Some(now + Duration::from_millis(binding.tick_ms));
            } else {
                binding.next_tick = None;
            }
        }

        swipes
    }
}

#[derive(Debug)]
struct JoystickBindingRuntime {
    name: String,
    center: wroid_core::Point,
    radius: u32,
    tick_ms: u64,
    swipe_duration_ms: u64,
    keys: JoystickKeyState,
    next_tick: Option<Instant>,
}

fn joystick_target_point(
    center: wroid_core::Point,
    radius: u32,
    keys: JoystickKeyState,
) -> Option<wroid_core::Point> {
    let dx = (keys.right as i32) - (keys.left as i32);
    let dy = (keys.down as i32) - (keys.up as i32);
    if dx == 0 && dy == 0 {
        return None;
    }

    let length = ((dx * dx + dy * dy) as f64).sqrt();
    let offset_x = ((dx as f64 / length) * radius as f64).round() as i64;
    let offset_y = ((dy as f64 / length) * radius as f64).round() as i64;

    Some(wroid_core::Point {
        x: offset_axis(center.x, offset_x),
        y: offset_axis(center.y, offset_y),
    })
}

fn offset_axis(value: u32, offset: i64) -> u32 {
    let shifted = value as i64 + offset;
    if shifted <= 0 {
        0
    } else {
        shifted.min(u32::MAX as i64) as u32
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

    #[test]
    fn joystick_target_w_moves_up_by_radius() {
        let target = joystick_target_point(
            Point { x: 320, y: 640 },
            120,
            JoystickKeyState {
                up: true,
                ..JoystickKeyState::default()
            },
        );

        assert_eq!(target, Some(Point { x: 320, y: 520 }));
    }

    #[test]
    fn joystick_target_a_moves_left_by_radius() {
        let target = joystick_target_point(
            Point { x: 320, y: 640 },
            120,
            JoystickKeyState {
                left: true,
                ..JoystickKeyState::default()
            },
        );

        assert_eq!(target, Some(Point { x: 200, y: 640 }));
    }

    #[test]
    fn joystick_target_w_and_d_is_normalized_diagonal() {
        let target = joystick_target_point(
            Point { x: 320, y: 640 },
            120,
            JoystickKeyState {
                up: true,
                right: true,
                ..JoystickKeyState::default()
            },
        );

        assert_eq!(target, Some(Point { x: 405, y: 555 }));
    }

    #[test]
    fn joystick_target_without_keys_returns_no_event() {
        let target =
            joystick_target_point(Point { x: 320, y: 640 }, 120, JoystickKeyState::default());

        assert_eq!(target, None);
    }

    #[test]
    fn joystick_runtime_emits_immediately_then_on_tick() {
        let profile = joystick_test_profile();
        let mut runtime = JoystickRuntime::from_bindings(&profile.bindings);
        let now = Instant::now();

        assert!(runtime.handle_key_event("W", KeyEventKind::Press, now));

        assert_eq!(
            runtime.due_swipes(now),
            vec![JoystickSwipe {
                binding_name: "movement".to_owned(),
                from: Point { x: 320, y: 640 },
                to: Point { x: 320, y: 520 },
                duration_ms: 70,
            }]
        );
        assert!(runtime
            .due_swipes(now + Duration::from_millis(79))
            .is_empty());
        assert_eq!(
            runtime.due_swipes(now + Duration::from_millis(80)),
            vec![JoystickSwipe {
                binding_name: "movement".to_owned(),
                from: Point { x: 320, y: 640 },
                to: Point { x: 320, y: 520 },
                duration_ms: 70,
            }]
        );
    }

    #[test]
    fn joystick_runtime_stops_after_release() {
        let profile = joystick_test_profile();
        let mut runtime = JoystickRuntime::from_bindings(&profile.bindings);
        let now = Instant::now();

        assert!(runtime.handle_key_event("W", KeyEventKind::Press, now));
        assert_eq!(runtime.due_swipes(now).len(), 1);
        assert!(runtime.handle_key_event("W", KeyEventKind::Release, now));

        assert!(runtime
            .due_swipes(now + Duration::from_millis(80))
            .is_empty());
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

    fn joystick_test_profile() -> ControlProfile {
        ControlProfile {
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
        }
    }
}
