use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::raw::c_int;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use evdev::{BusType, Device};

use crate::{WROID_TOUCHSCREEN_NAME, WROID_TOUCHSCREEN_PRODUCT, WROID_TOUCHSCREEN_VENDOR};

pub const DEFAULT_WAYDROID_CONFIG: &str = "/var/lib/waydroid/lxc/waydroid/config";
pub const DEFAULT_WAYDROID_BRIDGE_CONFIG: &str =
    "/var/lib/waydroid/lxc/waydroid/config_wroid_input";
pub const DEFAULT_WAYDROID_BRIDGE_LOCK: &str = "/run/wroid/input-bridge.lock";

const MANAGED_HEADER: &str = "# Managed by Wroid Gaming Hub input bridge";
const LOCK_EX: c_int = 2;
const LOCK_NB: c_int = 4;
const LOCK_UN: c_int = 8;

unsafe extern "C" {
    fn flock(fd: c_int, operation: c_int) -> c_int;
}

/// An OS-owned lease for the single Waydroid input bridge.
///
/// The advisory lock is released by the kernel when the process exits, including
/// crashes and signals that do not run Rust destructors.
pub struct WaydroidBridgeLease {
    file: File,
}

impl WaydroidBridgeLease {
    pub fn acquire_default(owner: &str) -> io::Result<Self> {
        let directory = Path::new(DEFAULT_WAYDROID_BRIDGE_LOCK)
            .parent()
            .expect("default bridge lock has a parent");
        fs::create_dir_all(directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o755))?;
        Self::acquire_named(
            DEFAULT_WAYDROID_BRIDGE_LOCK,
            "the Waydroid input bridge",
            owner,
        )
    }

    pub fn acquire(path: impl AsRef<Path>, owner: &str) -> io::Result<Self> {
        Self::acquire_named(path, "the Waydroid input bridge", owner)
    }

    pub fn acquire_named(path: impl AsRef<Path>, resource: &str, owner: &str) -> io::Result<Self> {
        let path = path.as_ref();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o644)
            .open(path)?;

        // SAFETY: flock only observes the valid file descriptor owned by `file`.
        if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::WouldBlock {
                let detail = bridge_owner_metadata(path)
                    .unwrap_or_else(|_| "owner details unavailable".to_owned());
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!("another Wroid process already owns {resource} ({detail})"),
                ));
            }
            return Err(error);
        }

        let owner = owner.replace(['\n', '\r'], " ");
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "pid={}", std::process::id())?;
        writeln!(file, "owner={owner}")?;
        file.sync_data()?;
        Ok(Self { file })
    }
}

impl Drop for WaydroidBridgeLease {
    fn drop(&mut self) {
        // SAFETY: the descriptor remains valid for the duration of `drop`.
        let _ = unsafe { flock(self.file.as_raw_fd(), LOCK_UN) };
    }
}

pub fn active_default_bridge_lease_owner() -> io::Result<Option<String>> {
    active_bridge_lease_owner(DEFAULT_WAYDROID_BRIDGE_LOCK)
}

pub fn active_bridge_lease_owner(path: impl AsRef<Path>) -> io::Result<Option<String>> {
    let path = path.as_ref();
    let file = match OpenOptions::new().read(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };

    // SAFETY: flock only observes the valid file descriptor owned by `file`.
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } == 0 {
        // SAFETY: the descriptor is still valid and this process owns the lock.
        let _ = unsafe { flock(file.as_raw_fd(), LOCK_UN) };
        return Ok(None);
    }

    let error = io::Error::last_os_error();
    if error.kind() != io::ErrorKind::WouldBlock {
        return Err(error);
    }
    Ok(Some(bridge_owner_metadata(path)?))
}

