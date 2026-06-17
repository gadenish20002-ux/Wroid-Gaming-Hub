use std::fs;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

pub const DEFAULT_WAYDROID_CONFIG: &str = "/var/lib/waydroid/lxc/waydroid/config";
pub const DEFAULT_WAYDROID_BRIDGE_CONFIG: &str =
    "/var/lib/waydroid/lxc/waydroid/config_wroid_input";

const MANAGED_HEADER: &str = "# Managed by Wroid Gaming Hub input bridge";

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

    fn allow_key(self) -> &'static str {
        match self {
            Self::V1 => "lxc.cgroup.devices.allow",
            Self::V2 => "lxc.cgroup2.devices.allow",
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
    original_main: Vec<u8>,
    original_managed: Option<Vec<u8>>,
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

        let main_text = std::str::from_utf8(&original_main).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Waydroid LXC config is not UTF-8: {error}"),
            )
        })?;
        let include_line = include_line(&paths.managed_config);
        let updated_main = add_include(main_text, &include_line);
        let managed = render_bridge_config(node, cgroup_mode)?;

        atomic_write(&paths.managed_config, managed.as_bytes(), 0o644)?;
        let main_mode = fs::metadata(&paths.main_config)?.permissions().mode() & 0o777;
        if updated_main.as_bytes() != original_main {
            atomic_write(&paths.main_config, updated_main.as_bytes(), main_mode)?;
        }

        Ok(Self {
            paths: paths.clone(),
            original_main,
            original_managed,
            active: true,
        })
    }

    pub fn cleanup(mut self) -> io::Result<()> {
        self.restore()?;
        self.active = false;
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        let main_mode = fs::metadata(&self.paths.main_config)
            .map(|metadata| metadata.permissions().mode() & 0o777)
            .unwrap_or(0o644);
        atomic_write(&self.paths.main_config, &self.original_main, main_mode)?;

        match &self.original_managed {
            Some(contents) => atomic_write(&self.paths.managed_config, contents, 0o644),
            None => match fs::remove_file(&self.paths.managed_config) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            },
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
    let main = fs::read_to_string(&paths.main_config)?;
    let include_line = include_line(&paths.managed_config);
    let updated = remove_include(&main, &include_line);
    let mode = fs::metadata(&paths.main_config)?.permissions().mode() & 0o777;
    if updated != main {
        atomic_write(&paths.main_config, updated.as_bytes(), mode)?;
    }

    match fs::remove_file(&paths.managed_config) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
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
        "{MANAGED_HEADER}\n{} = c {}:{} rwm\nlxc.mount.entry = {source} {destination} none bind,create=file 0 0\n",
        cgroup_mode.allow_key(),
        node.major,
        node.minor
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

    let temporary = parent.join(format!(
        ".{}.wroid-{}-tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    fs::write(&temporary, contents)?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
    fs::rename(&temporary, path)
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
    fn renders_exact_cgroup_and_mount_rules() {
        let node = InputDeviceNode::for_test("/dev/input/event29", 13, 93);
        let config = render_bridge_config(&node, CgroupMode::V2).unwrap();

        assert!(config.contains("lxc.cgroup2.devices.allow = c 13:93 rwm"));
        assert!(config.contains(
            "lxc.mount.entry = /dev/input/event29 dev/input/event29 none bind,create=file 0 0"
        ));
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
