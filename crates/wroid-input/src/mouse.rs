#![allow(dead_code)]

//! Relative mouse capture and normalization for future mouse-aim bindings.
//!
//! The production path will feed these normalized host events into profile v2
//! `mouse_move` / `mouse_aim` bindings through the daemon-managed runtime. This
//! module intentionally owns only evdev access and host-event normalization.

use std::io;
use std::path::{Path, PathBuf};

use evdev::{Device, EventSummary, InputEvent, KeyCode, RelativeAxisCode};
use thiserror::Error;

const REQUIRED_RELATIVE_AXES: [(RelativeAxisCode, &str); 2] = [
    (RelativeAxisCode::REL_X, "REL_X"),
    (RelativeAxisCode::REL_Y, "REL_Y"),
];

/// Profile-visible mouse buttons supported by the relative mouse capture path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Side,
    Extra,
}

impl MouseButton {
    pub const fn profile_name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Middle => "middle",
            Self::Side => "side",
            Self::Extra => "extra",
        }
    }
}

/// Linux mouse-button lifecycle after evdev values have been normalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonTransition {
    Pressed,
    Released,
    Repeated,
}

/// One profile-visible physical mouse-button event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseButtonEvent {
    pub button: MouseButton,
    pub transition: MouseButtonTransition,
}

impl MouseButtonEvent {
    pub const fn new(button: MouseButton, transition: MouseButtonTransition) -> Self {
        Self { button, transition }
    }
}

/// Relative pointer movement measured in host evdev units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RelativeMouseMotion {
    pub dx: i32,
    pub dy: i32,
}

impl RelativeMouseMotion {
    pub const fn new(dx: i32, dy: i32) -> Self {
        Self { dx, dy }
    }

    pub const fn is_zero(self) -> bool {
        self.dx == 0 && self.dy == 0
    }

    pub fn add_saturating(self, other: Self) -> Self {
        Self {
            dx: self.dx.saturating_add(other.dx),
            dy: self.dy.saturating_add(other.dy),
        }
    }

    pub fn scaled(self, sensitivity: f64) -> Self {
        if !sensitivity.is_finite() || sensitivity <= 0.0 {
            return self;
        }

        Self {
            dx: scale_axis(self.dx, sensitivity),
            dy: scale_axis(self.dy, sensitivity),
        }
    }
}

/// Relative wheel movement measured in host evdev units.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MouseWheel {
    pub vertical: i32,
    pub horizontal: i32,
}

impl MouseWheel {
    pub const fn new(vertical: i32, horizontal: i32) -> Self {
        Self {
            vertical,
            horizontal,
        }
    }

    pub const fn is_zero(self) -> bool {
        self.vertical == 0 && self.horizontal == 0
    }
}

/// One normalized host mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseEvent {
    Motion(RelativeMouseMotion),
    Wheel(MouseWheel),
    Button(MouseButtonEvent),
}