fn bridge_owner_metadata(path: &Path) -> io::Result<String> {
    let metadata = fs::read_to_string(path)?;
    let pid = metadata
        .lines()
        .find_map(|line| line.trim().strip_prefix("pid="));
    let owner = metadata
        .lines()
        .find_map(|line| line.trim().strip_prefix("owner="));
    Ok(match (pid, owner) {
        (Some(pid), Some(owner)) => format!("PID {pid} · {owner}"),
        (Some(pid), None) => format!("PID {pid}"),
        (None, Some(owner)) => owner.to_owned(),
        (None, None) => "owner details unavailable".to_owned(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgroupMode {
    V1,
    V2,
}

impl CgroupMode {
    pub fn detect() -> Self {
        if Path::new("/sys/fs/cgroup/cgroup.controllers").is_file() {
            Self::V2
        } else {
            Self::V1
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceNode {
    path: PathBuf,
    major: u64,
    minor: u64,
}

impl InputDeviceNode {
    pub fn from_path(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if !path.is_absolute() || path.parent() != Some(Path::new("/dev/input")) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "input device must be an absolute /dev/input/event* path; got {}",
                    path.display()
                ),
            ));
        }

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if !file_name.starts_with("event") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("input device must be an event node; got {}", path.display()),
            ));
        }

        let metadata = fs::metadata(path)?;
        if !metadata.file_type().is_char_device() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("input device is not a character device: {}", path.display()),
            ));
        }

        let device = metadata.rdev();
        Ok(Self {
            path: path.to_path_buf(),
            major: linux_major(device),
            minor: linux_minor(device),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn major(&self) -> u64 {
        self.major
    }

    pub const fn minor(&self) -> u64 {
        self.minor
    }

    #[cfg(test)]
    fn for_test(path: impl Into<PathBuf>, major: u64, minor: u64) -> Self {
        Self {
            path: path.into(),
            major,
            minor,
        }
    }
}

pub fn validate_wroid_touchscreen_node(node: &InputDeviceNode) -> io::Result<()> {
    let event_name = node
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid event node name"))?;
    let sysfs_path = fs::canonicalize(
        Path::new("/sys/class/input")
            .join(event_name)
            .join("device"),
    )
    .map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to resolve sysfs identity for {}: {error}",
                node.path().display()
            ),
        )
    })?;
    let device = Device::open(node.path()).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to inspect candidate Wroid touchscreen {}: {error}",
                node.path().display()
            ),
        )
    })?;
    let input_id = device.input_id();
    validate_wroid_touchscreen_identity(
        device.name(),
        input_id.bus_type(),
        input_id.vendor(),
        input_id.product(),
        &sysfs_path,
    )
}

