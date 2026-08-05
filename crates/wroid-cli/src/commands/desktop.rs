use std::env;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

const APPLICATION_ID: &str = "io.wroid.GamingHub";
const ICON_SVG: &str = include_str!("../../assets/desktop/wroid.svg");

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopPaths {
    binary: PathBuf,
    helper: PathBuf,
    daemon: PathBuf,
    desktop_entry: PathBuf,
    icon: PathBuf,
}

pub(crate) fn install() -> Result<()> {
    ensure_desktop_user()?;
    let paths = DesktopPaths::from_environment()?;
    let source = env::current_exe().context("failed to locate the running wroid binary")?;
    let helper_source = adjacent_or_staged_helper(&source, &paths)?;
    let daemon_source = adjacent_or_staged_daemon(&source, &paths)?;
    install_from(&source, &helper_source, &daemon_source, &paths)?;
    refresh_desktop_database(&paths);

    println!("Wroid Gaming Hub installed for the current user.");
    println!("Binary: {}", paths.binary.display());
    println!("Privileged helper staging: {}", paths.helper.display());
    println!("Runtime daemon: {}", paths.daemon.display());
    println!("Application entry: {}", paths.desktop_entry.display());
    println!("Profiles remain in ~/.config/wroid/profiles-v2.");
    Ok(())
}

pub(crate) fn status() -> Result<()> {
    ensure_desktop_user()?;
    let paths = DesktopPaths::from_environment()?;
    println!(
        "Binary: {} ({})",
        paths.binary.display(),
        presence(&paths.binary)
    );
    println!(
        "Application entry: {} ({})",
        paths.desktop_entry.display(),
        presence(&paths.desktop_entry)
    );
    println!(
        "Privileged helper staging: {} ({})",
        paths.helper.display(),
        presence(&paths.helper)
    );
    println!(
        "Runtime daemon: {} ({})",
        paths.daemon.display(),
        presence(&paths.daemon)
    );
    println!("Icon: {} ({})", paths.icon.display(), presence(&paths.icon));
    Ok(())
}

pub(crate) fn uninstall() -> Result<()> {
    ensure_desktop_user()?;
    let paths = DesktopPaths::from_environment()?;
    let removed = uninstall_paths(&paths)?;
    refresh_desktop_database(&paths);
    println!(
        "Wroid desktop application removed ({} file{}).",
        removed,
        if removed == 1 { "" } else { "s" }
    );
    println!("Profiles and calibration backgrounds were preserved.");
    Ok(())
}

impl DesktopPaths {
    fn from_environment() -> Result<Self> {
        let home = env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .context("HOME is not set; cannot locate the user installation directory")?;
        let data_home = env::var_os("XDG_DATA_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("share"));
        let bin_home = env::var_os("XDG_BIN_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local").join("bin"));

        Ok(Self {
            binary: bin_home.join("wroid"),
            helper: data_home.join("libexec").join("wroid").join("wroid-helper"),
            daemon: data_home.join("libexec").join("wroid").join("wroidd"),
            desktop_entry: data_home
                .join("applications")
                .join(format!("{APPLICATION_ID}.desktop")),
            icon: data_home
                .join("icons")
                .join("hicolor")
                .join("scalable")
                .join("apps")
                .join(format!("{APPLICATION_ID}.svg")),
        })
    }
}

pub(crate) fn staged_helper_path() -> Result<PathBuf> {
    Ok(DesktopPaths::from_environment()?.helper)
}

fn adjacent_or_staged_helper(source: &Path, paths: &DesktopPaths) -> Result<PathBuf> {
    let adjacent = source
        .parent()
        .context("wroid executable has no parent directory")?
        .join("wroid-helper");
    if adjacent.is_file() {
        return Ok(adjacent);
    }
    if paths.helper.is_file() {
        return Ok(paths.helper.clone());
    }
    bail!(
        "wroid-helper was not found beside {} or in staging {}; build/install the complete release",
        source.display(),
        paths.helper.display()
    )
}

