use std::env;
use std::ffi::CString;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use wroid_inject::{validate_installed_bridge_helper, DEFAULT_PRIVILEGED_BRIDGE_HELPER};

use super::desktop;

const SUDO: &str = "/usr/bin/sudo";
const PKEXEC: &str = "/usr/bin/pkexec";
const INSTALL: &str = "/usr/bin/install";
const HELPER_INSTALL_LOCK: &str = "helper-install.lock";
const HELPER_INSTALL_LOG: &str = "helper-install.log";
const MAX_STAGED_HELPER_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelperReadiness {
    pub(crate) state: &'static str,
    pub(crate) ready: bool,
    pub(crate) detail: String,
}

pub(crate) fn readiness() -> HelperReadiness {
    if let Ok(Some(owner)) = active_graphical_install_owner() {
        return HelperReadiness {
            state: "installing",
            ready: false,
            detail: format!("System authorization is open ({owner})"),
        };
    }
    let path = Path::new(DEFAULT_PRIVILEGED_BRIDGE_HELPER);
    match validate_installed_bridge_helper(path) {
        Ok(()) => match desktop::staged_helper_path()
            .ok()
            .filter(|staged| staged.is_file())
            .map(|staged| files_match(path, &staged).map(|matches| (matches, staged)))
            .transpose()
        {
            Ok(Some((true, _))) | Ok(None) => HelperReadiness {
                state: "ready",
                ready: true,
                detail: format!("{} is root-owned and release-matched", path.display()),
            },
            Ok(Some((false, staged))) => HelperReadiness {
                state: "outdated",
                ready: false,
                detail: format!(
                    "{} differs from staged release {}; reinstall the helper",
                    path.display(),
                    staged.display()
                ),
            },
            Err(error) => HelperReadiness {
                state: "unsafe",
                ready: false,
                detail: format!("cannot verify bridge helper release: {error}"),
            },
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HelperReadiness {
            state: "missing",
            ready: false,
            detail: "Install the minimal root-owned bridge helper once before production play"
                .to_owned(),
        },
        Err(error) => HelperReadiness {
            state: "unsafe",
            ready: false,
            detail: error.to_string(),
        },
    }
}

fn files_match(left: &Path, right: &Path) -> io::Result<bool> {
    if fs::metadata(left)?.len() != fs::metadata(right)?.len() {
        return Ok(false);
    }
    let mut left = fs::File::open(left)?;
    let mut right = fs::File::open(right)?;
    let mut left_buffer = [0_u8; 16 * 1024];
    let mut right_buffer = [0_u8; 16 * 1024];
    loop {
        let left_read = left.read(&mut left_buffer)?;
        let right_read = right.read(&mut right_buffer)?;
        if left_read != right_read || left_buffer[..left_read] != right_buffer[..right_read] {
            return Ok(false);
        }
        if left_read == 0 {
            return Ok(true);
        }
    }
}

pub(crate) fn ensure_ready() -> Result<()> {
    let readiness = readiness();
    if readiness.ready {
        Ok(())
    } else {
        bail!(
            "{}; run `wroid helper install` or use the Hub setup action",
            readiness.detail
        )
    }
}

pub(crate) fn status() -> Result<()> {
    let readiness = readiness();
    println!(
        "Production bridge helper: {} ({})",
        DEFAULT_PRIVILEGED_BRIDGE_HELPER, readiness.state
    );
    println!("{}", readiness.detail);
    if !readiness.ready {
        println!("Install or repair it with: wroid helper install");
    }
    Ok(())
}

pub(crate) fn install() -> Result<()> {
    if graphical_install_supported() {
        return install_graphical();
    }
    install_with_sudo()
}

fn install_with_sudo() -> Result<()> {
    ensure_input_group_membership()?;
    let source = desktop::staged_helper_path()?;
    let helper_bytes = read_staged_helper(&source)?;
    let source = source
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", source.display()))?;
    let destination = PathBuf::from(DEFAULT_PRIVILEGED_BRIDGE_HELPER);

    println!(
        "Installing the minimal Wroid bridge helper to {}…",
        destination.display()
    );
    println!(
        "sudo authorizes one root:input setuid installation; gameplay launches need no password."
    );
    let status = Command::new(SUDO)
        .args(install_arguments(&source, &destination))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to start sudo for the Wroid bridge helper installation")?;
    if !status.success() {
        bail!("Wroid bridge helper installation exited with {status}");
    }
    validate_installed_release(&destination, &helper_bytes)?;
    println!(
        "Production bridge helper is ready: {}",
        destination.display()
    );
    Ok(())
}

pub(crate) fn graphical_install_supported() -> bool {
    let graphical_session =
        env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("DISPLAY").is_some();
    graphical_session && trusted_root_executable(PKEXEC) && trusted_root_executable(INSTALL)
}

fn trusted_root_executable(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|metadata| {
        metadata.is_file()
            && metadata.uid() == 0
            && metadata.permissions().mode() & 0o111 != 0
            && metadata.permissions().mode() & 0o022 == 0
    })
}

