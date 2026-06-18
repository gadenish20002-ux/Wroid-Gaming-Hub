use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;

use wroid_core::{Point, Resolution};
use wroid_inject::{
    ensure_container_stopped, ensure_root, remove_default_bridge, spawn_android_getevent_trace,
    stop_child, wait_for_android_input_device, DesktopUser, DesktopWaydroidSession, DeviceConfig,
    InputDeviceNode, InstalledWaydroidBridge, UinputTouchInjector, WROID_TOUCHSCREEN_NAME,
};
use wroid_input::{DirectionalKeyState, EvdevKeyboard, KeyboardAction};
use wroid_runtime::{ContactId, TouchEngine, VirtualJoystick};

#[derive(Debug)]
struct Options {
    keyboard_path: PathBuf,
    width: u32,
    height: u32,
    grab: bool,
    show_ui: bool,
    trace_android: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|argument| argument == "--help") {
        print_usage();
        return Ok(());
    }

    ensure_root("Waydroid keyboard smoke test")?;
    if args.iter().any(|argument| argument == "--cleanup") {
        remove_default_bridge()?;
        println!("Removed the managed Wroid input bridge from the Waydroid LXC config.");
        return Ok(());
    }

    run(parse_options(&args)?)
}

fn run(options: Options) -> Result<(), Box<dyn Error>> {
    ensure_container_stopped()?;
    remove_default_bridge()?;

    let desktop_user = DesktopUser::from_sudo_environment()?;
    let mut keyboard = EvdevKeyboard::open(&options.keyboard_path)?;
    let config = DeviceConfig::new(options.width, options.height)?;
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
        "Keyboard: {} ({})",
        keyboard.name(),
        keyboard.path().display()
    );
    println!(
        "Created {WROID_TOUCHSCREEN_NAME} at {}",
        event_node.display()
    );
    let bridge = InstalledWaydroidBridge::install_default(&input_node)?;
    println!("Installed a temporary, reversible Waydroid LXC input bridge.");

    let mut session = DesktopWaydroidSession::start(desktop_user)?;
    let resolution = Resolution {
        width: options.width,
        height: options.height,
    };
    let joystick = VirtualJoystick::new(
        ContactId::new(1),
        Point {
            x: options.width / 5,
            y: options.height.saturating_mul(4) / 5,
        },
        options.width.min(options.height).max(10) / 10,
        resolution,
    )?;
    let mut engine = TouchEngine::new(injector);
    let mut key_state = DirectionalKeyState::default();
    let mut trace: Option<Child> = None;

    let capture_result = (|| -> Result<(), Box<dyn Error>> {
        wait_for_android_input_device(WROID_TOUCHSCREEN_NAME)?;
        println!("Android detected {WROID_TOUCHSCREEN_NAME}.");

        if options.show_ui {
            session.show_full_ui()?;
            println!("Opened the Waydroid full UI.");
        }
        if options.trace_android {
            trace = Some(spawn_android_getevent_trace(&event_node)?);
            println!("Android getevent tracing is active.");
        }
        if options.grab {
            keyboard.grab()?;
        }

        println!(
            "Controls are live: W/A/S/D move one persistent Android touch contact; Esc exits. Exclusive grab: {}.",
            if keyboard.is_grabbed() {
                "enabled"
            } else {
                "disabled"
            }
        );
        run_keyboard_loop(&mut keyboard, &mut key_state, &joystick, &mut engine)
    })();

    let contact_cleanup_result = release_contact(&mut key_state, &joystick, &mut engine);
    let keyboard_cleanup_result = keyboard.ungrab();
    let trace_cleanup_result = match trace.as_mut() {
        Some(child) => stop_child(child),
        None => Ok(()),
    };
    let stop_result = session.stop();
    let bridge_cleanup_result = bridge.cleanup();

    capture_result?;
    contact_cleanup_result?;
    keyboard_cleanup_result?;
    trace_cleanup_result?;
    stop_result?;
    bridge_cleanup_result?;

    println!("Keyboard capture stopped and the persistent contact was released.");
    println!("Waydroid stopped and the temporary LXC bridge was removed.");
    Ok(())
}

