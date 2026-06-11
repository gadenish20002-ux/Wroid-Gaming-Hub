use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use wroid_core::{Binding, BindingAction, BindingInput, ControlProfile, Point, Resolution};

use crate::backend::InputExecutor;
use crate::cli::InputBackend;
use crate::device::detect_device_screen;
use crate::interactive::normalize_key;
use crate::output::write_stdout;
use crate::registry::{
    create_current_profile_in_registry, ensure_binding_name_available, ensure_point_in_bounds,
    import_profile_to_registry, load_validated_profile, new_empty_control_profile, parse_point_arg,
    profile_bindings_listing, profile_registry_dir, profile_registry_file_path,
    profile_registry_listing, registered_profile_bindings_listing, save_profile,
};

pub(crate) fn validate_profile(path: PathBuf) -> Result<()> {
    load_validated_profile(&path)?;
    println!("valid profile: {}", path.display());
    Ok(())
}

pub(crate) fn write_example_profile(path: PathBuf) -> Result<()> {
    let profile = ControlProfile::example();
    profile
        .validate()
        .context("built-in example profile is invalid")?;
    profile
        .save_to_path(&path)
        .with_context(|| format!("failed to write example profile {}", path.display()))?;
    println!("wrote example profile: {}", path.display());
    Ok(())
}

pub(crate) fn create_profile(
    path: PathBuf,
    name: String,
    package_name: String,
    width: u32,
    height: u32,
    force: bool,
) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            path.display()
        );
    }

    let profile = new_empty_control_profile(name, package_name, Resolution { width, height })?;
    save_profile(&profile, &path)?;
    println!("created profile: {}", path.display());
    Ok(())
}

pub(crate) fn create_profile_from_current_screen(
    input_executor: &impl InputExecutor,
    path: PathBuf,
    name: String,
    package_name: String,
    backend: InputBackend,
    force: bool,
) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            path.display()
        );
    }

    let resolution = detect_device_screen(input_executor, backend)?;
    create_profile(
        path,
        name,
        package_name,
        resolution.width,
        resolution.height,
        force,
    )
}

pub(crate) fn add_tap_binding(
    path: PathBuf,
    name: String,
    key: String,
    x: u32,
    y: u32,
) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    ensure_binding_name_available(&profile, &name)?;

    let point = Point { x, y };
    ensure_point_in_bounds(&profile, point, "tap point")?;

    profile.bindings.push(Binding {
        name: name.clone(),
        input: BindingInput::Key {
            key: normalize_key(&key),
        },
        action: BindingAction::Tap { point },
    });
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("added tap binding: {name}");
    Ok(())
}

pub(crate) fn add_swipe_binding(
    path: PathBuf,
    name: String,
    key: String,
    from: String,
    to: String,
    duration_ms: u64,
) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    ensure_binding_name_available(&profile, &name)?;

    let from = parse_point_arg(&from, "--from")?;
    let to = parse_point_arg(&to, "--to")?;
    if duration_ms == 0 {
        bail!("swipe duration must be greater than zero");
    }
    ensure_point_in_bounds(&profile, from, "--from point")?;
    ensure_point_in_bounds(&profile, to, "--to point")?;

    profile.bindings.push(Binding {
        name: name.clone(),
        input: BindingInput::Key {
            key: normalize_key(&key),
        },
        action: BindingAction::Swipe {
            from,
            to,
            duration_ms,
        },
    });
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("added swipe binding: {name}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn add_joystick_binding(
    path: PathBuf,
    name: String,
    up: String,
    left: String,
    down: String,
    right: String,
    center: String,
    radius: u32,
    tick_ms: u64,
    swipe_duration_ms: u64,
) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    ensure_binding_name_available(&profile, &name)?;

    let center = parse_point_arg(&center, "--center")?;
    if [up.as_str(), left.as_str(), down.as_str(), right.as_str()]
        .iter()
        .any(|key| key.trim().is_empty())
    {
        bail!("joystick directional keys must not be empty");
    }
    if radius == 0 {
        bail!("joystick radius must be greater than zero");
    }
    if tick_ms == 0 {
        bail!("joystick tick interval must be greater than zero");
    }
    if swipe_duration_ms == 0 {
        bail!("joystick swipe duration must be greater than zero");
    }
    ensure_point_in_bounds(&profile, center, "--center point")?;

    profile.bindings.push(Binding {
        name: name.clone(),
        input: BindingInput::KeyCluster {
            up: normalize_key(&up),
            left: normalize_key(&left),
            down: normalize_key(&down),
            right: normalize_key(&right),
        },
        action: BindingAction::VirtualJoystick {
            center,
            radius,
            tick_ms,
            swipe_duration_ms,
        },
    });
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("added joystick binding: {name}");
    Ok(())
}

pub(crate) fn remove_binding(path: PathBuf, binding_name: &str) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    let index = profile
        .bindings
        .iter()
        .position(|binding| binding.name == binding_name)
        .with_context(|| format!("binding {binding_name} not found"))?;

    let removed = profile.bindings.remove(index);
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("removed binding: {}", removed.name);
    Ok(())
}

pub(crate) fn list_bindings(profile_path: PathBuf) -> Result<()> {
    let profile = load_validated_profile(&profile_path)?;
    write_stdout(&profile_bindings_listing(&profile))
}

pub(crate) fn import_profile(
    source_path: PathBuf,
    profile_id: Option<String>,
    force: bool,
) -> Result<()> {
    let registry_dir = profile_registry_dir()?;
    let imported =
        import_profile_to_registry(&source_path, profile_id.as_deref(), force, &registry_dir)?;

    println!("Imported profile:");
    println!("  ID: {}", imported.id);
    println!("  Path: {}", imported.path.display());
    Ok(())
}

