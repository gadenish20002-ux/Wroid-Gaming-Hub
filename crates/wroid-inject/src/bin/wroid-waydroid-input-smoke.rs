use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread::sleep;
use std::time::Duration;

use wroid_core::Point;
use wroid_inject::{
    remove_default_bridge, DeviceConfig, InputDeviceNode, InstalledWaydroidBridge,
    UinputTouchInjector,
};
use wroid_runtime::{ContactId, TouchEngine};

const DEVICE_NAME: &str = "Wroid Gaming Touchscreen";
const STATUS_ATTEMPTS: usize = 60;
const STATUS_INTERVAL: Duration = Duration::from_millis(500);

fn main() -> Result<(), Box<dyn Error>> {
    ensure_root()?;

    if std::env::args().any(|argument| argument == "--cleanup") {
        remove_default_bridge()?;
        println!("Removed the managed Wroid input bridge from the Waydroid LXC config.");
        return Ok(());
    }

    run_smoke()
}

fn run_smoke() -> Result<(), Box<dyn Error>> {
    ensure_container_stopped()?;
    remove_default_bridge()?;

    let desktop_user = DesktopUser::from_sudo_environment()?;
    let width = parse_dimension(1, 1920, "width")?;
    let height = parse_dimension(2, 1080, "height")?;
    let config = DeviceConfig::new(width, height)?;
    let mut injector = UinputTouchInjector::open(config)?;
    let event_node = injector
        .sink_mut()
        .event_nodes()?
        .into_iter()
        .find(|path| {
            path.parent() == Some(Path::new("/dev/input"))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("event"))
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "uinput event node not found"))?;
    let input_node = InputDeviceNode::from_path(&event_node)?;

    println!("Created {DEVICE_NAME} at {}", event_node.display());
    let bridge = InstalledWaydroidBridge::install_default(&input_node)?;
    println!("Installed a temporary, reversible Waydroid LXC input bridge.");

    let mut session = DesktopWaydroidSession::start(desktop_user)?;
    let mut engine = TouchEngine::new(injector);
    let verification = verify_android_input(&event_node, width, height, &mut engine);
    let stop_result = session.stop();
    let cleanup_result = bridge.cleanup();

    verification?;
    stop_result?;
    cleanup_result?;

    println!("Waydroid detected the virtual touchscreen and Android getevent received touch data.");
    println!(
        "The user session and container were stopped, and the temporary LXC bridge was removed."
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct DesktopUser {
    name: String,
    home: PathBuf,
    runtime_dir: PathBuf,
    wayland_display: OsString,
}

impl DesktopUser {
    fn from_sudo_environment() -> io::Result<Self> {
        let name = std::env::var("SUDO_USER").map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SUDO_USER is not set; run this integration test with sudo from the desktop user session",
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
}

struct DesktopWaydroidSession {
    child: Child,
    user: DesktopUser,
    active: bool,
}

impl DesktopWaydroidSession {
    fn start(user: DesktopUser) -> io::Result<Self> {
        println!(
            "Starting Waydroid session as desktop user {} on {}...",
            user.name,
            user.wayland_display.to_string_lossy()
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

    fn stop(&mut self) -> io::Result<()> {
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
            if container_state(&last_status) == Some("STOPPED") {
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
}

impl Drop for DesktopWaydroidSession {
    fn drop(&mut self) {
        if self.active {
            let _ = self.stop();
        }
    }
}

fn verify_android_input(
    event_node: &Path,
    width: u32,
    height: u32,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> Result<(), Box<dyn Error>> {
    wait_for_android_device()?;

    let event_path = event_node.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "event node path is not UTF-8")
    })?;
    let mut capture = Command::new("waydroid")
        .args(["shell", "--", "getevent", event_path])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    sleep(Duration::from_millis(250));
    let contact = ContactId::new(1);
    let start = Point {
        x: width / 3,
        y: height / 2,
    };
    let end = Point {
        x: width * 2 / 3,
        y: height / 2,
    };
    engine.begin_contact(contact, start)?;
    sleep(Duration::from_millis(50));
    engine.move_contact(contact, end)?;
    sleep(Duration::from_millis(50));
    engine.end_contact(contact)?;

    sleep(Duration::from_millis(500));
    if capture.try_wait()?.is_none() {
        capture.kill()?;
    }
    let output = capture.wait_with_output()?;
    let captured = combined_output(&output);
    let has_touch = captured.contains("0003 0039")
        && captured.contains("0001 014a")
        && captured.contains("0000 0000");
    if captured.trim().is_empty() || !has_touch {
        return Err(io::Error::other(format!(
            "Android getevent did not capture injected events\n{captured}"
        ))
        .into());
    }

    println!("Android getevent capabilities include {DEVICE_NAME}.");
    println!("Captured Android input events:\n{}", captured.trim());
    Ok(())
}

fn wait_for_android_device() -> io::Result<()> {
    let mut last_output = String::new();
    for _ in 0..STATUS_ATTEMPTS {
        let output = Command::new("waydroid")
            .args(["shell", "--", "getevent", "-pl"])
            .output()?;
        last_output = combined_output(&output);
        if output.status.success() && last_output.contains(DEVICE_NAME) {
            return Ok(());
        }
        sleep(STATUS_INTERVAL);
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "Android getevent did not list {DEVICE_NAME}; device bridge is not active\n{last_output}"
        ),
    ))
}

fn ensure_container_stopped() -> io::Result<()> {
    let status = waydroid_status()?;
    if container_state(&status) == Some("RUNNING") {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid container is running. Stop it first with: waydroid session stop",
        ));
    }
    Ok(())
}

fn waydroid_status() -> io::Result<String> {
    let output = Command::new("waydroid").arg("status").output()?;
    Ok(combined_output(&output))
}

fn container_state(status: &str) -> Option<&str> {
    status.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "Container").then_some(value.trim())
    })
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

fn home_directory(user: &str) -> io::Result<PathBuf> {
    let passwd = fs::read_to_string("/etc/passwd")?;
    passwd
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(':');
            let name = fields.next()?;
            let _password = fields.next()?;
            let _uid = fields.next()?;
            let _gid = fields.next()?;
            let _gecos = fields.next()?;
            let home = fields.next()?;
            (name == user).then(|| PathBuf::from(home))
        })
        .next()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("desktop user {user} is missing from /etc/passwd"),
            )
        })
}

fn detect_wayland_display(runtime_dir: &Path) -> io::Result<OsString> {
    if let Some(display) = std::env::var_os("WAYLAND_DISPLAY") {
        if runtime_dir.join(&display).exists() {
            return Ok(display);
        }
    }

    let mut candidates = fs::read_dir(runtime_dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let is_wayland = name.to_string_lossy().starts_with("wayland-");
            let is_socket = entry
                .file_type()
                .map(|file_type| file_type.is_socket())
                .unwrap_or(false);
            (is_wayland && is_socket).then_some(name)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no Wayland socket found in {}", runtime_dir.display()),
        )
    })
}

fn ensure_root() -> io::Result<()> {
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
            "Waydroid input smoke test requires root; run it with sudo",
        ));
    }
    Ok(())
}

fn parse_dimension(index: usize, default: u32, label: &str) -> Result<u32, Box<dyn Error>> {
    let Some(value) = std::env::args().nth(index) else {
        return Ok(default);
    };
    value.parse::<u32>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {label} '{value}': {error}"),
        )
        .into()
    })
}
