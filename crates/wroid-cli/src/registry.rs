use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use wroid_core::{
    Binding, BindingAction, BindingInput, ControlProfile, Point, ProfileError, Resolution,
    ValidationError,
};

use crate::backend::InputExecutor;
use crate::cli::InputBackend;
use crate::device::detect_device_screen;
use crate::interactive::action_kind;

pub(crate) fn load_validated_profile(profile_path: &Path) -> Result<ControlProfile> {
    let profile = load_profile_with_context(profile_path)?;
    profile
        .validate()
        .with_context(|| format!("profile {} is invalid", profile_path.display()))?;
    Ok(profile)
}

pub(crate) fn load_play_profile(profile_path: &Path) -> Result<ControlProfile> {
    let profile = load_profile_with_context(profile_path)?;

    match profile.validate() {
        Ok(()) => Ok(profile),
        Err(error)
            if error
                .errors
                .iter()
                .all(|error| matches!(error, ValidationError::UnsupportedAction { .. })) =>
        {
            Ok(profile)
        }
        Err(error) => {
            Err(error).with_context(|| format!("profile {} is invalid", profile_path.display()))
        }
    }
}

fn load_profile_with_context(profile_path: &Path) -> Result<ControlProfile> {
    match ControlProfile::load_from_path(profile_path) {
        Ok(profile) => Ok(profile),
        Err(error) => {
            let mut context = format!("failed to load profile {}", profile_path.display());
            if let Some(hint) = profile_error_hint(&error) {
                context.push_str("\nHint: ");
                context.push_str(hint);
            }
            Err(error).context(context)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportedProfile {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
}

pub(crate) fn import_profile_to_registry(
    source_path: &Path,
    profile_id: Option<&str>,
    force: bool,
    registry_dir: &Path,
) -> Result<ImportedProfile> {
    let profile = load_validated_profile(source_path)?;
    let id = profile_id.unwrap_or(&profile.package_name);
    validate_profile_id(id)?;

    let target_path = registry_profile_file_path(registry_dir, id)?;
    if target_path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            target_path.display()
        );
    }

    save_profile(&profile, &target_path)?;
    Ok(ImportedProfile {
        id: id.to_owned(),
        path: target_path,
    })
}

pub(crate) fn create_current_profile_in_registry(
    input_executor: &impl InputExecutor,
    name: String,
    package_name: String,
    backend: InputBackend,
    profile_id: Option<&str>,
    force: bool,
    registry_dir: &Path,
) -> Result<ImportedProfile> {
    let id = profile_id.unwrap_or(&package_name).to_owned();
    validate_profile_id(&id)?;

    let target_path = registry_profile_file_path(registry_dir, &id)?;
    if target_path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            target_path.display()
        );
    }

    let resolution = detect_device_screen(input_executor, backend)?;
    let profile = new_empty_control_profile(name, package_name, resolution)?;
    save_profile(&profile, &target_path)?;
    Ok(ImportedProfile {
        id,
        path: target_path,
    })
}

pub(crate) fn export_profile_from_registry(
    profile_id: &str,
    output_path: &Path,
    force: bool,
    registry_dir: &Path,
) -> Result<PathBuf> {
    let source_path = registry_profile_file_path(registry_dir, profile_id)?;
    ensure_registry_profile_exists(profile_id, &source_path)?;
    if output_path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            output_path.display()
        );
    }

    copy_profile_json(&source_path, output_path)?;
    Ok(output_path.to_owned())
}

pub(crate) fn remove_profile_from_registry(
    profile_id: &str,
    registry_dir: &Path,
) -> Result<PathBuf> {
    let path = registry_profile_file_path(registry_dir, profile_id)?;
    ensure_registry_profile_exists(profile_id, &path)?;
    fs::remove_file(&path)
        .with_context(|| format!("failed to remove profile {}", path.display()))?;
    Ok(path)
}

pub(crate) fn rename_profile_in_registry(
    old_id: &str,
    new_id: &str,
    registry_dir: &Path,
) -> Result<(PathBuf, PathBuf)> {
    let old_path = registry_profile_file_path(registry_dir, old_id)?;
    let new_path = registry_profile_file_path(registry_dir, new_id)?;
    ensure_registry_profile_exists(old_id, &old_path)?;
    if new_path.exists() {
        bail!("profile {new_id} already exists at {}", new_path.display());
    }

    fs::rename(&old_path, &new_path).with_context(|| {
        format!(
            "failed to rename profile {} to {}",
            old_path.display(),
            new_path.display()
        )
    })?;
    Ok((old_path, new_path))
}