pub(crate) fn registry_create_profile_from_current_screen(
    input_executor: &impl InputExecutor,
    name: String,
    package_name: String,
    backend: InputBackend,
    profile_id: Option<String>,
    force: bool,
) -> Result<()> {
    let registry_dir = profile_registry_dir()?;
    let created = create_current_profile_in_registry(
        input_executor,
        name,
        package_name,
        backend,
        profile_id.as_deref(),
        force,
        &registry_dir,
    )?;

    println!("Created profile:");
    println!("  ID: {}", created.id);
    println!("  Path: {}", created.path.display());
    Ok(())
}

pub(crate) fn list_profiles() -> Result<()> {
    let registry_dir = profile_registry_dir()?;
    write_stdout(&profile_registry_listing(&registry_dir)?)
}

pub(crate) fn print_profile_path(profile_id: &str) -> Result<()> {
    let path = profile_registry_file_path(profile_id)?;
    println!("{}", path.display());
    Ok(())
}

pub(crate) fn show_profile(profile_id: &str) -> Result<()> {
    let registry_dir = profile_registry_dir()?;
    write_stdout(&registered_profile_bindings_listing(
        profile_id,
        &registry_dir,
    )?)
}

#[cfg(test)]
mod tests {
    use wroid_core::{Binding, BindingAction, BindingInput, ControlProfile, Point, Resolution};

    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn new_current_uses_detected_screen_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let executor = FakeInputExecutor::with_waydroid_screen(1920, 1050);

        create_profile_from_current_screen(
            &executor,
            path.clone(),
            "Android Settings".to_owned(),
            "com.android.settings".to_owned(),
            InputBackend::WaydroidShell,
            false,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(path).unwrap();
        assert_eq!(profile.name, "Android Settings");
        assert_eq!(profile.package_name, "com.android.settings");
        assert_eq!(
            profile.resolution,
            Resolution {
                width: 1920,
                height: 1050
            }
        );
        assert!(profile.bindings.is_empty());
        profile.validate().unwrap();
        assert_eq!(executor.calls(), vec![InputCall::WaydroidShellWmSize]);
    }

    #[test]
    fn creates_profile_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("profile.json");

        create_profile(
            path.clone(),
            "Test Profile".to_owned(),
            "com.example.test".to_owned(),
            1280,
            720,
            false,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(profile.name, "Test Profile");
        assert_eq!(profile.package_name, "com.example.test");
        assert_eq!(
            profile.resolution,
            Resolution {
                width: 1280,
                height: 720
            }
        );
        assert!(profile.bindings.is_empty());
        profile.validate().unwrap();
    }

    #[test]
    fn adds_tap_binding() {
        let (_dir, path) = new_empty_profile();

        add_tap_binding(path.clone(), "fire".to_owned(), "F".to_owned(), 100, 200).unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(
            profile.bindings,
            vec![Binding {
                name: "fire".to_owned(),
                input: BindingInput::Key {
                    key: "f".to_owned()
                },
                action: BindingAction::Tap {
                    point: Point { x: 100, y: 200 }
                },
            }]
        );
        profile.validate().unwrap();
    }

    #[test]
    fn adds_swipe_binding() {
        let (_dir, path) = new_empty_profile();

        add_swipe_binding(
            path.clone(),
            "look_right".to_owned(),
            "D".to_owned(),
            "300,400".to_owned(),
            "600,400".to_owned(),
            180,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(
            profile.bindings,
            vec![Binding {
                name: "look_right".to_owned(),
                input: BindingInput::Key {
                    key: "d".to_owned()
                },
                action: BindingAction::Swipe {
                    from: Point { x: 300, y: 400 },
                    to: Point { x: 600, y: 400 },
                    duration_ms: 180,
                },
            }]
        );
        profile.validate().unwrap();
    }

    #[test]
    fn adds_joystick_binding() {
        let (_dir, path) = new_empty_profile();

        add_joystick_binding(
            path.clone(),
            "movement".to_owned(),
            "W".to_owned(),
            "A".to_owned(),
            "S".to_owned(),
            "D".to_owned(),
            "320,640".to_owned(),
            120,
            80,
            70,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(
            profile.bindings,
            vec![Binding {
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
            }]
        );
        profile.validate().unwrap();
    }

    #[test]
    fn duplicate_binding_fails() {
        let (_dir, path) = new_empty_profile();
        add_tap_binding(path.clone(), "fire".to_owned(), "f".to_owned(), 100, 200).unwrap();

        let err = add_tap_binding(path, "fire".to_owned(), "g".to_owned(), 300, 400).unwrap_err();

        assert!(err.to_string().contains("binding fire already exists"));
    }

    #[test]
    fn invalid_coordinate_format_fails() {
        let (_dir, path) = new_empty_profile();

        let err = add_swipe_binding(
            path,
            "look_right".to_owned(),
            "d".to_owned(),
            "300".to_owned(),
            "600,400".to_owned(),
            180,
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--from must use x,y coordinate format"));
    }

    #[test]
    fn removes_existing_binding() {
        let (_dir, path) = new_empty_profile();
        add_tap_binding(path.clone(), "fire".to_owned(), "f".to_owned(), 100, 200).unwrap();

        remove_binding(path.clone(), "fire").unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert!(profile.bindings.is_empty());
        profile.validate().unwrap();
    }

    #[test]
    fn removing_missing_binding_fails() {
        let (_dir, path) = new_empty_profile();

        let err = remove_binding(path, "fire").unwrap_err();

        assert!(err.to_string().contains("binding fire not found"));
    }

    fn new_empty_profile() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        create_profile(
            path.clone(),
            "Test Profile".to_owned(),
            "com.example.test".to_owned(),
            1280,
            720,
            false,
        )
        .unwrap();
        (dir, path)
    }
}