#[derive(Debug, Error)]
pub enum MouseDeviceError {
    #[error("failed to open mouse device {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("input device {path} does not report relative axes")]
    MissingRelativeAxisCapabilities { path: PathBuf },
    #[error("input device {path} is missing required relative axes: {axes}")]
    MissingRequiredRelativeAxes { path: PathBuf, axes: String },
    #[error("input device {path} does not report mouse buttons")]
    MissingButtonCapabilities { path: PathBuf },
    #[error("input device {path} is missing the primary left mouse button")]
    MissingPrimaryButton { path: PathBuf },
    #[error("failed to grab mouse device {path}: {source}")]
    Grab {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to release mouse device {path}: {source}")]
    Ungrab {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to configure mouse device {path}: {source}")]
    Configure {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read mouse device {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Blocking evdev relative mouse reader with optional exclusive grab.
///
/// Exclusive grab is opt-in. It is useful for gaming mode because relative
/// movement should not leak into the desktop while Wroid owns focus, but it is
/// disruptive for diagnostics and should be enabled only after the selected
/// device has been validated.
pub struct EvdevMouse {
    device: Device,
    path: PathBuf,
    name: String,
    grabbed: bool,
}

impl EvdevMouse {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, MouseDeviceError> {
        let path = path.as_ref().to_path_buf();
        let device = Device::open(&path).map_err(|source| MouseDeviceError::Open {
            path: path.clone(),
            source,
        })?;
        let name = device.name().unwrap_or("Unnamed evdev mouse").to_owned();

        let axes = device.supported_relative_axes().ok_or_else(|| {
            MouseDeviceError::MissingRelativeAxisCapabilities { path: path.clone() }
        })?;
        let missing_axes = REQUIRED_RELATIVE_AXES
            .iter()
            .filter_map(|(code, label)| (!axes.contains(*code)).then_some(*label))
            .collect::<Vec<_>>();
        if !missing_axes.is_empty() {
            return Err(MouseDeviceError::MissingRequiredRelativeAxes {
                path,
                axes: missing_axes.join(", "),
            });
        }

        let buttons = device
            .supported_keys()
            .ok_or_else(|| MouseDeviceError::MissingButtonCapabilities { path: path.clone() })?;
        if !buttons.contains(KeyCode::BTN_LEFT) {
            return Err(MouseDeviceError::MissingPrimaryButton { path });
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

    pub fn set_nonblocking(&mut self, nonblocking: bool) -> Result<(), MouseDeviceError> {
        self.device
            .set_nonblocking(nonblocking)
            .map_err(|source| MouseDeviceError::Configure {
                path: self.path.clone(),
                source,
            })
    }

    pub fn grab(&mut self) -> Result<(), MouseDeviceError> {
        if self.grabbed {
            return Ok(());
        }

        self.device
            .grab()
            .map_err(|source| MouseDeviceError::Grab {
                path: self.path.clone(),
                source,
            })?;
        self.grabbed = true;
        Ok(())
    }

    pub fn ungrab(&mut self) -> Result<(), MouseDeviceError> {
        if !self.grabbed {
            return Ok(());
        }

        self.device
            .ungrab()
            .map_err(|source| MouseDeviceError::Ungrab {
                path: self.path.clone(),
                source,
            })?;
        self.grabbed = false;
        Ok(())
    }

    /// Blocks until at least one kernel event is available and returns only
    /// normalized mouse events unless nonblocking mode was configured by the
    /// caller.
    pub fn next_events(&mut self) -> Result<Vec<MouseEvent>, MouseDeviceError> {
        let events = self
            .device
            .fetch_events()
            .map_err(|source| MouseDeviceError::Read {
                path: self.path.clone(),
                source,
            })?;
        Ok(events.filter_map(normalize_mouse_input_event).collect())
    }
}

impl Drop for EvdevMouse {
    fn drop(&mut self) {
        if self.grabbed {
            let _ = self.device.ungrab();
        }
    }
}

pub fn normalize_mouse_input_event(event: InputEvent) -> Option<MouseEvent> {
    match event.destructure() {
        EventSummary::RelativeAxis(_, code, value) => normalize_relative_axis(code, value),
        EventSummary::Key(_, code, value) => normalize_button_event(code, value),
        _ => None,
    }
}

fn normalize_relative_axis(code: RelativeAxisCode, value: i32) -> Option<MouseEvent> {
    match code {
        RelativeAxisCode::REL_X if value != 0 => {
            Some(MouseEvent::Motion(RelativeMouseMotion::new(value, 0)))
        }
        RelativeAxisCode::REL_Y if value != 0 => {
            Some(MouseEvent::Motion(RelativeMouseMotion::new(0, value)))
        }
        RelativeAxisCode::REL_WHEEL if value != 0 => {
            Some(MouseEvent::Wheel(MouseWheel::new(value, 0)))
        }
        RelativeAxisCode::REL_HWHEEL if value != 0 => {
            Some(MouseEvent::Wheel(MouseWheel::new(0, value)))
        }
        _ => None,
    }
}

fn normalize_button_event(code: KeyCode, value: i32) -> Option<MouseEvent> {
    let button = match code {
        KeyCode::BTN_LEFT => MouseButton::Left,
        KeyCode::BTN_RIGHT => MouseButton::Right,
        KeyCode::BTN_MIDDLE => MouseButton::Middle,
        KeyCode::BTN_SIDE => MouseButton::Side,
        KeyCode::BTN_EXTRA => MouseButton::Extra,
        _ => return None,
    };
    let transition = match value {
        0 => MouseButtonTransition::Released,
        1 => MouseButtonTransition::Pressed,
        2 => MouseButtonTransition::Repeated,
        _ => return None,
    };

    Some(MouseEvent::Button(MouseButtonEvent::new(
        button, transition,
    )))
}

fn scale_axis(value: i32, sensitivity: f64) -> i32 {
    let scaled = f64::from(value) * sensitivity;
    if !scaled.is_finite() {
        return value;
    }

    scaled
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_names_match_profile_v2_strings() {
        assert_eq!(MouseButton::Left.profile_name(), "left");
        assert_eq!(MouseButton::Right.profile_name(), "right");
        assert_eq!(MouseButton::Middle.profile_name(), "middle");
        assert_eq!(MouseButton::Side.profile_name(), "side");
        assert_eq!(MouseButton::Extra.profile_name(), "extra");
    }

    #[test]
    fn relative_axes_become_normalized_motion_or_wheel_events() {
        assert_eq!(
            normalize_relative_axis(RelativeAxisCode::REL_X, 12),
            Some(MouseEvent::Motion(RelativeMouseMotion::new(12, 0)))
        );
        assert_eq!(
            normalize_relative_axis(RelativeAxisCode::REL_Y, -7),
            Some(MouseEvent::Motion(RelativeMouseMotion::new(0, -7)))
        );
        assert_eq!(
            normalize_relative_axis(RelativeAxisCode::REL_WHEEL, 1),
            Some(MouseEvent::Wheel(MouseWheel::new(1, 0)))
        );
        assert_eq!(normalize_relative_axis(RelativeAxisCode::REL_X, 0), None);
    }

    #[test]
    fn mouse_button_values_become_transitions() {
        assert_eq!(
            normalize_button_event(KeyCode::BTN_LEFT, 1),
            Some(MouseEvent::Button(MouseButtonEvent::new(
                MouseButton::Left,
                MouseButtonTransition::Pressed
            )))
        );
        assert_eq!(
            normalize_button_event(KeyCode::BTN_RIGHT, 0),
            Some(MouseEvent::Button(MouseButtonEvent::new(
                MouseButton::Right,
                MouseButtonTransition::Released
            )))
        );
        assert_eq!(normalize_button_event(KeyCode::KEY_W, 1), None);
        assert_eq!(normalize_button_event(KeyCode::BTN_LEFT, 9), None);
    }

    #[test]
    fn motion_accumulation_and_sensitivity_are_saturating() {
        let motion = RelativeMouseMotion::new(10, -4)
            .add_saturating(RelativeMouseMotion::new(5, -6))
            .scaled(1.5);

        assert_eq!(motion, RelativeMouseMotion::new(23, -15));
        assert_eq!(
            RelativeMouseMotion::new(1, 2).scaled(0.0),
            RelativeMouseMotion::new(1, 2)
        );
    }
}
