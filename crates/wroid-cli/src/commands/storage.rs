use std::env;
use std::ffi::CString;
use std::fs;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

const WAYDROID_PROP: &str = "/var/lib/waydroid/waydroid.prop";
const FULL_DECK_RECOMMENDED_BYTES: u64 = 40 * 1024 * 1024 * 1024;
const CRITICAL_AVAILABLE_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageReport {
    path: PathBuf,
    total_bytes: u64,
    available_bytes: u64,
    health: &'static str,
    message: String,
}

impl StorageReport {
    pub(crate) fn probe() -> Result<Self> {
        let path = waydroid_data_path()?;
        let (total_bytes, available_bytes) = filesystem_capacity(&path)?;
        let (health, message) = classify_available_storage(available_bytes);
        Ok(Self {
            path,
            total_bytes,
            available_bytes,
            health,
            message,
        })
    }

    pub(crate) fn as_json(&self) -> Value {
        json!({
            "health": self.health,
            "path": self.path,
            "totalBytes": self.total_bytes,
            "availableBytes": self.available_bytes,
            "usedRatio": if self.total_bytes == 0 {
                0.0
            } else {
                1.0 - self.available_bytes as f64 / self.total_bytes as f64
            },
            "recommendedBytes": FULL_DECK_RECOMMENDED_BYTES,
            "message": self.message,
        })
    }
}

pub(crate) fn storage_json() -> Value {
    match StorageReport::probe() {
        Ok(report) => report.as_json(),
        Err(error) => json!({
            "health": "unknown",
            "path": null,
            "totalBytes": null,
            "availableBytes": null,
            "usedRatio": null,
            "recommendedBytes": FULL_DECK_RECOMMENDED_BYTES,
            "message": format!("Waydroid game storage could not be inspected: {error:#}"),
        }),
    }
}

fn waydroid_data_path() -> Result<PathBuf> {
    if let Ok(properties) = fs::read_to_string(WAYDROID_PROP) {
        if let Some(path) = host_data_path(&properties) {
            return Ok(path);
        }
    }
    let home = env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .context("HOME is unavailable and Waydroid does not publish waydroid.host_data_path")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("waydroid")
        .join("data"))
}

fn host_data_path(properties: &str) -> Option<PathBuf> {
    properties.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == "waydroid.host_data_path")
            .then(|| value.trim())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn filesystem_capacity(path: &Path) -> Result<(u64, u64)> {
    let encoded = CString::new(path.as_os_str().as_bytes())
        .with_context(|| format!("Waydroid data path contains a NUL byte: {}", path.display()))?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(encoded.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to inspect filesystem for {}", path.display()));
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = stats.f_frsize as u128;
    let total = block_size.saturating_mul(stats.f_blocks as u128);
    let available = block_size.saturating_mul(stats.f_bavail as u128);
    Ok((saturating_u64(total), saturating_u64(available)))
}

fn saturating_u64(value: u128) -> u64 {
    value.min(u64::MAX as u128) as u64
}

fn classify_available_storage(available_bytes: u64) -> (&'static str, String) {
    if available_bytes < CRITICAL_AVAILABLE_BYTES {
        (
            "critical",
            "Less than 8 GiB is available; large game installs and resource updates may fail"
                .to_owned(),
        )
    } else if available_bytes < FULL_DECK_RECOMMENDED_BYTES {
        (
            "warning",
            "40 GiB free is recommended before installing the complete four-game deck".to_owned(),
        )
    } else {
        (
            "ready",
            "Enough host storage is available for the four-game starter deck".to_owned(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_waydroid_host_data_path() {
        let properties = "ro.hardware.egl=mesa\nwaydroid.host_data_path=/srv/android/data\n";
        assert_eq!(
            host_data_path(properties),
            Some(PathBuf::from("/srv/android/data"))
        );
    }

    #[test]
    fn storage_thresholds_distinguish_critical_warning_and_ready() {
        assert_eq!(
            classify_available_storage(7 * 1024 * 1024 * 1024).0,
            "critical"
        );
        assert_eq!(
            classify_available_storage(20 * 1024 * 1024 * 1024).0,
            "warning"
        );
        assert_eq!(
            classify_available_storage(50 * 1024 * 1024 * 1024).0,
            "ready"
        );
    }

    #[test]
    fn filesystem_probe_reports_capacity_for_existing_directory() {
        let directory = tempfile::tempdir().unwrap();
        let (total, available) = filesystem_capacity(directory.path()).unwrap();
        assert!(total > 0);
        assert!(available <= total);
    }
}