fn adjacent_or_staged_daemon(source: &Path, paths: &DesktopPaths) -> Result<PathBuf> {
    let adjacent = source
        .parent()
        .context("wroid executable has no parent directory")?
        .join("wroidd");
    if adjacent.is_file() {
        return Ok(adjacent);
    }
    if paths.daemon.is_file() {
        return Ok(paths.daemon.clone());
    }
    bail!(
        "wroidd was not found beside {} or in staging {}; build/install the complete release",
        source.display(),
        paths.daemon.display()
    )
}

fn install_from(
    source: &Path,
    helper_source: &Path,
    daemon_source: &Path,
    paths: &DesktopPaths,
) -> Result<()> {
    if !source.is_file() {
        bail!(
            "wroid installation source is not a file: {}",
            source.display()
        );
    }
    let source = source
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", source.display()))?;
    let target_is_source = paths
        .binary
        .canonicalize()
        .is_ok_and(|target| target == source);

    if !target_is_source {
        atomic_copy_with_mode(&source, &paths.binary, 0o755)?;
    }
    atomic_copy_with_mode(helper_source, &paths.helper, 0o555)?;
    atomic_copy_with_mode(daemon_source, &paths.daemon, 0o555)?;
    atomic_write(
        &paths.desktop_entry,
        desktop_entry(&paths.binary)?.as_bytes(),
        0o644,
    )?;
    atomic_write(&paths.icon, ICON_SVG.as_bytes(), 0o644)?;
    Ok(())
}

fn uninstall_paths(paths: &DesktopPaths) -> Result<usize> {
    let mut removed = 0;
    for path in [
        &paths.desktop_entry,
        &paths.icon,
        &paths.helper,
        &paths.daemon,
        &paths.binary,
    ] {
        match fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to remove {}", path.display()))
            }
        }
    }
    Ok(removed)
}

fn atomic_copy_with_mode(source: &Path, target: &Path, mode: u32) -> Result<()> {
    let directory = target
        .parent()
        .context("binary target has no parent directory")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let temporary = temporary_path(target);
    fs::copy(source, &temporary).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            temporary.display()
        )
    })?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to set permissions on {}", temporary.display()))?;
    sync_and_replace(&temporary, target)
}

fn atomic_write(target: &Path, data: &[u8], mode: u32) -> Result<()> {
    let directory = target
        .parent()
        .context("installation target has no parent")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    let temporary = temporary_path(target);
    fs::write(&temporary, data)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to set permissions on {}", temporary.display()))?;
    sync_and_replace(&temporary, target)
}

