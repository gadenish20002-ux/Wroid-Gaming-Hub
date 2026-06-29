use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Point, Resolution};

pub const PROFILE_SCHEMA_VERSION: u16 = 2;
const EPSILON: f64 = 0.000_000_001;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileV2 {
    #[serde(default = "default_schema_version")]
    pub schema_version: u16,
    pub name: String,
    pub package_name: String,
    #[serde(default)]
    pub orientation: Orientation,
    #[serde(default)]
    pub bindings: Vec<BindingV2>,
}

impl ProfileV2 {
    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ProfileV2LoadError> {
        let data = fs::read_to_string(path.as_ref()).map_err(ProfileV2LoadError::Io)?;
        serde_json::from_str(&data).map_err(ProfileV2LoadError::Json)
    }

    pub fn validate(&self) -> Result<(), ProfileV2ValidationError> {
        let mut errors = Vec::new();

        if self.schema_version != PROFILE_SCHEMA_VERSION {
            errors.push(format!(
                "schema_version must be {PROFILE_SCHEMA_VERSION}, got {}",
                self.schema_version
            ));
        }
        if self.name.trim().is_empty() {
            errors.push("name must not be empty".to_owned());
        }
        if self.package_name.trim().is_empty() {
            errors.push("package_name must not be empty".to_owned());
        }

        let mut names = HashSet::new();
        for binding in &self.bindings {
            let binding_name = binding.name.trim();
            if binding_name.is_empty() {
                errors.push("binding name must not be empty".to_owned());
            } else if !names.insert(binding_name.to_owned()) {
                errors.push(format!("duplicate binding name: {}", binding.name));
            }

            validate_input(&binding.input, &binding.name, &mut errors);
            validate_action(&binding.action, &binding.name, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ProfileV2ValidationError { errors })
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Landscape,
    Portrait,
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingV2 {
    pub name: String,
    pub input: InputV2,
    pub action: ActionV2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputV2 {
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
    MouseMove,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionV2 {
    Tap {
        point: NormalizedPoint,
    },
    VirtualJoystick {
        center: NormalizedPoint,
        radius: f64,
        #[serde(default)]
        dead_zone: f64,
        #[serde(default)]
        mode: JoystickMode,
        #[serde(default)]
        reaffirm_ms: Option<u64>,
    },
    MouseAim {
        region: NormalizedRect,
        #[serde(default = "default_sensitivity")]
        sensitivity: f64,
    },
    Macro {
        steps: Vec<ActionV2>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoystickMode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizedPoint {
    pub x: f64,
    pub y: f64,
}

impl NormalizedPoint {
    pub fn materialize(self, resolution: Resolution) -> Point {
        Point {
            x: materialize_axis(self.x, resolution.width),
            y: materialize_axis(self.y, resolution.height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Error)]
pub enum ProfileV2LoadError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileV2ValidationError {
    pub errors: Vec<String>,
}

impl std::fmt::Display for ProfileV2ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} validation error(s)", self.errors.len())
    }
}

impl std::error::Error for ProfileV2ValidationError {}

pub fn materialize_axis(value: f64, limit: u32) -> u32 {
    if limit == 0 {
        return 0;
    }
    let maximum = limit - 1;
    ((value.clamp(0.0, 1.0) * f64::from(maximum)).round() as u32).min(maximum)
}

pub fn materialize_radius(value: f64, resolution: Resolution) -> u32 {
    let base = f64::from(resolution.width.min(resolution.height));
    (value.clamp(0.0, 1.0) * base).round().max(1.0) as u32
}

pub fn materialize_dead_zone(value: f64, radius: u32, resolution: Resolution) -> u32 {
    if value <= 0.0 || radius <= 1 {
        return 0;
    }

    let base = f64::from(resolution.width.min(resolution.height));
    let dead_zone = (value.clamp(0.0, 1.0) * base).round() as u32;
    dead_zone.min(radius - 1)
}

fn validate_input(input: &InputV2, binding: &str, errors: &mut Vec<String>) {
    match input {
        InputV2::Key { key } if key.trim().is_empty() => {
            errors.push(format!("binding {binding} has an empty key input"));
        }
        InputV2::KeyCluster {
            up,
            left,
            down,
            right,
        } if [up, left, down, right]
            .iter()
            .any(|value| value.trim().is_empty()) =>
        {
            errors.push(format!("binding {binding} has an empty key_cluster input"));
        }
        InputV2::MouseButton { button } if button.trim().is_empty() => {
            errors.push(format!("binding {binding} has an empty mouse button"));
        }
        InputV2::Key { .. }
        | InputV2::KeyCluster { .. }
        | InputV2::MouseButton { .. }
        | InputV2::MouseMove => {}
    }
}

fn validate_action(action: &ActionV2, binding: &str, errors: &mut Vec<String>) {
    match action {
        ActionV2::Tap { point } => {
            validate_point(*point, &format!("binding {binding} tap point"), errors);
        }
        ActionV2::VirtualJoystick {
            center,
            radius,
            dead_zone,
            reaffirm_ms,
            ..
        } => {
            validate_point(
                *center,
                &format!("binding {binding} virtual_joystick center"),
                errors,
            );
            if !radius.is_finite() || *radius <= 0.0 || *radius > 1.0 {
                errors.push(format!(
                    "binding {binding} virtual_joystick radius must be finite and within 0.0..=1.0"
                ));
            }
            if !dead_zone.is_finite() || *dead_zone < 0.0 || *dead_zone >= 1.0 {
                errors.push(format!(
                    "binding {binding} virtual_joystick dead_zone must be finite and within 0.0..1.0"
                ));
            } else if radius.is_finite() && *dead_zone >= *radius {
                errors.push(format!(
                    "binding {binding} virtual_joystick dead_zone must be smaller than radius"
                ));
            }
            if matches!(reaffirm_ms, Some(0)) {
                errors.push(format!(
                    "binding {binding} virtual_joystick reaffirm_ms must be greater than zero"
                ));
            }
        }
        ActionV2::MouseAim {
            region,
            sensitivity,
        } => {
            validate_rect(
                *region,
                &format!("binding {binding} mouse_aim region"),
                errors,
            );
            if !sensitivity.is_finite() || *sensitivity <= 0.0 {
                errors.push(format!(
                    "binding {binding} mouse_aim sensitivity must be finite and greater than zero"
                ));
            }
        }
        ActionV2::Macro { steps } => {
            if steps.is_empty() {
                errors.push(format!(
                    "binding {binding} macro must contain at least one step"
                ));
            }
            for (index, step) in steps.iter().enumerate() {
                validate_action(step, &format!("{binding}.step[{index}]"), errors);
            }
        }
    }
}

fn validate_point(point: NormalizedPoint, label: &str, errors: &mut Vec<String>) {
    if !normalized(point.x) || !normalized(point.y) {
        errors.push(format!(
            "{label} must use normalized x/y coordinates within 0.0..=1.0"
        ));
    }
}

fn validate_rect(rect: NormalizedRect, label: &str, errors: &mut Vec<String>) {
    if !normalized(rect.x)
        || !normalized(rect.y)
        || !rect.w.is_finite()
        || !rect.h.is_finite()
        || rect.w <= 0.0
        || rect.h <= 0.0
        || rect.x + rect.w > 1.0 + EPSILON
        || rect.y + rect.h > 1.0 + EPSILON
    {
        errors.push(format!(
            "{label} must stay inside the normalized viewport with positive w/h"
        ));
    }
}

fn normalized(value: f64) -> bool {
    value.is_finite() && (-EPSILON..=1.0 + EPSILON).contains(&value)
}

const fn default_schema_version() -> u16 {
    PROFILE_SCHEMA_VERSION
}

const fn default_sensitivity() -> f64 {
    1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_profile() -> ProfileV2 {
        serde_json::from_str(
            r#"
            {
              "schema_version": 2,
              "name": "Shooter v2",
              "package_name": "com.example.shooter",
              "orientation": "landscape",
              "bindings": [
                {
                  "name": "movement",
                  "input": { "kind": "key_cluster", "up": "w", "left": "a", "down": "s", "right": "d" },
                  "action": { "kind": "virtual_joystick", "center": { "x": 0.18, "y": 0.78 }, "radius": 0.09, "dead_zone": 0.02, "mode": "hold", "reaffirm_ms": 50 }
                },
                {
                  "name": "fire",
                  "input": { "kind": "mouse_button", "button": "left" },
                  "action": { "kind": "tap", "point": { "x": 0.86, "y": 0.50 } }
                }
              ]
            }
            "#,
        )
        .unwrap()
    }

    #[test]
    fn valid_profile_passes() {
        valid_profile().validate().unwrap();
    }

    #[test]
    fn materializes_normalized_joystick_values() {
        let resolution = Resolution {
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            NormalizedPoint { x: 0.18, y: 0.78 }.materialize(resolution),
            Point { x: 345, y: 842 }
        );
        assert_eq!(materialize_radius(0.09, resolution), 97);
        assert_eq!(materialize_dead_zone(0.02, 97, resolution), 22);
    }

    #[test]
    fn materialized_dead_zone_stays_smaller_than_runtime_radius() {
        let resolution = Resolution {
            width: 1920,
            height: 1080,
        };

        assert_eq!(materialize_dead_zone(0.0, 97, resolution), 0);
        assert_eq!(materialize_dead_zone(0.02, 1, resolution), 0);
        assert_eq!(materialize_dead_zone(0.09, 97, resolution), 96);
        assert_eq!(materialize_dead_zone(1.0, 97, resolution), 96);
    }

    #[test]
    fn duplicate_binding_names_fail() {
        let mut profile = valid_profile();
        profile.bindings[1].name = "movement".to_owned();

        let error = profile.validate().unwrap_err();

        assert!(error
            .errors
            .iter()
            .any(|item| item.contains("duplicate binding name")));
    }

    #[test]
    fn legacy_virtual_joystick_without_dead_zone_still_passes() {
        let profile: ProfileV2 = serde_json::from_str(
            r#"
            {
              "schema_version": 2,
              "name": "Legacy joystick",
              "package_name": "com.example.shooter",
              "bindings": [
                {
                  "name": "movement",
                  "input": { "kind": "key_cluster", "up": "w", "left": "a", "down": "s", "right": "d" },
                  "action": { "kind": "virtual_joystick", "center": { "x": 0.18, "y": 0.78 }, "radius": 0.09 }
                }
              ]
            }
            "#,
        )
        .unwrap();

        profile.validate().unwrap();
        assert!(matches!(
            profile.bindings[0].action,
            ActionV2::VirtualJoystick { dead_zone: 0.0, .. }
        ));
    }

    #[test]
    fn dead_zone_must_be_smaller_than_radius() {
        let mut profile = valid_profile();
        profile.bindings[0].action = ActionV2::VirtualJoystick {
            center: NormalizedPoint { x: 0.18, y: 0.78 },
            radius: 0.09,
            dead_zone: 0.09,
            mode: JoystickMode::Hold,
            reaffirm_ms: Some(50),
        };

        let error = profile.validate().unwrap_err();

        assert!(error
            .errors
            .iter()
            .any(|item| item.contains("dead_zone must be smaller than radius")));
    }
}