pub(crate) fn start_graphical_install() -> Result<String> {
    if !graphical_install_supported() {
        bail!("graphical Polkit authorization is unavailable");
    }
    ensure_input_group_membership()?;
    let source = desktop::staged_helper_path()?;
    validate_staged_helper(&source)?;
    if let Some(owner) = active_graphical_install_owner()? {
        bail!("another helper installation is already active ({owner})");
    }
    let mut log = open_private_install_log()?;
    writeln!(log, "Wroid graphical helper setup")?;
    let stderr = log
        .try_clone()
        .context("failed to clone the helper installation log")?;
    let executable = env::current_exe().context("failed to locate the Wroid executable")?;
    let mut command = Command::new(executable);
    command
        .arg("install-helper-graphical")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr));
    command.process_group(0);
    command
        .spawn()
        .context("failed to open graphical system authorization")?;

    Ok("System authorization opened for the one-time secure helper setup".to_owned())
}

pub(crate) fn install_graphical() -> Result<()> {
    if !graphical_install_supported() {
        bail!("graphical Polkit authorization is unavailable");
    }
    ensure_input_group_membership()?;
    let lease_path = helper_install_lease_path()?;
    let lease_directory = lease_path
        .parent()
        .context("helper installation lease has no parent")?;
    fs::create_dir_all(lease_directory)
        .with_context(|| format!("failed to create {}", lease_directory.display()))?;
    fs::set_permissions(lease_directory, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", lease_directory.display()))?;
    let _lease = wroid_inject::WaydroidBridgeLease::acquire_named(
        &lease_path,
        "the secure helper installer",
        "authorizing the production bridge helper",
    )
    .context("another helper installation is already active")?;

    let source = desktop::staged_helper_path()?;
    let helper_bytes = read_staged_helper(&source)?;
    let (sealed_helper, sealed_source) = sealed_helper_file(&helper_bytes)?;
    let destination = PathBuf::from(DEFAULT_PRIVILEGED_BRIDGE_HELPER);
    println!(
        "Requesting one-time system authorization for {}…",
        destination.display()
    );
    let status = Command::new(PKEXEC)
        .args(graphical_install_arguments(&sealed_source, &destination))
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to open graphical system authorization")?;
    drop(sealed_helper);
    if !status.success() {
        bail!("system authorization or helper installation exited with {status}");
    }
    validate_installed_release(&destination, &helper_bytes)?;
    println!(
        "Production bridge helper is ready: {}",
        destination.display()
    );
    Ok(())
}

fn active_graphical_install_owner() -> io::Result<Option<String>> {
    let Some(path) = optional_helper_install_lease_path() else {
        return Ok(None);
    };
    wroid_inject::active_bridge_lease_owner(path)
}

fn helper_install_lease_path() -> Result<PathBuf> {
    optional_helper_install_lease_path()
        .context("XDG_RUNTIME_DIR is unavailable for helper installation")
}

fn optional_helper_install_lease_path() -> Option<PathBuf> {
    env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|directory| directory.join("wroid").join(HELPER_INSTALL_LOCK))
}

fn helper_install_log_path() -> Result<PathBuf> {
    let state_directory = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("state"))
        })
        .context("HOME and XDG_STATE_HOME are unavailable for the helper installation log")?;
    Ok(state_directory.join("wroid").join(HELPER_INSTALL_LOG))
}

