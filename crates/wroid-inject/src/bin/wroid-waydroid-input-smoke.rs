use std::error::Error;
use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::Duration;

use wroid_core::Point;
use wroid_inject::{
    ensure_container_stopped, ensure_root, remove_default_bridge, wait_for_android_input_device,
    DesktopUser, DesktopWaydroidSession, DeviceConfig, InputDeviceNode, InstalledWaydroidBridge,
    UinputTouchInjector, WROID_TOUCHSCREEN_NAME,
};
use wroid_runtime::{ContactId, TouchEngine};

const CAPTURE_EVENT_COUNT: &str = "13";

fn main() -> Result<(), Box<dyn Error>> {
    ensure_root("Waydroid input smoke test")?;

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

    println!(
        "Created {WROID_TOUCHSCREEN_NAME} at {}",
        event_node.display()
    );
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

fn verify_android_input(
    event_node: &Path,
    width: u32,
    height: u32,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> Result<(), Box<dyn Error>> {
    wait_for_android_input_device(WROID_TOUCHSCREEN_NAME)?;

    let event_path = event_node.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "event node path is not UTF-8")
    })?;
    let capture = Command::new("waydroid")
        .args([
            "shell",
            "--",
            "getevent",
            "-c",
            CAPTURE_EVENT_COUNT,
            event_path,
        ])
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
    let expected_start = format!("0003 0035 {:08x}", start.x);
    let expected_end = format!("0003 0035 {:08x}", end.x);
    let has_down = captured.contains("0001 014a 00000001")
        && captured.contains("0003 0039 00000001")
        && captured.contains(&expected_start);
    let has_move = captured.contains(&expected_end);
    let has_up = captured.contains("0001 014a 00000000")
        && captured.contains("0003 0039 ffffffff");
    let has_sync = captured.contains("0000 0000 00000000");
    if captured.trim().is_empty() || !(has_down && has_move && has_up && has_sync) {
        return Err(io::Error::other(format!(
            "Android getevent did not capture a complete down/move/up sequence\n{captured}"
        ))
        .into());
    }

    println!("Android getevent capabilities include {WROID_TOUCHSCREEN_NAME}.");
    println!("Captured Android input events:\n{}", captured.trim());
    Ok(())
}

fn combined_output(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
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