fn run_keyboard_loop(
    keyboard: &mut EvdevKeyboard,
    state: &mut DirectionalKeyState,
    joystick: &VirtualJoystick,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> Result<(), Box<dyn Error>> {
    loop {
        for event in keyboard.next_events()? {
            match state.apply(event) {
                KeyboardAction::DirectionChanged(input) => {
                    joystick.apply(engine, input)?;
                    println!(
                        "direction up={} left={} down={} right={}",
                        input.up, input.left, input.down, input.right
                    );
                }
                KeyboardAction::ExitRequested => return Ok(()),
                KeyboardAction::Ignored => {}
            }
        }
    }
}

fn release_contact(
    state: &mut DirectionalKeyState,
    joystick: &VirtualJoystick,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> Result<(), Box<dyn Error>> {
    if let Some(neutral) = state.release_all() {
        joystick.apply(engine, neutral)?;
    } else {
        joystick.cancel(engine)?;
    }
    Ok(())
}

fn parse_options(args: &[String]) -> Result<Options, Box<dyn Error>> {
    let grab = args.iter().any(|argument| argument == "--grab");
    let show_ui = !args.iter().any(|argument| argument == "--no-ui");
    let trace_android = !args.iter().any(|argument| argument == "--no-trace");

    if let Some(unknown) = args
        .iter()
        .find(|argument| argument.starts_with("--") && !is_supported_flag(argument))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown option: {unknown}"),
        )
        .into());
    }

    let positional = args
        .iter()
        .filter(|argument| !argument.starts_with("--"))
        .collect::<Vec<_>>();
    let keyboard_path = positional.first().copied().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "keyboard event node is required",
        )
    })?;
    if positional.len() > 3 {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "too many positional arguments").into(),
        );
    }

    Ok(Options {
        keyboard_path: PathBuf::from(keyboard_path),
        width: parse_dimension(positional.get(1).map(|value| value.as_str()), 1920, "width")?,
        height: parse_dimension(
            positional.get(2).map(|value| value.as_str()),
            1080,
            "height",
        )?,
        grab,
        show_ui,
        trace_android,
    })
}

fn is_supported_flag(argument: &str) -> bool {
    matches!(
        argument,
        "--grab" | "--no-ui" | "--no-trace" | "--cleanup" | "--help"
    )
}

fn parse_dimension(value: Option<&str>, default: u32, label: &str) -> Result<u32, Box<dyn Error>> {
    let Some(value) = value else {
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

fn print_usage() {
    println!(
        "Usage: wroid-waydroid-keyboard-smoke <keyboard-event-node> [width] [height] [--grab] [--no-ui] [--no-trace]"
    );
    println!(
        "Example: sudo ./target/release/wroid-waydroid-keyboard-smoke /dev/input/event7 1920 1050 --grab"
    );
    println!("Recovery: sudo ./target/release/wroid-waydroid-keyboard-smoke --cleanup");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_live_options() {
        let options = parse_options(&["/dev/input/event7".to_owned()]).unwrap();

        assert_eq!(options.keyboard_path, PathBuf::from("/dev/input/event7"));
        assert_eq!(options.width, 1920);
        assert_eq!(options.height, 1080);
        assert!(!options.grab);
        assert!(options.show_ui);
        assert!(options.trace_android);
    }

    #[test]
    fn parses_safety_and_diagnostics_flags() {
        let options = parse_options(&[
            "/dev/input/event7".to_owned(),
            "1600".to_owned(),
            "900".to_owned(),
            "--grab".to_owned(),
            "--no-ui".to_owned(),
            "--no-trace".to_owned(),
        ])
        .unwrap();

        assert_eq!(options.width, 1600);
        assert_eq!(options.height, 900);
        assert!(options.grab);
        assert!(!options.show_ui);
        assert!(!options.trace_android);
    }

    #[test]
    fn rejects_unknown_options() {
        let error =
            parse_options(&["/dev/input/event7".to_owned(), "--unsafe".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("unknown option"));
    }
}
