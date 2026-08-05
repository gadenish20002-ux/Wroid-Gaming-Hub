use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

const PREFERENCES_SCHEMA_VERSION: u16 = 1;
const DEFAULT_RESOLUTION: &str = "1600x900";
const MAX_DEVICE_PATH_BYTES: usize = 4_096;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UserPreferences {
    #[serde(default = "preferences_schema_version")]
    pub(crate) schema_version: u16,
    #[serde(default = "default_resolution")]
    pub(crate) resolution: String,
    #[serde(default)]
    pub(crate) keyboard: Option<String>,
    #[serde(default)]
    pub(crate) mouse: Option<String>,
    #[serde(default = "default_game_mode")]
    pub(crate) game_mode: bool,
}

impl Default for UserPreferences {
    fn default() -> Self {
        Self {
            schema_version: PREFERENCES_SCHEMA_VERSION,
            resolution: DEFAULT_RESOLUTION.to_owned(),
            keyboard: None,
            mouse: None,
            game_mode: default_game_mode(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreferencesPatch {
    pub(crate) resolution: Option<String>,
    pub(crate) keyboard: Option<Option<String>>,
    pub(crate) mouse: Option<Option<String>>,
    pub(crate) game_mode: Option<bool>,
}

pub(crate) fn load_default() -> Result<UserPreferences> {
    load_from_path(&preferences_path()?)
}

pub(crate) fn update_default(body: &[u8]) -> Result<UserPreferences> {
    let patch: PreferencesPatch =
        serde_json::from_slice(body).context("preferences request must be valid JSON")?;
    update_at_path(&preferences_path()?, patch)
}

fn preferences_path() -> Result<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
        .context("XDG_CONFIG_HOME and HOME are unavailable for Wroid preferences")?;
    Ok(config_home.join("wroid").join("preferences.json"))
}

fn load_from_path(path: &Path) -> Result<UserPreferences> {
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(UserPreferences::default());
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read preferences {}", path.display()));
        }
    };
    let preferences: UserPreferences = serde_json::from_slice(&data)
        .with_context(|| format!("invalid Wroid preferences {}", path.display()))?;
    validate(&preferences)?;
    Ok(preferences)
}

fn update_at_path(path: &Path, patch: PreferencesPatch) -> Result<UserPreferences> {
    let directory = path.parent().context("preferences path has no parent")?;
    fs::create_dir_all(directory).with_context(|| {
        format!(
            "failed to create preferences directory {}",
            directory.display()
        )
    })?;
    let lock_path = directory.join("preferences.lock");
    let lock = secure_file(&lock_path)?;
    lock.lock()
        .with_context(|| format!("failed to lock preferences {}", lock_path.display()))?;

    let mut preferences = load_from_path(path)?;
    if let Some(resolution) = patch.resolution {
        preferences.resolution = resolution;
    }
    if let Some(keyboard) = patch.keyboard {
        preferences.keyboard = keyboard;
    }
    if let Some(mouse) = patch.mouse {
        preferences.mouse = mouse;
    }
    if let Some(game_mode) = patch.game_mode {
        preferences.game_mode = game_mode;
    }
    validate(&preferences)?;
    save_at_path(path, &preferences)?;
    Ok(preferences)
}

fn validate(preferences: &UserPreferences) -> Result<()> {
    if preferences.schema_version != PREFERENCES_SCHEMA_VERSION {
        bail!(
            "preferences schemaVersion must be {PREFERENCES_SCHEMA_VERSION}, got {}",
            preferences.schema_version
        );
    }
    if !matches!(
        preferences.resolution.as_str(),
        "1280x720" | "1600x900" | "1920x1080"
    ) {
        bail!("unsupported session resolution {}", preferences.resolution);
    }
    validate_device_path("keyboard", preferences.keyboard.as_deref())?;
    validate_device_path("mouse", preferences.mouse.as_deref())?;
    Ok(())
}

fn validate_device_path(kind: &str, path: Option<&str>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if path.len() > MAX_DEVICE_PATH_BYTES
        || !path.starts_with("/dev/input/")
        || path.chars().any(char::is_control)
    {
        bail!("invalid saved {kind} device path");
    }
    Ok(())
}

fn save_at_path(path: &Path, preferences: &UserPreferences) -> Result<()> {
    let directory = path.parent().context("preferences path has no parent")?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(
        ".preferences-{}-{sequence}.tmp",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        let mut data = serde_json::to_vec_pretty(preferences)?;
        data.push(b'\n');
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
            .with_context(|| format!("failed to replace preferences {}", path.display()))?;
        File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn secure_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))
}