fn open_private_install_log() -> Result<fs::File> {
    let path = helper_install_log_path()?;
    let directory = path.parent().context("helper install log has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("failed to secure {}", directory.display()))?;
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to secure {}", path.display()))?;
    Ok(file)
}

fn read_staged_helper(path: &Path) -> Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("failed to open staged helper {}", path.display()))?;
    validate_staged_helper_metadata(path, &file.metadata()?)?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_STAGED_HELPER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_STAGED_HELPER_BYTES {
        bail!("staged wroid-helper exceeds the 64 MiB safety limit");
    }
    Ok(bytes)
}

fn validate_installed_release(destination: &Path, expected: &[u8]) -> Result<()> {
    validate_installed_bridge_helper(destination)
        .context("installed bridge helper failed its ownership check")?;
    let installed = fs::read(destination)
        .with_context(|| format!("failed to verify {}", destination.display()))?;
    if installed != expected {
        bail!("installed bridge helper differs from the sealed staged release");
    }
    Ok(())
}

fn sealed_helper_file(bytes: &[u8]) -> Result<(fs::File, PathBuf)> {
    let name = CString::new("wroid-helper-release").expect("fixed memfd name has no NUL");
    // SAFETY: name is a valid NUL-terminated string and flags are fixed.
    let raw_fd = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            name.as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if raw_fd < 0 {
        return Err(io::Error::last_os_error()).context("failed to create sealed helper memory");
    }
    // SAFETY: memfd_create returned a new descriptor owned by this process.
    let mut file = unsafe { fs::File::from_raw_fd(raw_fd as i32) };
    file.write_all(bytes)
        .context("failed to populate sealed helper memory")?;
    file.sync_all()?;
    let seals = libc::F_SEAL_SEAL | libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_WRITE;
    // SAFETY: fcntl applies fixed seals to the valid memfd descriptor.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_ADD_SEALS, seals) } != 0 {
        return Err(io::Error::last_os_error()).context("failed to seal helper memory");
    }
    let path = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        file.as_raw_fd()
    ));
    Ok((file, path))
}

fn validate_staged_helper(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("staged wroid-helper is missing: {}", path.display()))?;
    validate_staged_helper_metadata(path, &file.metadata()?)
}

