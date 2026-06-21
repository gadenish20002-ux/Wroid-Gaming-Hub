pub mod profile_v2;

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlProfile {
    pub name: String,
    #[serde(default)]
    pub package_name: String,
    pub resolution: Resolution,
    pub bindings: Vec<Binding>,
}

impl ControlProfile {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ProfileError> {
        let data = fs::read_to_string(path)?;
        let profile = serde_json::from_str(&data)?;
        Ok(profile)
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<(), ProfileError> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(path, data)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ProfileValidationError> {
        let mut errors = Vec::new();

        if self.name.trim().is_empty() {
            errors.push(ValidationError::EmptyProfileName);
        }

        if self.package_name.trim().is_empty() {
            errors.push(ValidationError::EmptyPackageName);
        }

        if self.resolution.width == 0 || self.resolution.height == 0 {
            errors.push(ValidationError::InvalidResolution {
                width: self.resolution.width,
                height: self.resolution.height,
            });
        }

        let mut names = HashSet::new();
        for binding in &self.bindings {
            let name = binding.name.trim();
            if name.is_empty() {
                errors.push(ValidationError::EmptyBindingName);
            } else if !names.insert(name.to_owned()) {
                errors.push(ValidationError::DuplicateBindingName {
                    name: binding.name.clone(),
                });
            }

            validate_input(&binding.input, &binding.name, &mut errors);
            validate_action(
                &binding.action,
                &binding.name,
                &self.resolution,
                &mut errors,
            );
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ProfileValidationError { errors })
        }
    }

    pub fn binding(&self, name: &str) -> Option<&Binding> {
        self.bindings.iter().find(|binding| binding.name == name)
    }

