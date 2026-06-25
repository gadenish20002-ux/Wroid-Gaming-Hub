use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread::sleep;
use std::time::Duration;

pub const WROID_TOUCHSCREEN_NAME: &str = "Wroid Gaming Touchscreen";

const STATUS_ATTEMPTS: usize = 60;
const STATUS_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct DesktopUser {
    name: String,
    home: PathBuf,
    runtime_dir: PathBuf,
    wayland_display: OsString,
}

impl DesktopUser {
    pub fn from_sudo_environment() -> io::Result<Self> {
        let name = std::env::var("SUDO_USER").map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SUDO_USER is not set; run this integration tool with sudo from the desktop user session",
            )
        })?;
        let uid = std::env::var("SUDO_UID")
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SUDO_UID is not set"))?
            .parse::<u32>()
            .map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("invalid SUDO_UID: {error}"),
                )
            })?;
        let home = home_directory(&name)?;
        let runtime_dir = PathBuf::from(format!("/run/user/{uid}"));
        if !runtime_dir.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "desktop runtime directory is missing: {}",
                    runtime_dir.display()
                ),
            ));
        }
        if !runtime_dir.join("bus").exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "desktop DBus socket is missing: {}/bus",
                    runtime_dir.display()
                ),
            ));
        }

        let wayland_display = detect_wayland_display(&runtime_dir)?;
        Ok(Self {
            name,
            home,
            runtime_dir,
            wayland_display,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn wayland_display(&self) -> &OsString {
        &self.wayland_display
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut wayland = OsString::from("WAYLAND_DISPLAY=");
        wayland.push(&self.wayland_display);

        let mut command = Command::new("runuser");
        command
            .arg("-u")
            .arg(&self.name)
            .arg("--")
            .arg("env")
            .arg(format!("HOME={}", self.home.display()))
            .arg(format!("XDG_RUNTIME_DIR={}", self.runtime_dir.display()))
            .arg(format!(
                "DBUS_SESSION_BUS_ADDRESS=unix:path={}/bus",
                self.runtime_dir.display()
            ))
            .arg(wayland)
            .arg("waydroid")
            .args(arguments);
        command
    }

    fn run(&self, arguments: &[&str]) -> io::Result<()> {
        let output = self.command(arguments).output()?;
        if output.status.success() {
            return Ok(());
        }

        Err(io::Error::other(format!(
            "waydroid {} as desktop user {} failed\n{}",
            arguments.join(" "),
            self.name,
            combined_output(&output)
        )))
    }
}

pub struct DesktopWaydroidSession {
    child: Child,
    user: DesktopUser,
    active: bool,
}

impl DesktopWaydroidSession {
    pub fn start_from_sudo_environment() -> io::Result<Self> {
        Self::start(DesktopUser::from_sudo_environment()?)
    }

    pub fn start(user: DesktopUser) -> io::Result<Self> {
        println!(
            "Starting Waydroid session as desktop user {} on {}...",
            user.name(),
            user.wayland_display().to_string_lossy()
        );
        let child = user
            .command(&["session", "start"])
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut session = Self {
            child,
            user,
            active: true,
        };

        if let Err(error) = session.wait_until_running() {
            let _ = session.stop();
            return Err(error);
        }
        Ok(session)
    }

    pub fn user(&self) -> &DesktopUser {
        &self.user
    }

    pub fn show_full_ui(&self) -> io::Result<()> {
        self.user.run(&["show-full-ui"])
    }

    pub fn stop(&mut self) -> io::Result<()> {
        if !self.active {
            return Ok(());
        }

        let output = self.user.command(&["session", "stop"]).output()?;
        let user_stop_error = (!output.status.success()).then(|| {
            io::Error::other(format!(
                "waydroid session stop failed\n{}",
                combined_output(&output)
            ))
        });

        let mut last_status = String::new();
        for _ in 0..STATUS_ATTEMPTS {
            last_status = waydroid_status()?;
            if waydroid_is_stopped(&last_status) {
                self.active = false;
                let _ = self.child.wait();
                return match user_stop_error {
                    Some(error) => Err(error),
                    None => Ok(()),
                };
            }
            sleep(STATUS_INTERVAL);
        }

        let fallback = run_waydroid(&["container", "stop"]);
        self.active = false;
        let _ = self.child.kill();
        let _ = self.child.wait();
        fallback?;
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("Waydroid container did not stop cleanly\n{last_status}"),
        ))
    }

    fn wait_until_running(&mut self) -> io::Result<()> {
        let mut last_status = String::new();
        for _ in 0..STATUS_ATTEMPTS {
            if let Some(status) = self.child.try_wait()? {
                return Err(io::Error::other(format!(
                    "Waydroid user session exited before the container started: {status}\n{last_status}"
                )));
            }

            last_status = waydroid_status()?;
            if container_state(&last_status) == Some("RUNNING") {
                println!("Waydroid container is RUNNING.");
                return Ok(());
            }
            sleep(STATUS_INTERVAL);
        }

        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("Waydroid container did not reach RUNNING state\n{last_status}"),
        ))
    }
}

