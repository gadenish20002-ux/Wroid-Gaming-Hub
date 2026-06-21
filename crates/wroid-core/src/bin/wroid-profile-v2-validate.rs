use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use wroid_core::{Point, Resolution};

const PROFILE_SCHEMA_VERSION: u16 = 2;
const EPSILON: f64 = 0.000_000_001;

fn main() -> Result<(), Box<dyn Error>> {
    let Some(options) = Options::parse(env::args().skip(1))? else {
        print_usage();
        return Ok(());
    };

    let profile = ProfileV2::load_from_path(&options.path)?;
    match profile.validate() {
        Ok(()) => {
            println!(
                "Profile v{} OK: {} ({}) orientation={:?} with {} binding(s)",
                profile.schema_version,
                profile.name,
                profile.package_name,
                profile.orientation,
                profile.bindings.len()
            );
            if let Some(resolution) = options.materialize {
                print_materialized_bindings(&profile, resolution);
            }
            Ok(())
        }
        Err(error) => {
            eprintln!("Profile v2 validation failed:");
            for item in &error.errors {
                eprintln!("  - {item}");
            }
            Err(Box::new(error))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ProfileV2 {
    #[serde(default = "default_schema_version")]
    schema_version: u16,
    name: String,
    package_name: String,
    #[serde(default)]
    orientation: Orientation,
    #[serde(default)]
    bindings: Vec<BindingV2>,
}

impl ProfileV2 {
    fn load_from_path(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let path = path.as_ref();
        let data = fs::read_to_string(path)?;
        serde_json::from_str(&data).map_err(|source| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "failed to parse {} as profile v2 JSON: {source}",
                    path.display()
                ),
            )
            .into()
        })
    }

    fn validate(&self) -> Result<(), ProfileV2ValidationError> {
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
enum Orientation {
    #[default]
    Landscape,
    Portrait,
    Any,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BindingV2 {
    name: String,
    input: InputV2,
    action: ActionV2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum InputV2 {
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
enum ActionV2 {
    Tap {
        point: NormalizedPoint,
    },
    VirtualJoystick {
        center: NormalizedPoint,
        radius: f64,
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
enum JoystickMode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct NormalizedPoint {
    x: f64,
    y: f64,
}

impl NormalizedPoint {
    fn materialize(self, resolution: Resolution) -> Point {
        Point {
            x: materialize_axis(self.x, resolution.width),
            y: materialize_axis(self.y, resolution.height),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
struct NormalizedRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileV2ValidationError {
    errors: Vec<String>,
}

impl std::fmt::Display for ProfileV2ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} validation error(s)", self.errors.len())
    }
}

impl Error for ProfileV2ValidationError {}

#[derive(Debug)]
struct Options {
    path: PathBuf,
    materialize: Option<Resolution>,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, Box<dyn Error>> {
        let mut path = None;
        let mut materialize = None;
        let mut args = args.into_iter();

        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--materialize" => {
                    let width: u32 = parse_next(&mut args, "--materialize width")?;
                    let height: u32 = parse_next(&mut args, "--materialize height")?;
                    if width == 0 || height == 0 {
                        return Err(invalid_input("materialized resolution must be non-zero"));
                    }
                    materialize = Some(Resolution { width, height });
                }
                value if value.starts_with("--") => {
                    return Err(invalid_input(format!("unknown option: {value}")));
                }
                value => {
                    if path.replace(PathBuf::from(value)).is_some() {
                        return Err(invalid_input("only one profile path is supported"));
                    }
                }
            }
        }

        let path = path.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing profile path; pass a profile-v2 JSON file",
            )
        })?;
        Ok(Some(Self { path, materialize }))
    }
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

fn print_materialized_bindings(profile: &ProfileV2, resolution: Resolution) {
    println!("Materialized coordinates for {resolution}:");
    for binding in &profile.bindings {
        print_materialized_action(&binding.name, &binding.action, resolution, 1);
    }
}

fn print_materialized_action(label: &str, action: &ActionV2, resolution: Resolution, depth: usize) {
    let indent = "  ".repeat(depth);
    match action {
        ActionV2::Tap { point } => {
            println!("{indent}{label}: tap {}", point.materialize(resolution));
        }
        ActionV2::VirtualJoystick {
            center,
            radius,
            mode,
            reaffirm_ms,
        } => {
            let center = center.materialize(resolution);
            let radius_px = materialize_radius(*radius, resolution);
            println!(
                "{indent}{label}: virtual_joystick center={center} radius={radius_px}px mode={mode:?} reaffirm_ms={reaffirm_ms:?}"
            );
        }
        ActionV2::MouseAim {
            region,
            sensitivity,
        } => {
            let origin = NormalizedPoint {
                x: region.x,
                y: region.y,
            }
            .materialize(resolution);
            let size = Point {
                x: materialize_axis(region.w, resolution.width),
                y: materialize_axis(region.h, resolution.height),
            };
            println!(
                "{indent}{label}: mouse_aim origin={origin} size={},{} sensitivity={sensitivity}",
                size.x, size.y
            );
        }
        ActionV2::Macro { steps } => {
            println!("{indent}{label}: macro with {} step(s)", steps.len());
            for (index, step) in steps.iter().enumerate() {
                print_materialized_action(&format!("step[{index}]"), step, resolution, depth + 1);
            }
        }
    }
}

fn materialize_axis(value: f64, limit: u32) -> u32 {
    if limit == 0 {
        return 0;
    }
    let maximum = limit - 1;
    ((value.clamp(0.0, 1.0) * f64::from(maximum)).round() as u32).min(maximum)
}

fn materialize_radius(value: f64, resolution: Resolution) -> u32 {
    let base = f64::from(resolution.width.min(resolution.height));
    (value.clamp(0.0, 1.0) * base).round().max(1.0) as u32
}

fn parse_next<T>(args: &mut impl Iterator<Item = String>, label: &str) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    let value = args
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("missing {label}")))?;
    value.parse::<T>().map_err(|source| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {label} '{value}': {source}"),
        )
        .into()
    })
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

const fn default_schema_version() -> u16 {
    PROFILE_SCHEMA_VERSION
}

const fn default_sensitivity() -> f64 {
    1.0
}

fn print_usage() {
    println!("Usage: wroid-profile-v2-validate <profile-v2.json> [--materialize WIDTH HEIGHT]");
    println!("Example: cargo run -p wroid-core --bin wroid-profile-v2-validate -- profiles/examples/shooter-v2.json --materialize 1920 1080");
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
                  "action": { "kind": "virtual_joystick", "center": { "x": 0.18, "y": 0.78 }, "radius": 0.09, "mode": "hold", "reaffirm_ms": 50 }
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
    fn out_of_bounds_normalized_coordinate_fails() {
        let mut profile = valid_profile();
        profile.bindings[0].action = ActionV2::Tap {
            point: NormalizedPoint { x: 1.25, y: 0.5 },
        };

        let error = profile.validate().unwrap_err();

        assert!(error
            .errors
            .iter()
            .any(|item| item.contains("normalized x/y")));
    }

    #[test]
    fn materializes_normalized_point_to_target_resolution() {
        let point = NormalizedPoint { x: 0.5, y: 1.0 }.materialize(Resolution {
            width: 1920,
            height: 1080,
        });

        assert_eq!(point, Point { x: 960, y: 1079 });
    }
}