    pub fn example() -> Self {
        Self {
            name: "Shooter Basic".to_owned(),
            package_name: "com.example.shooter".to_owned(),
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
                    name: "look_right".to_owned(),
                    input: BindingInput::Key {
                        key: "d".to_owned(),
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

    pub fn scaled_to_resolution(&self, resolution: Resolution) -> Self {
        scale_profile(self, resolution)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub name: String,
    pub input: BindingInput,
    pub action: BindingAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BindingInput {
    Key {
        key: String,
    },
    KeyCluster {
        up: String,
        left: String,
        down: String,
        right: String,
    },
    MouseButton {
        button: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BindingAction {
    Tap {
        point: Point,
    },
    Swipe {
        from: Point,
        to: Point,
        duration_ms: u64,
    },
    VirtualJoystick {
        center: Point,
        radius: u32,
        tick_ms: u64,
        swipe_duration_ms: u64,
    },
    MouseAim {
        anchor: Point,
    },
    Macro {
        steps: Vec<BindingAction>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileValidationError {
    pub errors: Vec<ValidationError>,
}

impl fmt::Display for ProfileValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, error) in self.errors.iter().enumerate() {
            if index > 0 {
                write!(f, "; ")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ProfileValidationError {}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    #[error("profile name must not be empty")]
    EmptyProfileName,
    #[error("package name must not be empty")]
    EmptyPackageName,
    #[error("resolution must be non-zero, got {width}x{height}")]
    InvalidResolution { width: u32, height: u32 },
    #[error("binding name must not be empty")]
    EmptyBindingName,
    #[error("duplicate binding name: {name}")]
    DuplicateBindingName { name: String },
    #[error("binding {binding} has an empty input value")]
    EmptyInputValue { binding: String },
    #[error("binding {binding} has point {point} outside {resolution}")]
    PointOutOfBounds {
        binding: String,
        point: Point,
        resolution: Resolution,
    },
    #[error("binding {binding} has a swipe duration of zero")]
    ZeroSwipeDuration { binding: String },
    #[error("binding {binding} has a virtual joystick radius of zero")]
    ZeroJoystickRadius { binding: String },
    #[error("binding {binding} has a virtual joystick tick interval of zero")]
    ZeroJoystickTick { binding: String },
    #[error("binding {binding} has a virtual joystick swipe duration of zero")]
    ZeroJoystickSwipeDuration { binding: String },
    #[error("binding {binding} uses unsupported action kind: {kind}")]
    UnsupportedAction { binding: String, kind: String },
}

impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", self.x, self.y)
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}

pub fn scale_point(point: Point, from_resolution: Resolution, to_resolution: Resolution) -> Point {
    Point {
        x: scale_axis(point.x, from_resolution.width, to_resolution.width),
        y: scale_axis(point.y, from_resolution.height, to_resolution.height),
    }
}

pub fn scale_action(
    action: &BindingAction,
    from_resolution: Resolution,
    to_resolution: Resolution,
) -> BindingAction {
    match action {
        BindingAction::Tap { point } => BindingAction::Tap {
            point: scale_point(*point, from_resolution, to_resolution),
        },
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => BindingAction::Swipe {
            from: scale_point(*from, from_resolution, to_resolution),
            to: scale_point(*to, from_resolution, to_resolution),
            duration_ms: *duration_ms,
        },
        BindingAction::VirtualJoystick {
            center,
            radius,
            tick_ms,
            swipe_duration_ms,
        } => BindingAction::VirtualJoystick {
            center: scale_point(*center, from_resolution, to_resolution),
            radius: scale_radius_average(*radius, from_resolution, to_resolution),
            tick_ms: *tick_ms,
            swipe_duration_ms: *swipe_duration_ms,
        },
        BindingAction::MouseAim { anchor } => BindingAction::MouseAim { anchor: *anchor },
        BindingAction::Macro { steps } => BindingAction::Macro {
            steps: steps.clone(),
        },
    }
}

pub fn scale_profile(profile: &ControlProfile, resolution: Resolution) -> ControlProfile {
    let from_resolution = profile.resolution;
    let mut scaled = profile.clone();
    scaled.resolution = resolution;
    for binding in &mut scaled.bindings {
        binding.action = scale_action(&binding.action, from_resolution, resolution);
    }
    scaled
}

fn scale_axis(value: u32, from: u32, to: u32) -> u32 {
    if from == 0 || to == 0 {
        return 0;
    }

    let scaled = ((value as u128 * to as u128) + (from as u128 / 2)) / from as u128;
    let clamped = scaled.min((to - 1) as u128);
    clamped as u32
}

fn scale_radius_average(
    radius: u32,
    from_resolution: Resolution,
    to_resolution: Resolution,
) -> u32 {
    if radius == 0
        || from_resolution.width == 0
        || from_resolution.height == 0
        || to_resolution.width == 0
        || to_resolution.height == 0
    {
        return 0;
    }

    // Joystick radius is a single scalar, so use the average of the x/y scale factors.
    let numerator = radius as u128
        * ((to_resolution.width as u128 * from_resolution.height as u128)
            + (to_resolution.height as u128 * from_resolution.width as u128));
    let denominator = 2_u128 * from_resolution.width as u128 * from_resolution.height as u128;
    let scaled = (numerator + denominator / 2) / denominator;
    scaled.max(1).min(u32::MAX as u128) as u32
}

fn validate_input(input: &BindingInput, binding: &str, errors: &mut Vec<ValidationError>) {
    match input {
        BindingInput::Key { key } if key.trim().is_empty() => {
            errors.push(ValidationError::EmptyInputValue {
                binding: binding.to_owned(),
            });
        }
        BindingInput::KeyCluster {
            up,
            left,
            down,
            right,
        } if [up, left, down, right]
            .iter()
            .any(|key| key.trim().is_empty()) =>
        {
            errors.push(ValidationError::EmptyInputValue {
                binding: binding.to_owned(),
            });
        }
        BindingInput::MouseButton { button } if button.trim().is_empty() => {
            errors.push(ValidationError::EmptyInputValue {
                binding: binding.to_owned(),
            });
        }
        BindingInput::Key { .. }
        | BindingInput::KeyCluster { .. }
        | BindingInput::MouseButton { .. } => {}
    }
}

fn validate_action(
    action: &BindingAction,
    binding: &str,
    resolution: &Resolution,
    errors: &mut Vec<ValidationError>,
) {
    match action {
        BindingAction::Tap { point } => validate_point(*point, binding, *resolution, errors),
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => {
            validate_point(*from, binding, *resolution, errors);
            validate_point(*to, binding, *resolution, errors);
            if *duration_ms == 0 {
                errors.push(ValidationError::ZeroSwipeDuration {
                    binding: binding.to_owned(),
                });
            }
        }
        BindingAction::VirtualJoystick {
            center,
            radius,
            tick_ms,
            swipe_duration_ms,
        } => {
            validate_point(*center, binding, *resolution, errors);
            if *radius == 0 {
                errors.push(ValidationError::ZeroJoystickRadius {
                    binding: binding.to_owned(),
                });
            }
            if *tick_ms == 0 {
                errors.push(ValidationError::ZeroJoystickTick {
                    binding: binding.to_owned(),
                });
            }
            if *swipe_duration_ms == 0 {
                errors.push(ValidationError::ZeroJoystickSwipeDuration {
                    binding: binding.to_owned(),
                });
            }
        }
        BindingAction::MouseAim { .. } => errors.push(ValidationError::UnsupportedAction {
            binding: binding.to_owned(),
            kind: "mouse_aim".to_owned(),
        }),
        BindingAction::Macro { .. } => errors.push(ValidationError::UnsupportedAction {
            binding: binding.to_owned(),
            kind: "macro".to_owned(),
        }),
    }
}

fn validate_point(
    point: Point,
    binding: &str,
    resolution: Resolution,
    errors: &mut Vec<ValidationError>,
) {
    if point.x >= resolution.width || point.y >= resolution.height {
        errors.push(ValidationError::PointOutOfBounds {
            binding: binding.to_owned(),
            point,
            resolution,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_profile_passes_validation() {
        ControlProfile::example().validate().unwrap();
    }

    #[test]
    fn invalid_coordinates_fail_validation() {
        let mut profile = ControlProfile::example();
        profile.bindings[0].action = BindingAction::Tap {
            point: Point { x: 1920, y: 540 },
        };

        let err = profile.validate().unwrap_err();

        assert!(err
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::PointOutOfBounds { .. })));
    }

    #[test]
    fn duplicate_binding_names_fail_validation() {
        let mut profile = ControlProfile::example();
        profile.bindings[1].name = "fire".to_owned();

        let err = profile.validate().unwrap_err();

        assert!(err.errors.iter().any(|error| {
            matches!(
                error,
                ValidationError::DuplicateBindingName { name } if name == "fire"
            )
        }));
    }

    #[test]
    fn zero_swipe_duration_fails_validation() {
        let mut profile = ControlProfile::example();
        profile.bindings[1].action = BindingAction::Swipe {
            from: Point { x: 960, y: 540 },
            to: Point { x: 1260, y: 540 },
            duration_ms: 0,
        };

        let err = profile.validate().unwrap_err();

        assert!(err
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::ZeroSwipeDuration { .. })));
    }

    #[test]
    fn valid_virtual_joystick_profile_passes_validation() {
        let profile = joystick_profile();

        profile.validate().unwrap();
    }

    #[test]
    fn invalid_virtual_joystick_center_fails_validation() {
        let mut profile = joystick_profile();
        profile.bindings[0].action = BindingAction::VirtualJoystick {
            center: Point { x: 1280, y: 360 },
            radius: 120,
            tick_ms: 80,
            swipe_duration_ms: 70,
        };

        let err = profile.validate().unwrap_err();

        assert!(err
            .errors
            .iter()
            .any(|error| matches!(error, ValidationError::PointOutOfBounds { .. })));
    }

    fn joystick_profile() -> ControlProfile {
        ControlProfile {
            name: "Joystick".to_owned(),
            package_name: "com.example.shooter".to_owned(),
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
                    center: Point { x: 260, y: 560 },
                    radius: 120,
                    tick_ms: 80,
                    swipe_duration_ms: 70,
                },
            }],
        }
    }
}