impl Drop for DesktopWaydroidSession {
    fn drop(&mut self) {
        if self.active {
            let _ = self.stop();
        }
    }
}

pub fn ensure_root(tool_name: &str) -> io::Result<()> {
    let status = fs::read_to_string("/proc/self/status")?;
    let effective_uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|uids| uids.split_whitespace().nth(1))
        .and_then(|uid| uid.parse::<u32>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "cannot read effective UID"))?;

    if effective_uid != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{tool_name} requires root; run it with sudo"),
        ));
    }
    Ok(())
}

pub fn ensure_container_stopped() -> io::Result<()> {
    let status = waydroid_status()?;
    ensure_container_stopped_status(&status)
}

fn ensure_container_stopped_status(status: &str) -> io::Result<()> {
    if container_state(status) == Some("RUNNING") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid container is running. Stop it first with: waydroid session stop",
        ));
    }
    if session_state(status) == Some("RUNNING") && container_state(status) == Some("FROZEN") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid session is already running but the container is FROZEN. Recover with:\n  sudo target/debug/wroid-native-keyboard --cleanup\n  waydroid session stop\n  sudo systemctl restart waydroid-container",
        ));
    }
    Ok(())
}

pub fn wait_for_android_boot_completed() -> io::Result<()> {
    let mut last_output = String::new();
    for _ in 0..STATUS_ATTEMPTS {
        let output = Command::new("waydroid")
            .args(["shell", "--", "getprop", "sys.boot_completed"])
            .output()?;
        last_output = combined_output(&output);
        if output.status.success() && last_output.trim() == "1" {
            println!("Android boot_completed=1.");
            return Ok(());
        }
        sleep(STATUS_INTERVAL);
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("Android did not report sys.boot_completed=1\n{last_output}"),
    ))
}

pub fn wait_for_android_input_device(device_name: &str) -> io::Result<()> {
    let mut last_output = String::new();
    for _ in 0..STATUS_ATTEMPTS {
        let output = Command::new("waydroid")
            .args(["shell", "--", "getevent", "-pl"])
            .output()?;
        last_output = combined_output(&output);
        if output.status.success() && last_output.contains(device_name) {
            return Ok(());
        }
        sleep(STATUS_INTERVAL);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Android getevent did not list {device_name}; device bridge is not active\n{last_output}"
        ),
    ))
}

pub fn spawn_android_getevent_trace(event_node: &Path) -> io::Result<Child> {
    let event_path = event_node.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "event node path is not UTF-8")
    })?;
    Command::new("waydroid")
        .args(["shell", "--", "getevent", "-lt", event_path])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
}

pub fn stop_child(child: &mut Child) -> io::Result<()> {
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    child.wait()?;
    Ok(())
}

fn waydroid_status() -> io::Result<String> {
    let output = Command::new("waydroid").arg("status").output()?;
    Ok(combined_output(&output))
}

fn status_field<'a>(status: &'a str, field: &str) -> Option<&'a str> {
    status.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == field).then_some(value.trim())
    })
}

fn session_state(status: &str) -> Option<&str> {
    status_field(status, "Session")
}

fn container_state(status: &str) -> Option<&str> {
    status_field(status, "Container")
}

fn waydroid_is_stopped(status: &str) -> bool {
    match container_state(status) {
        Some("STOPPED") => true,
        Some("RUNNING") => false,
        _ => session_state(status) == Some("STOPPED"),
    }
}

fn run_waydroid(arguments: &[&str]) -> io::Result<()> {
    let output = Command::new("waydroid").args(arguments).output()?;
    if output.status.success() {
        return Ok(());
    }

    Err(io::Error::other(format!(
        "waydroid {} failed\n{}",
        arguments.join(" "),
        combined_output(&output)
    )))
}

fn combined_output(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_explicitly_stopped_container() {
        assert!(waydroid_is_stopped("Session:\tSTOPPED\nContainer:\tSTOPPED\n"));
    }

    #[test]
    fn detects_stopped_waydroid_without_container_field() {
        assert!(waydroid_is_stopped("Session:\tSTOPPED\n"));
    }

    #[test]
    fn running_container_wins_over_stopped_session() {
        assert!(!waydroid_is_stopped(
            "Session:\tSTOPPED\nContainer:\tRUNNING\n"
        ));
    }

    #[test]
    fn rejects_running_container_before_native_setup() {
        let error = ensure_container_stopped_status("Session:\tRUNNING\nContainer:\tRUNNING\n")
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("container is running"));
    }

    #[test]
    fn rejects_frozen_container_before_native_setup() {
        let error = ensure_container_stopped_status("Session:\tRUNNING\nContainer:\tFROZEN\n")
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(error.to_string().contains("container is FROZEN"));
        assert!(error.to_string().contains("wroid-native-keyboard --cleanup"));
        assert!(error.to_string().contains("systemctl restart waydroid-container"));
    }

    #[test]
    fn allows_stopped_container_before_native_setup() {
        ensure_container_stopped_status("Session:\tSTOPPED\nContainer:\tSTOPPED\n").unwrap();
    }
}
