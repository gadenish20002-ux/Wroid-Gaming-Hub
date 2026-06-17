use std::error::Error;
use std::fs;
use std::io;
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::Duration;

use wroid_core::Point;
use wroid_inject::{
    remove_default_bridge, DeviceConfig, InputDeviceNode, InstalledWaydroidBridge,
    UinputTouchInjector,
};
use wroid_runtime::{ContactId, TouchEngine};

const DEVICE_NAME: &str = "Wroid Gaming Touchscreen";

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

    let width = parse_dimension(1, 1920, "width")?;
    let height = parse_dimension(2, 1080, "height")?;
    let config = DeviceConfig::new(width, height)?;
    let mut injector = UinputTouchInjector::open(config)?;
    let event_node = injector
        .sink_mut()
        .event_nodes()?
        .into_iter()
        .find(|path| {
            path.parent() == Some(std::path::Path::new("/dev/input"))
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

    if let Err(error) = run_waydroid(&["container", "start"]) {
        let _ = bridge.cleanup();
        return Err(error.into());
    }

    let mut engine = TouchEngine::new(injector);
    let verification = verify_android_input(&event_node, width, height, &mut engine);
    let stop_result = run_waydroid(&["container", "stop"]);
    let cleanup_result = bridge.cleanup();

    verification?;
    stop_result?;
    cleanup_result?;

    println!("Waydroid detected the virtual touchscreen and Android getevent received touch data.");
    println!("The container was stopped and the temporary LXC bridge was removed.");
    Ok(())
}

fn verify_android_input(
    event_node: &std::path::Path,
    width: u32,
    height: u32,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> Result<(), Box<dyn Error>> {
    let capabilities = wait_for_getevent_capabilities()?;
    if !capabilities.contains(DEVICE_NAME) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Android getevent did not list {DEVICE_NAME}; device bridge is not active\n{capabilities}"
            ),
        )
        .into());
    }

    let event_path = event_node.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "event node path is not UTF-8")
    })?;
    let mut capture = Command::new("waydroid")
        .args(["shell", "--", "getevent", "-c", "10", event_path])
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

    let output = capture.wait_with_output()?;
    let captured = combined_output(&output);
    if !output.status.success() || captured.trim().is_empty() {
        return Err(io::Error::other(format!(
            "Android getevent did not capture injected events\n{captured}"
        ))
        .into());
    }

    println!("Android getevent capabilities include {DEVICE_NAME}.");
    println!("Captured Android input events:\n{}", captured.trim());
    Ok(())
}

fn wait_for_getevent_capabilities() -> io::Result<String> {
    let mut last_output = String::new();
    for _ in 0..30 {
        let output = Command::new("waydroid")
            .args(["shell", "--", "getevent", "-pl"])
            .output()?;
        last_output = combined_output(&output);
        if output.status.success() {
            return Ok(last_output);
        }
        sleep(Duration::from_secs(1));
    }

    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!("Waydroid shell did not become ready\n{last_output}"),
    ))
}

fn ensure_container_stopped() -> io::Result<()> {
    let output = Command::new("waydroid").arg("status").output()?;
    let status = combined_output(&output);
    let running = status.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(key, value)| key.trim() == "Container" && value.trim() == "RUNNING")
    });
    if running {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Waydroid container is running. Stop it first with: waydroid session stop && sudo waydroid container stop",
        ));
    }
    Ok(())
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
