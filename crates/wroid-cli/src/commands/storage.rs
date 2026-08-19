use std::env;
use std::ffi::CString;
use std::fs::{self, File};
use std::mem::MaybeUninit;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};

const WAYDROID_PROP: &str = "/var/lib/waydroid/waydroid.prop";
const FULL_DECK_RECOMMENDED_BYTES: u64 = 40 * 1024 * 1024 * 1024;
const CRITICAL_AVAILABLE_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const BTRFS_SUPER_MAGIC: libc::c_long = 0x9123_683e;
const FS_IOC_GETFLAGS: libc::c_ulong = 0x8008_6601;
const FS_NOCOW_FL: libc::c_long = 0x0080_0000;
type FilesystemFlags = libc::c_long;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyOnWriteState {
    NotBtrfs,
    Disabled,
    Enabled,
    Unknown,
}

impl CopyOnWriteState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::NotBtrfs => "not_btrfs",
            Self::Disabled => "disabled",
            Self::Enabled => "enabled",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StorageReport {
    path: PathBuf,
    total_bytes: u64,
    available_bytes: u64,
    copy_on_write: CopyOnWriteState,
    health: &'static str,
    message: String,
}

impl StorageReport {
    pub(crate) fn probe() -> Result<Self> {
        let path = waydroid_data_path()?;
        let (total_bytes, available_bytes) = filesystem_capacity(&path)?;
        let copy_on_write = copy_on_write_state(&path);
        let (health, message) = classify_storage(available_bytes, copy_on_write);
        Ok(Self {
            path,
            total_bytes,
            available_bytes,
            copy_on_write,
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
            "copyOnWrite": self.copy_on_write.as_str(),
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

fn copy_on_write_state(path: &Path) -> CopyOnWriteState {
    let Ok(encoded) = CString::new(path.as_os_str().as_bytes()) else {
        return CopyOnWriteState::Unknown;
    };
    let mut stats = MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: encoded is NUL-terminated and stats points to writable storage.
    if unsafe { libc::statfs(encoded.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return CopyOnWriteState::Unknown;
    }
    // SAFETY: statfs initialized stats after returning success.
    let stats = unsafe { stats.assume_init() };
    if stats.f_type != BTRFS_SUPER_MAGIC {
        return CopyOnWriteState::NotBtrfs;
    }

    let Ok(directory) = File::open(path) else {
        return CopyOnWriteState::Unknown;
    };
    let mut flags: FilesystemFlags = 0;
    // SAFETY: directory is a live read-only descriptor and flags points to a
    // writable long expected by Linux _IOR('f', 1, long).
    if unsafe { libc::ioctl(directory.as_raw_fd(), FS_IOC_GETFLAGS, &mut flags) } != 0 {
        return CopyOnWriteState::Unknown;
    }
    if flags & FS_NOCOW_FL != 0 {
        CopyOnWriteState::Disabled
    } else {
        CopyOnWriteState::Enabled
    }
}

fn saturating_u64(value: u128) -> u64 {
    value.min(u64::MAX as u128) as u64
}

fn classify_storage(
    available_bytes: u64,
    copy_on_write: CopyOnWriteState,
) -> (&'static str, String) {
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
    } else if copy_on_write == CopyOnWriteState::Enabled {
        (
            "warning",
            "Btrfs copy-on-write is enabled for Waydroid data; Android cold starts may stall under write-heavy I/O"
                .to_owned(),
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

    const GIB: u64 = 1024 * 1024 * 1024;

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
            classify_storage(7 * 1024 * 1024 * 1024, CopyOnWriteState::Unknown).0,
            "critical"
        );
        assert_eq!(
            classify_storage(20 * 1024 * 1024 * 1024, CopyOnWriteState::Unknown).0,
            "warning"
        );
        assert_eq!(
            classify_storage(50 * 1024 * 1024 * 1024, CopyOnWriteState::Unknown).0,
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

    #[test]
    fn capacity_warnings_precede_btrfs_cow_warning() {
        assert_eq!(
            classify_storage(7 * GIB, CopyOnWriteState::Enabled).0,
            "critical"
        );
        assert_eq!(
            classify_storage(20 * GIB, CopyOnWriteState::Enabled).0,
            "warning"
        );
    }

    #[test]
    fn healthy_capacity_warns_only_for_btrfs_cow() {
        assert_eq!(
            classify_storage(50 * GIB, CopyOnWriteState::Enabled).0,
            "warning"
        );
        assert_eq!(
            classify_storage(50 * GIB, CopyOnWriteState::Disabled).0,
            "ready"
        );
        assert_eq!(
            classify_storage(50 * GIB, CopyOnWriteState::NotBtrfs).0,
            "ready"
        );
        assert_eq!(
            classify_storage(50 * GIB, CopyOnWriteState::Unknown).0,
            "ready"
        );
    }

    #[test]
    fn storage_json_exposes_copy_on_write_state() {
        let report = StorageReport {
            path: PathBuf::from("/srv/android/data"),
            total_bytes: 100 * GIB,
            available_bytes: 50 * GIB,
            copy_on_write: CopyOnWriteState::Enabled,
            health: "warning",
            message: "Btrfs copy-on-write is enabled".to_owned(),
        };

        assert_eq!(report.as_json()["copyOnWrite"], "enabled");
    }

    #[test]
    fn getflags_buffer_matches_linux_ioctl_abi() {
        const IOC_SIZE_SHIFT: u32 = 16;
        const IOC_SIZE_MASK: libc::c_ulong = 0x3fff;
        let encoded_size = (FS_IOC_GETFLAGS >> IOC_SIZE_SHIFT) & IOC_SIZE_MASK;

        assert_eq!(
            encoded_size as usize,
            std::mem::size_of::<FilesystemFlags>()
        );
    }
}
