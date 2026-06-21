//! Low-latency host input capture and normalization.
//!
//! This crate owns physical input device access and converts Linux evdev
//! events into backend-independent runtime actions. It does not know about
//! Waydroid lifecycle, Android packages, GUI toolkits, or profile storage.

use std::io;
use std::path::{Path, PathBuf};

use evdev::{Device, EventSummary, InputEvent, KeyCode};
use thiserror::Error;
use wroid_runtime::DirectionalInput;

const REQUIRED_KEYS: [(KeyCode, &str); 5] = [
    (KeyCode::KEY_W, "W"),
    (KeyCode::KEY_A, "A"),
    (KeyCode::KEY_S, "S"),
    (KeyCode::KEY_D, "D"),
    (KeyCode::KEY_ESC, "Esc"),
];

/// Logical keys used by the first keyboard-to-joystick runtime slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementKey {
    Up,
    Left,
    Down,
    Right,
    Exit,
}

/// Linux key lifecycle after evdev values have been normalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTransition {
    Pressed,
    Released,
    Repeated,
}

/// One relevant physical keyboard event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub key: MovementKey,
    pub transition: KeyTransition,
}

impl KeyboardEvent {
    pub const fn new(key: MovementKey, transition: KeyTransition) -> Self {
        Self { key, transition }
    }
}

/// Result of applying a keyboard event to the movement state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardAction {
    DirectionChanged(DirectionalInput),
    ExitRequested,
    Ignored,
}

/// Stateful WASD normalizer.
///
/// Duplicate presses, duplicate releases, and kernel auto-repeat events do not
/// create runtime updates. Opposing keys remain represented independently; the
/// virtual joystick runtime decides that opposing directions cancel per axis.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectionalKeyState {
    current: DirectionalInput,
}

impl DirectionalKeyState {
    pub const fn current(&self) -> DirectionalInput {
        self.current
    }

    pub fn apply(&mut self, event: KeyboardEvent) -> KeyboardAction {
        if event.key == MovementKey::Exit {
            return match event.transition {
                KeyTransition::Pressed => KeyboardAction::ExitRequested,
                KeyTransition::Released | KeyTransition::Repeated => KeyboardAction::Ignored,
            };
        }

        let pressed = match event.transition {
            KeyTransition::Pressed => true,
            KeyTransition::Released => false,
            KeyTransition::Repeated => return KeyboardAction::Ignored,
        };

        let target = match event.key {
            MovementKey::Up => &mut self.current.up,
            MovementKey::Left => &mut self.current.left,
            MovementKey::Down => &mut self.current.down,
            MovementKey::Right => &mut self.current.right,
            MovementKey::Exit => unreachable!("exit was handled above"),
        };

        if *target == pressed {
            return KeyboardAction::Ignored;
        }

        *target = pressed;
        KeyboardAction::DirectionChanged(self.current)
    }

    /// Releases every logical movement key.
    ///
    /// Returns `Some(neutral)` when state changed, allowing callers to issue one
    /// final joystick release during focus loss or controlled shutdown.
    pub fn release_all(&mut self) -> Option<DirectionalInput> {
        if self.current == DirectionalInput::default() {
            return None;
        }

        self.current = DirectionalInput::default();
        Some(self.current)
    }
}