const fn preferences_schema_version() -> u16 {
    PREFERENCES_SCHEMA_VERSION
}

fn default_resolution() -> String {
    DEFAULT_RESOLUTION.to_owned()
}

const fn default_game_mode() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn missing_preferences_use_safe_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = load_from_path(&directory.path().join("preferences.json")).unwrap();
        assert_eq!(preferences, UserPreferences::default());
        assert!(preferences.game_mode);
    }

    #[test]
    fn legacy_preferences_enable_game_mode_by_default() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        fs::write(&path, br#"{"schemaVersion":1,"resolution":"1280x720"}"#).unwrap();

        let preferences = load_from_path(&path).unwrap();

        assert!(preferences.game_mode);
    }

    #[test]
    fn preference_updates_are_persisted_with_private_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let updated = update_at_path(
            &path,
            PreferencesPatch {
                resolution: Some("1280x720".to_owned()),
                keyboard: Some(Some("/dev/input/by-id/gaming-event-kbd".to_owned())),
                mouse: Some(None),
                game_mode: None,
            },
        )
        .unwrap();

        assert_eq!(updated.resolution, "1280x720");
        assert_eq!(
            updated.keyboard.as_deref(),
            Some("/dev/input/by-id/gaming-event-kbd")
        );
        assert_eq!(load_from_path(&path).unwrap(), updated);
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn invalid_update_does_not_replace_existing_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let existing = update_at_path(
            &path,
            PreferencesPatch {
                resolution: Some("1920x1080".to_owned()),
                ..PreferencesPatch::default()
            },
        )
        .unwrap();

        assert!(update_at_path(
            &path,
            PreferencesPatch {
                resolution: Some("1024x768".to_owned()),
                ..PreferencesPatch::default()
            }
        )
        .is_err());
        assert_eq!(load_from_path(&path).unwrap(), existing);
    }

    #[test]
    fn json_patch_changes_one_field_without_resetting_the_others() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        update_at_path(
            &path,
            PreferencesPatch {
                resolution: Some("1920x1080".to_owned()),
                keyboard: Some(Some("/dev/input/by-id/first-event-kbd".to_owned())),
                mouse: Some(Some("/dev/input/by-id/first-event-mouse".to_owned())),
                game_mode: None,
            },
        )
        .unwrap();
        let patch: PreferencesPatch =
            serde_json::from_slice(br#"{"keyboard":"/dev/input/by-id/second-event-kbd"}"#).unwrap();

        let updated = update_at_path(&path, patch).unwrap();

        assert_eq!(updated.resolution, "1920x1080");
        assert_eq!(
            updated.keyboard.as_deref(),
            Some("/dev/input/by-id/second-event-kbd")
        );
        assert_eq!(
            updated.mouse.as_deref(),
            Some("/dev/input/by-id/first-event-mouse")
        );
    }

    #[test]
    fn game_mode_patch_persists_without_resetting_other_preferences() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        update_at_path(
            &path,
            PreferencesPatch {
                resolution: Some("1920x1080".to_owned()),
                keyboard: Some(Some("/dev/input/by-id/first-event-kbd".to_owned())),
                ..PreferencesPatch::default()
            },
        )
        .unwrap();
        let patch: PreferencesPatch = serde_json::from_slice(br#"{"gameMode":false}"#).unwrap();

        let updated = update_at_path(&path, patch).unwrap();

        assert!(!updated.game_mode);
        assert_eq!(updated.resolution, "1920x1080");
        assert_eq!(
            updated.keyboard.as_deref(),
            Some("/dev/input/by-id/first-event-kbd")
        );
        assert!(!load_from_path(&path).unwrap().game_mode);
    }
}