pub(crate) fn duplicate_profile_in_registry(
    source_id: &str,
    target_id: &str,
    registry_dir: &Path,
) -> Result<PathBuf> {
    let source_path = registry_profile_file_path(registry_dir, source_id)?;
    let target_path = registry_profile_file_path(registry_dir, target_id)?;
    ensure_registry_profile_exists(source_id, &source_path)?;
    if target_path.exists() {
        bail!(
            "profile {target_id} already exists at {}",
            target_path.display()
        );
    }

    copy_profile_json(&source_path, &target_path)?;
    Ok(target_path)
}

pub(crate) fn profile_registry_listing(registry_dir: &Path) -> Result<String> {
    if !registry_dir.exists() {
        return Ok(empty_profile_registry_message(registry_dir));
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(registry_dir)
        .with_context(|| format!("failed to read profile registry {}", registry_dir.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "failed to read an entry in profile registry {}",
                registry_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
            entries.push(path);
        }
    }
    entries.sort();

    if entries.is_empty() {
        return Ok(empty_profile_registry_message(registry_dir));
    }

    let mut output = String::new();
    for path in entries {
        let Some(profile_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            output.push_str(&format!(
                "warning: skipped profile with non-UTF-8 path: {}\n",
                path.display()
            ));
            continue;
        };

        if let Err(error) = validate_profile_id(profile_id) {
            output.push_str(&format!(
                "warning: skipped invalid profile ID from {}: {error:#}\n",
                path.display()
            ));
            continue;
        }

        match load_validated_profile(&path) {
            Ok(profile) => {
                output.push_str(&format!(
                    "{} -> {} -> {} -> {}\n",
                    profile_id, profile.name, profile.package_name, profile.resolution
                ));
            }
            Err(error) => {
                output.push_str(&format!(
                    "warning: skipped invalid profile {}: {error:#}\n",
                    path.display()
                ));
            }
        }
    }

    Ok(output)
}

fn ensure_registry_profile_exists(profile_id: &str, path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "profile {profile_id} not found in registry at {}",
            path.display()
        );
    }

    Ok(())
}

fn copy_profile_json(source_path: &Path, target_path: &Path) -> Result<()> {
    if let Some(parent) = target_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create profile directory {}", parent.display()))?;
    }

    fs::copy(source_path, target_path).with_context(|| {
        format!(
            "failed to copy profile {} to {}",
            source_path.display(),
            target_path.display()
        )
    })?;
    Ok(())
}

fn empty_profile_registry_message(registry_dir: &Path) -> String {
    format!(
        "No profiles found in {}. Import one with `wroid profile import <path>`.\n",
        registry_dir.display()
    )
}

pub(crate) fn registered_profile_bindings_listing(
    profile_id: &str,
    registry_dir: &Path,
) -> Result<String> {
    let path = registry_profile_file_path(registry_dir, profile_id)?;
    let profile = load_validated_profile(&path)?;
    Ok(profile_bindings_listing(&profile))
}

pub(crate) fn profile_registry_file_path(profile_id: &str) -> Result<PathBuf> {
    let registry_dir = profile_registry_dir()?;
    registry_profile_file_path(&registry_dir, profile_id)
}

pub(crate) fn registry_profile_file_path(registry_dir: &Path, profile_id: &str) -> Result<PathBuf> {
    validate_profile_id(profile_id)?;
    Ok(registry_dir.join(format!("{profile_id}.json")))
}

pub(crate) fn validate_profile_id(profile_id: &str) -> Result<()> {
    if profile_id.is_empty() {
        bail!("profile ID must not be empty");
    }

    if !profile_id.chars().all(is_profile_id_char) {
        bail!(
            "profile ID {profile_id:?} is invalid; use only ASCII letters, digits, dot, dash, and underscore"
        );
    }

    Ok(())
}

fn is_profile_id_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
}

pub(crate) fn profile_registry_dir() -> Result<PathBuf> {
    profile_registry_dir_from_env(
        env_path("XDG_CONFIG_HOME"),
        env_path("HOME"),
        env_value("SUDO_USER"),
        env_value("SUDO_UID"),
        effective_uid(),
        system_user_home_dir,
    )
}