#[derive(Debug, Error)]
pub enum KeyboardDeviceError {
    #[error("failed to open keyboard device {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("input device {path} does not report keyboard keys")]
    MissingKeyCapabilities { path: PathBuf },
    #[error("input device {path} is missing required keys: {keys}")]
    MissingRequiredKeys { path: PathBuf, keys: String },
    #[error("failed to grab keyboard device {path}: {source}")]
    Grab {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to release keyboard device {path}: {source}")]
    Ungrab {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to configure keyboard device {path}: {source}")]
    Configure {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read keyboard device {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Blocking evdev keyboard reader with optional exclusive grab.
///
/// Exclusive grab is intentionally opt-in because it prevents the compositor,
/// terminal, and other clients from receiving events from the selected device.
/// Closing the file descriptor releases the kernel grab even after abnormal
/// process termination.
pub struct EvdevKeyboard {
    device: Device,
    path: PathBuf,
    name: String,
    grabbed: bool,
}

impl EvdevKeyboard {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, KeyboardDeviceError> {
        let path = path.as_ref().to_path_buf();
        let device = Device::open(&path).map_err(|source| KeyboardDeviceError::Open {
            path: path.clone(),
            source,
        })?;
        let name = device.name().unwrap_or("Unnamed evdev device").to_owned();

        let supported = device
            .supported_keys()
            .ok_or_else(|| KeyboardDeviceError::MissingKeyCapabilities { path: path.clone() })?;
        let missing = REQUIRED_KEYS
            .iter()
            .filter_map(|(code, label)| (!supported.contains(*code)).then_some(*label))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(KeyboardDeviceError::MissingRequiredKeys {
                path,
                keys: missing.join(", "),
            });
        }

        Ok(Self {
            device,
            path,
            name,
            grabbed: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn is_grabbed(&self) -> bool {
        self.grabbed
    }

    pub fn set_nonblocking(&mut self, nonblocking: bool) -> Result<(), KeyboardDeviceError> {
        self.device
            .set_nonblocking(nonblocking)
            .map_err(|source| KeyboardDeviceError::Configure {
                path: self.path.clone(),
                source,
            })
    }

    pub fn grab(&mut self) -> Result<(), KeyboardDeviceError> {
        if self.grabbed {
            return Ok(());
        }

        self.device
            .grab()
            .map_err(|source| KeyboardDeviceError::Grab {
                path: self.path.clone(),
                source,
            })?;
        self.grabbed = true;
        Ok(())
    }

    pub fn ungrab(&mut self) -> Result<(), KeyboardDeviceError> {
        if !self.grabbed {
            return Ok(());
        }

        self.device
            .ungrab()
            .map_err(|source| KeyboardDeviceError::Ungrab {
                path: self.path.clone(),
                source,
            })?;
        self.grabbed = false;
        Ok(())
    }

    /// Blocks until at least one kernel event is available and returns only the
    /// movement events relevant to the current runtime slice unless the device
    /// was configured as nonblocking by the caller.
    pub fn next_events(&mut self) -> Result<Vec<KeyboardEvent>, KeyboardDeviceError> {
        let events = self
            .device
            .fetch_events()
            .map_err(|source| KeyboardDeviceError::Read {
                path: self.path.clone(),
                source,
            })?;
        Ok(events.filter_map(normalize_input_event).collect())
    }
}

impl Drop for EvdevKeyboard {
    fn drop(&mut self) {
        if self.grabbed {
            let _ = self.device.ungrab();
        }
    }
}

fn normalize_input_event(event: InputEvent) -> Option<KeyboardEvent> {
    let EventSummary::Key(_, code, value) = event.destructure() else {
        return None;
    };

    let key = match code {
        KeyCode::KEY_W => MovementKey::Up,
        KeyCode::KEY_A => MovementKey::Left,
        KeyCode::KEY_S => MovementKey::Down,
        KeyCode::KEY_D => MovementKey::Right,
        KeyCode::KEY_ESC => MovementKey::Exit,
        _ => return None,
    };
    let transition = match value {
        0 => KeyTransition::Released,
        1 => KeyTransition::Pressed,
        2 => KeyTransition::Repeated,
        _ => return None,
    };

    Some(KeyboardEvent::new(key, transition))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(key: MovementKey, transition: KeyTransition) -> KeyboardEvent {
        KeyboardEvent::new(key, transition)
    }

    #[test]
    fn press_and_release_update_direction_once() {
        let mut state = DirectionalKeyState::default();

        assert_eq!(
            state.apply(event(MovementKey::Up, KeyTransition::Pressed)),
            KeyboardAction::DirectionChanged(DirectionalInput::new(true, false, false, false))
        );
        assert_eq!(
            state.apply(event(MovementKey::Up, KeyTransition::Pressed)),
            KeyboardAction::Ignored
        );
        assert_eq!(
            state.apply(event(MovementKey::Up, KeyTransition::Released)),
            KeyboardAction::DirectionChanged(DirectionalInput::default())
        );
    }

    #[test]
    fn repeat_events_do_not_create_runtime_updates() {
        let mut state = DirectionalKeyState::default();

        assert_eq!(
            state.apply(event(MovementKey::Right, KeyTransition::Repeated)),
            KeyboardAction::Ignored
        );
        assert_eq!(state.current(), DirectionalInput::default());
    }

    #[test]
    fn opposing_keys_are_tracked_independently() {
        let mut state = DirectionalKeyState::default();
        state.apply(event(MovementKey::Left, KeyTransition::Pressed));

        assert_eq!(
            state.apply(event(MovementKey::Right, KeyTransition::Pressed)),
            KeyboardAction::DirectionChanged(DirectionalInput::new(false, true, false, true))
        );
        assert_eq!(
            state.apply(event(MovementKey::Left, KeyTransition::Released)),
            KeyboardAction::DirectionChanged(DirectionalInput::new(false, false, false, true))
        );
    }

    #[test]
    fn escape_requests_exit_only_on_press() {
        let mut state = DirectionalKeyState::default();

        assert_eq!(
            state.apply(event(MovementKey::Exit, KeyTransition::Pressed)),
            KeyboardAction::ExitRequested
        );
        assert_eq!(
            state.apply(event(MovementKey::Exit, KeyTransition::Released)),
            KeyboardAction::Ignored
        );
    }

    #[test]
    fn release_all_returns_one_neutral_update() {
        let mut state = DirectionalKeyState::default();
        state.apply(event(MovementKey::Down, KeyTransition::Pressed));

        assert_eq!(state.release_all(), Some(DirectionalInput::default()));
        assert_eq!(state.release_all(), None);
    }
}