fn validate_staged_helper_metadata(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    // SAFETY: geteuid has no preconditions.
    let current_uid = unsafe { libc::geteuid() };
    if !metadata.file_type().is_file()
        || metadata.uid() != current_uid
        || metadata.permissions().mode() & 0o777 != 0o555
    {
        bail!(
            "staged wroid-helper must be a current-user-owned 0555 regular file: {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_input_group_membership() -> Result<()> {
    let input = CString::new("input").expect("fixed group name has no NUL bytes");
    // SAFETY: `input` is NUL-terminated and getgrnam returns process-owned
    // group database storage that is only read during this call.
    let group = unsafe { libc::getgrnam(input.as_ptr()) };
    if group.is_null() {
        bail!("the system has no `input` group required by the Wroid helper");
    }
    // SAFETY: the null pointer case was handled above.
    let input_gid = unsafe { (*group).gr_gid };
    // SAFETY: getegid has no preconditions.
    let primary_gid = unsafe { libc::getegid() };
    // SAFETY: the first call requests the required length; the second writes
    // into an allocated buffer of exactly that length.
    let group_count = unsafe { libc::getgroups(0, std::ptr::null_mut()) };
    if group_count < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to read process groups");
    }
    let mut groups = vec![0; group_count as usize];
    if group_count > 0 {
        // SAFETY: `groups` has capacity for group_count gid_t values.
        let read = unsafe { libc::getgroups(group_count, groups.as_mut_ptr()) };
        if read < 0 {
            return Err(std::io::Error::last_os_error()).context("failed to read process groups");
        }
        groups.truncate(read as usize);
    }
    if !group_is_authorized(input_gid, primary_gid, &groups) {
        bail!(
            "the current user is not in the `input` group; add the user and sign in again before installing the helper"
        );
    }
    Ok(())
}

fn group_is_authorized(
    input_gid: libc::gid_t,
    primary_gid: libc::gid_t,
    groups: &[libc::gid_t],
) -> bool {
    primary_gid == input_gid || groups.contains(&input_gid)
}

fn install_arguments(source: &Path, destination: &Path) -> Vec<OsString> {
    [
        OsString::from("--"),
        OsString::from(INSTALL),
        OsString::from("-D"),
        OsString::from("-o"),
        OsString::from("root"),
        OsString::from("-g"),
        OsString::from("input"),
        OsString::from("-m"),
        OsString::from("4750"),
        source.as_os_str().to_owned(),
        destination.as_os_str().to_owned(),
    ]
    .into()
}

fn graphical_install_arguments(source: &Path, destination: &Path) -> Vec<OsString> {
    [
        OsString::from(INSTALL),
        OsString::from("-D"),
        OsString::from("-o"),
        OsString::from("root"),
        OsString::from("-g"),
        OsString::from("input"),
        OsString::from("-m"),
        OsString::from("4750"),
        source.as_os_str().to_owned(),
        destination.as_os_str().to_owned(),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_install_invocation_has_fixed_program_and_destination() {
        assert_eq!(
            install_arguments(
                Path::new("/home/player/.local/share/libexec/wroid/wroid-helper"),
                Path::new(DEFAULT_PRIVILEGED_BRIDGE_HELPER),
            ),
            [
                "--",
                "/usr/bin/install",
                "-D",
                "-o",
                "root",
                "-g",
                "input",
                "-m",
                "4750",
                "/home/player/.local/share/libexec/wroid/wroid-helper",
                "/usr/lib/wroid/wroid-helper",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn staged_helper_rejects_writable_or_non_executable_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wroid-helper");
        fs::write(&path, b"helper").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o555)).unwrap();
        validate_staged_helper(&path).unwrap();

        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(validate_staged_helper(&path).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(validate_staged_helper(&path).is_err());
    }

    #[test]
    fn graphical_install_uses_a_sealed_proc_fd_and_fixed_destination() {
        assert_eq!(
            graphical_install_arguments(
                Path::new("/proc/42/fd/7"),
                Path::new(DEFAULT_PRIVILEGED_BRIDGE_HELPER),
            ),
            [
                "/usr/bin/install",
                "-D",
                "-o",
                "root",
                "-g",
                "input",
                "-m",
                "4750",
                "/proc/42/fd/7",
                "/usr/lib/wroid/wroid-helper",
            ]
            .map(OsString::from)
        );
    }

    #[test]
    fn helper_memfd_is_complete_and_write_sealed() {
        let bytes = b"immutable helper release";
        let (mut file, path) = sealed_helper_file(bytes).unwrap();

        assert_eq!(fs::read(path).unwrap(), bytes);
        assert!(file.write_all(b"changed").is_err());
    }

    #[test]
    fn fixed_install_can_copy_the_sealed_proc_fd() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("installed-helper");
        let bytes = b"sealed helper executable";
        let (_file, source) = sealed_helper_file(bytes).unwrap();

        let status = Command::new(INSTALL)
            .args(["-D", "-m", "0500"])
            .arg(source)
            .arg(&destination)
            .status()
            .unwrap();

        assert!(status.success());
        assert_eq!(fs::read(destination).unwrap(), bytes);
    }

    #[test]
    fn staged_helper_bytes_are_read_only_after_validation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wroid-helper");
        fs::write(&path, b"sealed helper release").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o555)).unwrap();

        assert_eq!(read_staged_helper(&path).unwrap(), b"sealed helper release");
    }

    #[test]
    fn release_match_detects_changed_or_truncated_helpers() {
        let directory = tempfile::tempdir().unwrap();
        let installed = directory.path().join("installed");
        let staged = directory.path().join("staged");
        fs::write(&installed, b"same helper bytes").unwrap();
        fs::write(&staged, b"same helper bytes").unwrap();
        assert!(files_match(&installed, &staged).unwrap());

        fs::write(&staged, b"same helper byte!").unwrap();
        assert!(!files_match(&installed, &staged).unwrap());
        fs::write(&staged, b"short").unwrap();
        assert!(!files_match(&installed, &staged).unwrap());
    }

    #[test]
    fn helper_group_accepts_primary_or_supplementary_input_membership() {
        assert!(group_is_authorized(992, 992, &[]));
        assert!(group_is_authorized(992, 1000, &[10, 992, 998]));
        assert!(!group_is_authorized(992, 1000, &[10, 998]));
    }
}