pub(crate) fn profile_registry_dir_from_env(
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
    sudo_user: Option<String>,
    sudo_uid: Option<String>,
    effective_uid: u32,
    user_home_dir: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<PathBuf> {
    if let Some(xdg_config_home) = xdg_config_home.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(xdg_config_home.join("wroid").join("profiles"));
    }

    if let Some(sudo_user) =
        original_sudo_user_for_registry(effective_uid, sudo_user.as_deref(), sudo_uid.as_deref())
    {
        let home =
            user_home_dir(sudo_user).unwrap_or_else(|| PathBuf::from("/home").join(sudo_user));
        return Ok(home.join(".config").join("wroid").join("profiles"));
    }

    if let Some(home) = home.filter(|path| !path.as_os_str().is_empty()) {
        return Ok(home.join(".config").join("wroid").join("profiles"));
    }

    bail!("could not determine profile registry directory; set XDG_CONFIG_HOME or HOME")
}

fn original_sudo_user_for_registry<'a>(
    effective_uid: u32,
    sudo_user: Option<&'a str>,
    sudo_uid: Option<&str>,
) -> Option<&'a str> {
    if effective_uid != 0 {
        return None;
    }

    let sudo_uid = sudo_uid.map(str::trim).filter(|uid| !uid.is_empty())?;
    let sudo_user = sudo_user.map(str::trim).filter(|user| {
        !user.is_empty()
            && *user != "root"
            && !user.contains('/')
            && !user.contains('\\')
            && !user.contains('\0')
    })?;

    if sudo_uid == "0" {
        return None;
    }

    Some(sudo_user)
}

#[cfg(unix)]
fn system_user_home_dir(user: &str) -> Option<PathBuf> {
    fs::read_to_string("/etc/passwd")
        .ok()?
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .find_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            if name != user {
                return None;
            }

            let home = fields.nth(4)?;
            if home.is_empty() {
                None
            } else {
                Some(PathBuf::from(home))
            }
        })
}

#[cfg(not(unix))]
fn system_user_home_dir(_user: &str) -> Option<PathBuf> {
    None
}

pub(crate) fn env_value(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_path(key: &str) -> Option<PathBuf> {
    env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn save_profile(profile: &ControlProfile, path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create profile directory {}", parent.display()))?;
    }

    profile
        .save_to_path(path)
        .with_context(|| format!("failed to write profile {}", path.display()))?;
    Ok(())
}

pub(crate) fn new_empty_control_profile(
    name: String,
    package_name: String,
    resolution: Resolution,
) -> Result<ControlProfile> {
    let profile = ControlProfile {
        name,
        package_name,
        resolution,
        bindings: Vec::new(),
    };
    profile.validate().context("new profile is invalid")?;
    Ok(profile)
}

pub(crate) fn ensure_binding_name_available(profile: &ControlProfile, name: &str) -> Result<()> {
    if profile.binding(name).is_some() {
        bail!("binding {name} already exists");
    }

    Ok(())
}

pub(crate) fn ensure_point_in_bounds(
    profile: &ControlProfile,
    point: Point,
    label: &str,
) -> Result<()> {
    if point.x >= profile.resolution.width || point.y >= profile.resolution.height {
        bail!(
            "{label} {point} is outside profile resolution {}",
            profile.resolution
        );
    }

    Ok(())
}

pub(crate) fn parse_point_arg(value: &str, label: &str) -> Result<Point> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
        bail!("{label} must use x,y coordinate format");
    }

    let x = parts[0]
        .trim()
        .parse()
        .with_context(|| format!("{label} has an invalid x coordinate"))?;
    let y = parts[1]
        .trim()
        .parse()
        .with_context(|| format!("{label} has an invalid y coordinate"))?;

    Ok(Point { x, y })
}

fn profile_error_hint(error: &ProfileError) -> Option<&'static str> {
    let ProfileError::Json(error) = error else {
        return None;
    };

    let message = error.to_string();
    if message.contains("missing field `kind`") {
        Some("profile input and action objects are tagged; include a `kind` field such as `key`, `tap`, or `swipe`.")
    } else if message.contains("unknown variant") {
        Some("check `kind` values. Supported input kinds include `key` and `key_cluster`; supported action kinds include `tap`, `swipe`, and `virtual_joystick`.")
    } else if message.contains("invalid type: map, expected a sequence") {
        Some("check array fields. `bindings` must be a JSON array, and macro `steps` must be an array.")
    } else {
        None
    }
}