fn validate_wroid_touchscreen_identity(
    name: Option<&str>,
    bus_type: BusType,
    vendor: u16,
    product: u16,
    sysfs_path: &Path,
) -> io::Result<()> {
    let expected_sysfs = Path::new("/sys/devices/virtual/input");
    if name != Some(WROID_TOUCHSCREEN_NAME)
        || bus_type != BusType::BUS_VIRTUAL
        || vendor != WROID_TOUCHSCREEN_VENDOR
        || product != WROID_TOUCHSCREEN_PRODUCT
        || !sysfs_path.starts_with(expected_sysfs)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "refusing to bridge an input node that is not the Wroid virtual touchscreen",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaydroidBridgePaths {
    pub main_config: PathBuf,
    pub managed_config: PathBuf,
}

impl Default for WaydroidBridgePaths {
    fn default() -> Self {
        Self {
            main_config: PathBuf::from(DEFAULT_WAYDROID_CONFIG),
            managed_config: PathBuf::from(DEFAULT_WAYDROID_BRIDGE_CONFIG),
        }
    }
}

pub struct InstalledWaydroidBridge {
    paths: WaydroidBridgePaths,
    original_managed: Option<Vec<u8>>,
    original_managed_mode: Option<u32>,
    include_added: bool,
    active: bool,
}

impl InstalledWaydroidBridge {
    pub fn install_default(node: &InputDeviceNode) -> io::Result<Self> {
        Self::install(&WaydroidBridgePaths::default(), node, CgroupMode::detect())
    }

    pub fn install(
        paths: &WaydroidBridgePaths,
        node: &InputDeviceNode,
        cgroup_mode: CgroupMode,
    ) -> io::Result<Self> {
        let original_main = fs::read(&paths.main_config)?;
        let original_managed = match fs::read(&paths.managed_config) {
            Ok(contents) => Some(contents),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let original_managed_mode = original_managed
            .as_ref()
            .map(|_| {
                fs::metadata(&paths.managed_config)
                    .map(|metadata| metadata.permissions().mode() & 0o777)
            })
            .transpose()?;

        let main_text = std::str::from_utf8(&original_main).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Waydroid LXC config is not UTF-8: {error}"),
            )
        })?;
        let include_line = include_line(&paths.managed_config);
        let updated_main = add_include(main_text, &include_line);
        let include_added = updated_main.as_bytes() != original_main;
        let managed = render_bridge_config(node, cgroup_mode)?;
        let managed_mode = original_managed_mode.unwrap_or(0o644);
        let main_mode = fs::metadata(&paths.main_config)?.permissions().mode() & 0o777;

        atomic_write(&paths.managed_config, managed.as_bytes(), managed_mode)?;
        if include_added {
            if let Err(install_error) =
                atomic_write(&paths.main_config, updated_main.as_bytes(), main_mode)
            {
                let rollback = restore_managed_file(
                    &paths.managed_config,
                    original_managed.as_deref(),
                    original_managed_mode,
                );
                return Err(combine_io_errors(
                    "failed to add the Wroid bridge include",
                    install_error,
                    rollback.err(),
                ));
            }
        }

        Ok(Self {
            paths: paths.clone(),
            original_managed,
            original_managed_mode,
            include_added,
            active: true,
        })
    }

    pub fn cleanup(mut self) -> io::Result<()> {
        self.restore()?;
        self.active = false;
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        let main_result = if self.include_added {
            remove_managed_include(&self.paths)
        } else {
            Ok(())
        };
        let managed_result = restore_managed_file(
            &self.paths.managed_config,
            self.original_managed.as_deref(),
            self.original_managed_mode,
        );
        match (main_result, managed_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(main_error), Err(managed_error)) => Err(io::Error::other(format!(
                "failed to restore the Waydroid main config: {main_error}; \
                 managed config restore also failed: {managed_error}"
            ))),
        }
    }
}

impl Drop for InstalledWaydroidBridge {
    fn drop(&mut self) {
        if self.active {
            let _ = self.restore();
        }
    }
}

pub fn remove_default_bridge() -> io::Result<()> {
    remove_bridge(&WaydroidBridgePaths::default())
}

pub fn remove_bridge(paths: &WaydroidBridgePaths) -> io::Result<()> {
    remove_managed_include(paths)?;
    remove_file_if_exists(&paths.managed_config)
}

fn remove_managed_include(paths: &WaydroidBridgePaths) -> io::Result<()> {
    let main = fs::read_to_string(&paths.main_config)?;
    let include_line = include_line(&paths.managed_config);
    let updated = remove_include(&main, &include_line);
    let mode = fs::metadata(&paths.main_config)?.permissions().mode() & 0o777;
    if updated != main {
        atomic_write(&paths.main_config, updated.as_bytes(), mode)?;
    }
    Ok(())
}

fn restore_managed_file(
    path: &Path,
    original: Option<&[u8]>,
    original_mode: Option<u32>,
) -> io::Result<()> {
    match original {
        Some(contents) => atomic_write(path, contents, original_mode.unwrap_or(0o644)),
        None => remove_file_if_exists(path),
    }
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn combine_io_errors(context: &str, primary: io::Error, rollback: Option<io::Error>) -> io::Error {
    match rollback {
        Some(rollback) => io::Error::other(format!(
            "{context}: {primary}; rollback also failed: {rollback}"
        )),
        None => io::Error::new(primary.kind(), format!("{context}: {primary}")),
    }
}

pub fn render_bridge_config(node: &InputDeviceNode, cgroup_mode: CgroupMode) -> io::Result<String> {
    let source = node.path.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "input device path is not UTF-8",
        )
    })?;
    if source.contains(['\n', '\r']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input device path contains a line break",
        ));
    }
    let destination = source.trim_start_matches('/');

    Ok(format!(
        "{MANAGED_HEADER}\n# Host cgroup mode: {}; Waydroid device policy is intentionally unchanged\nlxc.mount.entry = {source} {destination} none bind,create=file 0 0\n",
        cgroup_mode.label()
    ))
}