fn sync_and_replace(temporary: &Path, target: &Path) -> Result<()> {
    let result = (|| {
        fs::File::open(temporary)
            .with_context(|| format!("failed to reopen {}", temporary.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync {}", temporary.display()))?;
        fs::rename(temporary, target).with_context(|| {
            format!(
                "failed to replace {} with {}",
                target.display(),
                temporary.display()
            )
        })
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn temporary_path(target: &Path) -> PathBuf {
    let directory = target.parent().unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wroid");
    directory.join(format!(".{file_name}.{}.tmp", std::process::id()))
}

fn desktop_entry(binary: &Path) -> Result<String> {
    let executable = binary
        .to_str()
        .context("the Wroid binary path is not valid UTF-8")?;
    if executable.chars().any(|character| character.is_control()) {
        bail!("the Wroid binary path contains unsupported control characters");
    }
    let executable = quote_exec_argument(executable);
    Ok(format!(
        "[Desktop Entry]\n\
         Version=1.0\n\
         Type=Application\n\
         Name=Wroid Gaming Hub\n\
         GenericName=Android Gaming Hub\n\
         Comment=Launch and configure low-latency Android games through Waydroid\n\
         Exec={executable} hub\n\
         Icon={APPLICATION_ID}\n\
         Terminal=false\n\
         StartupNotify=true\n\
         Categories=Game;Emulator;\n\
         Keywords=Android;Gaming;Waydroid;Emulator;\n\
         X-GNOME-UsesNotifications=false\n"
    ))
}

fn quote_exec_argument(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        if matches!(character, '\\' | '"' | '`' | '$') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped.push('"');
    escaped
}

fn refresh_desktop_database(paths: &DesktopPaths) {
    let Some(directory) = paths.desktop_entry.parent() else {
        return;
    };
    match Command::new("update-desktop-database")
        .arg(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("Warning: update-desktop-database exited with {status}"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => eprintln!("Warning: could not refresh the application database: {error}"),
    }
}

fn presence(path: &Path) -> &'static str {
    if path.is_file() {
        "installed"
    } else {
        "missing"
    }
}

fn ensure_desktop_user() -> Result<()> {
    if effective_uid_from_proc().unwrap_or(u32::MAX) == 0 {
        bail!("desktop integration must run as the desktop user, without sudo");
    }
    Ok(())
}

fn effective_uid_from_proc() -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let uid_line = status.lines().find(|line| line.starts_with("Uid:"))?;
    uid_line.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(root: &Path) -> DesktopPaths {
        DesktopPaths {
            binary: root.join("bin/wroid"),
            helper: root.join("share/libexec/wroid/wroid-helper"),
            daemon: root.join("share/libexec/wroid/wroidd"),
            desktop_entry: root.join("share/applications/io.wroid.GamingHub.desktop"),
            icon: root.join("share/icons/hicolor/scalable/apps/io.wroid.GamingHub.svg"),
        }
    }

    #[test]
    fn installs_and_uninstalls_without_touching_profile_data() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source-wroid");
        let helper_source = directory.path().join("wroid-helper");
        let daemon_source = directory.path().join("wroidd");
        fs::write(&source, b"fake-wroid-binary").unwrap();
        fs::write(&helper_source, b"fake-helper-binary").unwrap();
        fs::write(&daemon_source, b"fake-daemon-binary").unwrap();
        let paths = paths(&directory.path().join("install"));
        let profile = directory
            .path()
            .join("install/config/wroid/profiles-v2/game.json");
        fs::create_dir_all(profile.parent().unwrap()).unwrap();
        fs::write(&profile, "{}").unwrap();

        install_from(&source, &helper_source, &daemon_source, &paths).unwrap();
        assert_eq!(fs::read(&paths.binary).unwrap(), b"fake-wroid-binary");
        assert_eq!(
            fs::metadata(&paths.binary).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            fs::metadata(&paths.helper).unwrap().permissions().mode() & 0o777,
            0o555
        );
        assert_eq!(fs::read(&paths.daemon).unwrap(), b"fake-daemon-binary");
        assert_eq!(
            fs::metadata(&paths.daemon).unwrap().permissions().mode() & 0o777,
            0o555
        );
        let entry = fs::read_to_string(&paths.desktop_entry).unwrap();
        assert!(entry.contains("Name=Wroid Gaming Hub"));
        assert!(entry.contains(" hub\n"));
        assert!(fs::read_to_string(&paths.icon).unwrap().contains("<svg"));

        assert_eq!(uninstall_paths(&paths).unwrap(), 5);
        assert!(!paths.binary.exists());
        assert!(!paths.desktop_entry.exists());
        assert!(!paths.icon.exists());
        assert!(!paths.helper.exists());
        assert!(!paths.daemon.exists());
        assert!(profile.exists());
    }

    #[test]
    fn desktop_exec_path_is_safely_quoted() {
        let entry = desktop_entry(Path::new("/home/Test User/$Games/wroid")).unwrap();
        assert!(entry.contains(r#"Exec="/home/Test User/\$Games/wroid" hub"#));
    }

    #[test]
    fn uninstall_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        assert_eq!(uninstall_paths(&paths(directory.path())).unwrap(), 0);
    }
}