pub(crate) fn profile_bindings_listing(profile: &ControlProfile) -> String {
    let mut output = String::new();
    output.push_str(&format!("Profile: {}\n", profile.name));
    output.push_str(&format!("Package: {}\n", profile.package_name));
    output.push_str(&format!("Resolution: {}\n", profile.resolution));
    output.push_str("Bindings:\n");

    if profile.bindings.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for binding in &profile.bindings {
            output.push_str("  ");
            output.push_str(&binding_description(binding));
            output.push('\n');
        }
    }

    output
}

pub(crate) fn binding_description(binding: &Binding) -> String {
    format!(
        "{} -> {} -> {}",
        input_description(&binding.input),
        binding.name,
        action_description(&binding.action)
    )
}

fn input_description(input: &BindingInput) -> String {
    match input {
        BindingInput::Key { key } => key.trim().to_owned(),
        BindingInput::KeyCluster {
            up,
            left,
            down,
            right,
        } => format!(
            "key_cluster up={} left={} down={} right={}",
            up.trim(),
            left.trim(),
            down.trim(),
            right.trim()
        ),
        BindingInput::MouseButton { button } => format!("mouse_button {}", button.trim()),
    }
}

fn action_description(action: &BindingAction) -> String {
    match action {
        BindingAction::Tap { point } => format!("tap {point}"),
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => format!("swipe {from} to {to} ({duration_ms} ms)"),
        BindingAction::VirtualJoystick {
            center,
            radius,
            tick_ms,
            swipe_duration_ms,
        } => format!(
            "virtual_joystick center {center} radius {radius} tick {tick_ms} ms swipe {swipe_duration_ms} ms"
        ),
        unsupported => format!("unsupported {}", action_kind(unsupported)),
    }
}

#[cfg(unix)]
pub(crate) fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }

    unsafe { geteuid() }
}