fn include_line(managed_config: &Path) -> String {
    format!("lxc.include = {}", managed_config.display())
}

fn add_include(config: &str, include: &str) -> String {
    if config.lines().any(|line| line.trim() == include) {
        return config.to_string();
    }

    let mut updated = config.trim_end_matches('\n').to_string();
    updated.push('\n');
    updated.push_str(include);
    updated.push('\n');
    updated
}

fn remove_include(config: &str, include: &str) -> String {
    let mut updated = config
        .lines()
        .filter(|line| line.trim() != include)
        .collect::<Vec<_>>()
        .join("\n");
    if config.ends_with('\n') || !updated.is_empty() {
        updated.push('\n');
    }
    updated
}

fn atomic_write(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path has no parent directory: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;

    let temporary = atomic_temporary_path(path, parent);
    let result = (|| -> io::Result<()> {
        fs::write(&temporary, contents)?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn atomic_temporary_path(path: &Path, parent: &Path) -> PathBuf {
    parent.join(format!(
        ".{}.wroid-{}-tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        std::process::id()
    ))
}

const fn linux_major(device: u64) -> u64 {
    ((device >> 8) & 0xfff) | ((device >> 32) & 0xfffff000)
}

const fn linux_minor(device: u64) -> u64 {
    (device & 0xff) | ((device >> 12) & 0xffffff00)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_lease_rejects_concurrent_owner_and_recovers_after_drop() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bridge.lock");

        let first = WaydroidBridgeLease::acquire(&path, "PUBG Mobile").unwrap();
        let error = WaydroidBridgeLease::acquire(&path, "Standoff 2")
            .err()
            .expect("the second owner must be rejected");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error
            .to_string()
            .contains("another Wroid process already owns the Waydroid input bridge"));
        assert!(error.to_string().contains("PID"));
        assert!(error.to_string().contains("PUBG Mobile"));
        assert!(active_bridge_lease_owner(&path)
            .unwrap()
            .is_some_and(|owner| owner.contains("PUBG Mobile")));

        drop(first);
        assert_eq!(active_bridge_lease_owner(&path).unwrap(), None);
        WaydroidBridgeLease::acquire(&path, "Standoff 2").unwrap();
    }

    #[test]
    fn renders_mount_rules_without_replacing_waydroid_device_policy() {
        let node = InputDeviceNode::for_test("/dev/input/event29", 13, 93);
        let config = render_bridge_config(&node, CgroupMode::V2).unwrap();

        assert!(config.contains("Host cgroup mode: v2"));
        assert!(!config.contains("devices.allow"));
        assert!(!config.contains("devices.deny"));
        assert!(!config.contains("tmpfs dev/input"));
        assert!(config.contains(
            "lxc.mount.entry = /dev/input/event29 dev/input/event29 none bind,create=file 0 0"
        ));
    }

    #[test]
    fn privileged_identity_check_accepts_only_the_wroid_virtual_device() {
        validate_wroid_touchscreen_identity(
            Some(WROID_TOUCHSCREEN_NAME),
            BusType::BUS_VIRTUAL,
            WROID_TOUCHSCREEN_VENDOR,
            WROID_TOUCHSCREEN_PRODUCT,
            Path::new("/sys/devices/virtual/input/input42"),
        )
        .unwrap();

        for invalid in [
            validate_wroid_touchscreen_identity(
                Some("Physical keyboard"),
                BusType::BUS_VIRTUAL,
                WROID_TOUCHSCREEN_VENDOR,
                WROID_TOUCHSCREEN_PRODUCT,
                Path::new("/sys/devices/virtual/input/input42"),
            ),
            validate_wroid_touchscreen_identity(
                Some(WROID_TOUCHSCREEN_NAME),
                BusType::BUS_USB,
                WROID_TOUCHSCREEN_VENDOR,
                WROID_TOUCHSCREEN_PRODUCT,
                Path::new("/sys/devices/pci0000:00/input/input42"),
            ),
        ] {
            assert_eq!(invalid.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        }
    }

    #[test]
    fn install_and_cleanup_restore_original_files() {
        let directory = tempfile::tempdir().unwrap();
        let paths = WaydroidBridgePaths {
            main_config: directory.path().join("config"),
            managed_config: directory.path().join("config_wroid_input"),
        };
        fs::write(&paths.main_config, "lxc.rootfs.path = /rootfs\n").unwrap();
        let node = InputDeviceNode::for_test("/dev/input/event29", 13, 93);

        let bridge = InstalledWaydroidBridge::install(&paths, &node, CgroupMode::V2).unwrap();
        let installed_main = fs::read_to_string(&paths.main_config).unwrap();
        assert!(installed_main.contains("config_wroid_input"));
        assert!(paths.managed_config.is_file());

        bridge.cleanup().unwrap();
        assert_eq!(
            fs::read_to_string(&paths.main_config).unwrap(),
            "lxc.rootfs.path = /rootfs\n"
        );
        assert!(!paths.managed_config.exists());
    }

    #[test]
    fn failed_main_config_update_rolls_back_managed_config() {
        let directory = tempfile::tempdir().unwrap();
        let paths = WaydroidBridgePaths {
            main_config: directory.path().join("config"),
            managed_config: directory.path().join("config_wroid_input"),
        };
        fs::write(&paths.main_config, "lxc.rootfs.path = /rootfs\n").unwrap();
        fs::write(&paths.managed_config, "previous managed config\n").unwrap();
        let main_temporary = atomic_temporary_path(&paths.main_config, directory.path());
        fs::create_dir(&main_temporary).unwrap();
        let node = InputDeviceNode::for_test("/dev/input/event29", 13, 93);

        let error = InstalledWaydroidBridge::install(&paths, &node, CgroupMode::V2)
            .err()
            .expect("main config write should fail");

        assert!(error
            .to_string()
            .contains("failed to add the Wroid bridge include"));
        assert_eq!(
            fs::read_to_string(&paths.main_config).unwrap(),
            "lxc.rootfs.path = /rootfs\n"
        );
        assert_eq!(
            fs::read_to_string(&paths.managed_config).unwrap(),
            "previous managed config\n"
        );
    }

    #[test]
    fn cleanup_preserves_unrelated_main_config_changes() {
        let directory = tempfile::tempdir().unwrap();
        let paths = WaydroidBridgePaths {
            main_config: directory.path().join("config"),
            managed_config: directory.path().join("config_wroid_input"),
        };
        fs::write(&paths.main_config, "lxc.rootfs.path = /rootfs\n").unwrap();
        let node = InputDeviceNode::for_test("/dev/input/event29", 13, 93);
        let bridge = InstalledWaydroidBridge::install(&paths, &node, CgroupMode::V2).unwrap();
        let mut main = fs::read_to_string(&paths.main_config).unwrap();
        main.push_str("lxc.apparmor.profile = generated\n");
        fs::write(&paths.main_config, main).unwrap();

        bridge.cleanup().unwrap();

        assert_eq!(
            fs::read_to_string(&paths.main_config).unwrap(),
            "lxc.rootfs.path = /rootfs\nlxc.apparmor.profile = generated\n"
        );
        assert!(!paths.managed_config.exists());
    }

    #[test]
    fn adding_include_is_idempotent() {
        let include = "lxc.include = /tmp/config_wroid_input";
        let once = add_include("base\n", include);
        let twice = add_include(&once, include);
        assert_eq!(once, twice);
    }

    #[test]
    fn remove_bridge_preserves_unrelated_config() {
        let directory = tempfile::tempdir().unwrap();
        let paths = WaydroidBridgePaths {
            main_config: directory.path().join("config"),
            managed_config: directory.path().join("config_wroid_input"),
        };
        fs::write(
            &paths.main_config,
            format!(
                "first = 1\n{}\nlast = 2\n",
                include_line(&paths.managed_config)
            ),
        )
        .unwrap();
        fs::write(&paths.managed_config, "managed\n").unwrap();

        remove_bridge(&paths).unwrap();

        assert_eq!(
            fs::read_to_string(&paths.main_config).unwrap(),
            "first = 1\nlast = 2\n"
        );
        assert!(!paths.managed_config.exists());
    }
}