#[cfg(not(unix))]
pub(crate) fn effective_uid() -> u32 {
    if std::process::id() == 0 {
        0
    } else {
        u32::MAX
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use wroid_core::{Binding, BindingAction, BindingInput, ControlProfile, Point, Resolution};

    use crate::commands::run::{resolve_registered_profile_and_run, RunOptions};
    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn profile_id_validation_accepts_supported_characters() {
        for profile_id in ["com.android.settings", "my-game", "game_1"] {
            validate_profile_id(profile_id).unwrap();
        }
    }

    #[test]
    fn profile_id_validation_rejects_unsafe_or_empty_values() {
        for profile_id in ["../evil", "evil/path", "", "name with spaces"] {
            let err = validate_profile_id(profile_id).unwrap_err();
            assert!(
                err.to_string().contains("profile ID"),
                "unexpected error for {profile_id:?}: {err:#}"
            );
        }
    }

    #[test]
    fn profile_registry_dir_uses_xdg_config_home_when_set() {
        let path = profile_registry_dir_from_env(
            Some(PathBuf::from("/tmp/xdg-config")),
            Some(PathBuf::from("/root")),
            Some("supergut".to_owned()),
            Some("1000".to_owned()),
            0,
            |_| Some(PathBuf::from("/home/supergut")),
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/tmp/xdg-config")
                .join("wroid")
                .join("profiles")
        );
    }

    #[test]
    fn profile_registry_dir_falls_back_to_current_user_home_config() {
        let path = profile_registry_dir_from_env(
            None,
            Some(PathBuf::from("/tmp/home")),
            None,
            None,
            1000,
            |_| None,
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/tmp/home")
                .join(".config")
                .join("wroid")
                .join("profiles")
        );
    }

    #[test]
    fn profile_registry_dir_uses_original_sudo_user_home_when_root() {
        let path = profile_registry_dir_from_env(
            None,
            Some(PathBuf::from("/root")),
            Some("supergut".to_owned()),
            Some("1000".to_owned()),
            0,
            |_| None,
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/home/supergut")
                .join(".config")
                .join("wroid")
                .join("profiles")
        );
    }

    #[test]
    fn profile_registry_dir_prefers_resolved_sudo_user_home() {
        let path = profile_registry_dir_from_env(
            None,
            Some(PathBuf::from("/root")),
            Some("alice".to_owned()),
            Some("1001".to_owned()),
            0,
            |_| Some(PathBuf::from("/var/home/alice")),
        )
        .unwrap();

        assert_eq!(
            path,
            PathBuf::from("/var/home/alice")
                .join(".config")
                .join("wroid")
                .join("profiles")
        );
    }

    #[test]
    fn run_profile_resolves_original_user_registry_path_under_sudo() {
        let registry_dir = profile_registry_dir_from_env(
            None,
            Some(PathBuf::from("/root")),
            Some("supergut".to_owned()),
            Some("1000".to_owned()),
            0,
            |_| None,
        )
        .unwrap();
        let called_path = RefCell::new(None);

        resolve_registered_profile_and_run(
            "com.android.settings",
            &registry_dir,
            InputBackend::WaydroidShell,
            RunOptions {
                launch_delay_ms: 1500,
                no_launch: false,
                scale_to_current: false,
            },
            |path, _options| {
                *called_path.borrow_mut() = Some(path);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            called_path.into_inner(),
            Some(
                PathBuf::from("/home/supergut")
                    .join(".config")
                    .join("wroid")
                    .join("profiles")
                    .join("com.android.settings.json")
            )
        );
    }

    #[test]
    fn import_profile_writes_default_id_json() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.json");
        let registry_dir = dir.path().join("registry");
        ControlProfile::example()
            .save_to_path(&source_path)
            .unwrap();

        let imported =
            import_profile_to_registry(&source_path, None, false, &registry_dir).unwrap();

        assert_eq!(imported.id, "com.example.shooter");
        assert_eq!(imported.path, registry_dir.join("com.example.shooter.json"));
        assert!(imported.path.exists());
        assert_eq!(
            ControlProfile::load_from_path(imported.path).unwrap(),
            ControlProfile::example()
        );
    }

    #[test]
    fn import_profile_refuses_overwrite_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("source.json");
        let registry_dir = dir.path().join("registry");
        ControlProfile::example()
            .save_to_path(&source_path)
            .unwrap();
        import_profile_to_registry(&source_path, None, false, &registry_dir).unwrap();

        let err = import_profile_to_registry(&source_path, None, false, &registry_dir).unwrap_err();

        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn export_profile_writes_expected_file() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        let source_path = registry_dir.join("com.example.game.json");
        let output_path = dir.path().join("exports").join("game.json");
        let source_json = valid_profile_json("Game", "com.example.game");
        fs::write(&source_path, source_json.as_bytes()).unwrap();

        let exported =
            export_profile_from_registry("com.example.game", &output_path, false, &registry_dir)
                .unwrap();

        assert_eq!(exported, output_path);
        assert_eq!(fs::read_to_string(output_path).unwrap(), source_json);
    }

    #[test]
    fn remove_profile_deletes_profile() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        let path = registry_dir.join("com.example.game.json");
        fs::write(&path, valid_profile_json("Game", "com.example.game")).unwrap();

        let removed = remove_profile_from_registry("com.example.game", &registry_dir).unwrap();

        assert_eq!(removed, path);
        assert!(!removed.exists());
    }

    #[test]
    fn rename_profile_rejects_existing_target() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        fs::write(
            registry_dir.join("com.example.source.json"),
            valid_profile_json("Source", "com.example.source"),
        )
        .unwrap();
        fs::write(
            registry_dir.join("com.example.target.json"),
            valid_profile_json("Target", "com.example.target"),
        )
        .unwrap();

        let err =
            rename_profile_in_registry("com.example.source", "com.example.target", &registry_dir)
                .unwrap_err();

        assert!(err.to_string().contains("already exists"));
        assert!(registry_dir.join("com.example.source.json").exists());
        assert!(registry_dir.join("com.example.target.json").exists());
    }

    #[test]
    fn duplicate_profile_creates_copy() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        let source_path = registry_dir.join("com.example.source.json");
        let target_path = registry_dir.join("com.example.copy.json");
        let source_json = valid_profile_json("Source", "com.example.source");
        fs::write(&source_path, source_json.as_bytes()).unwrap();

        let duplicated =
            duplicate_profile_in_registry("com.example.source", "com.example.copy", &registry_dir)
                .unwrap();

        assert_eq!(duplicated, target_path);
        assert_eq!(fs::read_to_string(target_path).unwrap(), source_json);
        assert_eq!(fs::read_to_string(source_path).unwrap(), source_json);
    }

    #[test]
    fn list_profiles_handles_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();

        let listing = profile_registry_listing(&registry_dir).unwrap();

        assert!(listing.contains("No profiles found"));
        assert!(listing.contains("wroid profile import <path>"));
    }

    #[test]
    fn list_profiles_prints_valid_profiles_and_warns_for_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        let mut profile = ControlProfile::example();
        profile.name = "Settings".to_owned();
        profile.package_name = "com.android.settings".to_owned();
        profile
            .save_to_path(registry_dir.join("com.android.settings.json"))
            .unwrap();
        fs::write(registry_dir.join("broken.json"), b"{ not json").unwrap();

        let listing = profile_registry_listing(&registry_dir).unwrap();

        assert!(listing
            .contains("com.android.settings -> Settings -> com.android.settings -> 1920x1080\n"));
        assert!(listing.contains("warning: skipped invalid profile"));
        assert!(listing.contains("broken.json"));
    }

    #[test]
    fn show_profile_resolves_profile_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        fs::create_dir_all(&registry_dir).unwrap();
        let mut profile = ControlProfile::example();
        profile.name = "Settings".to_owned();
        profile.package_name = "com.android.settings".to_owned();
        profile
            .save_to_path(registry_dir.join("com.android.settings.json"))
            .unwrap();

        let listing =
            registered_profile_bindings_listing("com.android.settings", &registry_dir).unwrap();

        assert!(listing.contains("Profile: Settings\n"));
        assert!(listing.contains("Package: com.android.settings\n"));
        assert!(listing.contains("Resolution: 1920x1080\n"));
    }

    #[test]
    fn registry_new_current_writes_detected_screen_size_to_registry() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        let executor = FakeInputExecutor::with_waydroid_screen(1920, 1050);

        let created = create_current_profile_in_registry(
            &executor,
            "Android Settings".to_owned(),
            "com.android.settings".to_owned(),
            InputBackend::WaydroidShell,
            None,
            false,
            &registry_dir,
        )
        .unwrap();

        assert_eq!(created.id, "com.android.settings");
        assert_eq!(created.path, registry_dir.join("com.android.settings.json"));
        let profile = ControlProfile::load_from_path(created.path).unwrap();
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
    fn binding_description_formats_tap_binding() {
        let binding = &ControlProfile::example().bindings[0];

        assert_eq!(binding_description(binding), "f -> fire -> tap 1640,540");
    }

    #[test]
    fn binding_description_formats_joystick_binding() {
        let binding = Binding {
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
        };

        assert_eq!(
            binding_description(&binding),
            "key_cluster up=w left=a down=s right=d -> movement -> virtual_joystick center 320,640 radius 120 tick 80 ms swipe 70 ms"
        );
    }

    #[test]
    fn binding_description_formats_swipe_binding() {
        let binding = &ControlProfile::example().bindings[2];

        assert_eq!(
            binding_description(binding),
            "d -> look_right -> swipe 960,540 to 1260,540 (180 ms)"
        );
    }

    #[test]
    fn profile_bindings_listing_includes_metadata_and_bindings() {
        let listing = profile_bindings_listing(&ControlProfile::example());

        assert_eq!(
            listing,
            "\
Profile: Shooter Basic
Package: com.example.shooter
Resolution: 1920x1080
Bindings:
  f -> fire -> tap 1640,540
  r -> reload -> tap 1760,900
  d -> look_right -> swipe 960,540 to 1260,540 (180 ms)
"
        );
    }

    #[test]
    fn parse_point_arg_rejects_invalid_coordinate_format() {
        let err = parse_point_arg("300", "--from").unwrap_err();

        assert!(err
            .to_string()
            .contains("--from must use x,y coordinate format"));
    }

    #[test]
    fn ensure_point_in_bounds_rejects_out_of_bounds_point() {
        let profile = new_empty_control_profile(
            "Test".to_owned(),
            "com.example.test".to_owned(),
            Resolution {
                width: 100,
                height: 100,
            },
        )
        .unwrap();

        let err =
            ensure_point_in_bounds(&profile, Point { x: 100, y: 99 }, "tap point").unwrap_err();

        assert!(err.to_string().contains("outside profile resolution"));
    }

    #[test]
    fn unsupported_actions_are_listed_without_validation_execution() {
        let mut profile = ControlProfile::example();
        profile.bindings[0].action = BindingAction::MouseAim {
            anchor: Point { x: 100, y: 100 },
        };

        let listing = profile_bindings_listing(&profile);

        assert!(listing.contains("unsupported mouse_aim"));
    }

    fn valid_profile_json(name: &str, package_name: &str) -> String {
        format!(
            "{{\n  \"name\": \"{name}\",\n  \"package_name\": \"{package_name}\",\n  \"resolution\": {{ \"width\": 1280, \"height\": 720 }},\n  \"bindings\": []\n}}\n"
        )
    }
}
