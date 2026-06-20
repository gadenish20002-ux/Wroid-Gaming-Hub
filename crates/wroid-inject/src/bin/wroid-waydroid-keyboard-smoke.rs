use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use wroid_core::{Point, Resolution};
use wroid_inject::{
    ensure_container_stopped, ensure_root, remove_default_bridge, spawn_android_getevent_trace,
    stop_child, wait_for_android_input_device, DesktopUser, DesktopWaydroidSession, DeviceConfig,
    InputDeviceNode, InstalledWaydroidBridge, UinputTouchInjector, WROID_TOUCHSCREEN_NAME,
};
use wroid_input::{
    DirectionalKeyState, EvdevKeyboard, KeyboardAction, KeyboardDeviceError, KeyboardEvent,
};
use wroid_runtime::{ContactId, DirectionalInput, TouchEngine, VirtualJoystick};

const DEFAULT_REAFFIRM_INTERVAL: Duration = Duration::from_millis(50);
const DEFAULT_HOLD_LOG_INTERVAL: Duration = Duration::from_millis(1_000);
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
struct Options {
    keyboard_path: PathBuf,
    width: u32,
    height: u32,
    grab: bool,
    show_ui: bool,
    trace_android: bool,
    reaffirm_interval: Option<Duration>,
    hold_log_interval: Option<Duration>,
}

#[derive(Debug, Default)]
struct HoldTimers {
    started_at: Option<Instant>,
    next_reaffirm_at: Option<Instant>,
    next_log_at: Option<Instant>,
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
            "Controls are live: W/A/S/D move one persistent Android touch contact; Esc exits. Exclusive grab: {}. Reaffirm: {}. Hold log: {}.",
            if keyboard.is_grabbed() {
                "enabled"
            } else {
                "disabled"
            },
            interval_label(options.reaffirm_interval),
            interval_label(options.hold_log_interval),
        );
        let reader = KeyboardReader::spawn(keyboard);
        let loop_result = run_keyboard_loop(
            reader.receiver(),
            &mut key_state,
            &joystick,
            &mut engine,
            &options,
        );
        if loop_result.is_ok() {
            reader.join()?;
        }
        loop_result
    })();

    let contact_cleanup_result = release_contact(&mut key_state, &joystick, &mut engine);
    let trace_cleanup_result = match trace.as_mut() {
        Some(child) => stop_child(child),
        None => Ok(()),
    };
    let stop_result = session.stop();
    let bridge_cleanup_result = bridge.cleanup();

    capture_result?;
    contact_cleanup_result?;
    trace_cleanup_result?;
    stop_result?;
    bridge_cleanup_result?;

    println!("Keyboard capture stopped and the persistent contact was released.");
    println!("Waydroid stopped and the temporary LXC bridge was removed.");
    Ok(())
}

struct KeyboardReader {
    receiver: Receiver<KeyboardEvent>,
    handle: JoinHandle<Result<(), KeyboardDeviceError>>,
}

impl KeyboardReader {
    fn spawn(mut keyboard: EvdevKeyboard) -> Self {
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || loop {
            for event in keyboard.next_events()? {
                let exit = matches!(state_preview(event), KeyboardAction::ExitRequested);
                if sender.send(event).is_err() {
                    return Ok(());
                }
                if exit {
                    return Ok(());
                }
            }
        });

        Self { receiver, handle }
    }

    fn receiver(&self) -> &Receiver<KeyboardEvent> {
        &self.receiver
    }

    fn join(self) -> Result<(), Box<dyn Error>> {
        match self.handle.join() {
            Ok(result) => result.map_err(|error| error.into()),
            Err(_) => Err(io::Error::other("keyboard reader thread panicked").into()),
        }
    }
}

fn state_preview(event: KeyboardEvent) -> KeyboardAction {
    let mut state = DirectionalKeyState::default();
    state.apply(event)
}

fn run_keyboard_loop(
    receiver: &Receiver<KeyboardEvent>,
    state: &mut DirectionalKeyState,
    joystick: &VirtualJoystick,
    engine: &mut TouchEngine<UinputTouchInjector>,
    options: &Options,
) -> Result<(), Box<dyn Error>> {
    let mut timers = HoldTimers::default();

    loop {
        match receiver.recv_timeout(next_timeout(&timers)) {
            Ok(event) => {
                if handle_keyboard_event(event, state, joystick, engine, options, &mut timers)? {
                    return Ok(());
                }
                while let Ok(event) = receiver.try_recv() {
                    if handle_keyboard_event(event, state, joystick, engine, options, &mut timers)? {
                        return Ok(());
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "keyboard reader stopped before Esc was pressed",
                )
                .into());
            }
        }

        service_hold_timers(state, joystick, engine, options, &mut timers)?;
    }
}

fn handle_keyboard_event(
    event: KeyboardEvent,
    state: &mut DirectionalKeyState,
    joystick: &VirtualJoystick,
    engine: &mut TouchEngine<UinputTouchInjector>,
    options: &Options,
    timers: &mut HoldTimers,
) -> Result<bool, Box<dyn Error>> {
    match state.apply(event) {
        KeyboardAction::DirectionChanged(input) => {
            joystick.apply(engine, input)?;
            println!(
                "direction up={} left={} down={} right={}",
                input.up, input.left, input.down, input.right
            );
            refresh_hold_timers(input, options, timers);
            Ok(false)
        }
        KeyboardAction::ExitRequested => Ok(true),
        KeyboardAction::Ignored => Ok(false),
    }
}

fn refresh_hold_timers(input: DirectionalInput, options: &Options, timers: &mut HoldTimers) {
    if input == DirectionalInput::default() {
        *timers = HoldTimers::default();
        return;
    }

    let now = Instant::now();
    if timers.started_at.is_none() {
        timers.started_at = Some(now);
    }
    timers.next_reaffirm_at = options.reaffirm_interval.map(|interval| now + interval);
    timers.next_log_at = options.hold_log_interval.map(|interval| now + interval);
}

fn service_hold_timers(
    state: &DirectionalKeyState,
    joystick: &VirtualJoystick,
    engine: &mut TouchEngine<UinputTouchInjector>,
    options: &Options,
    timers: &mut HoldTimers,
) -> Result<(), Box<dyn Error>> {
    if state.current() == DirectionalInput::default() {
        timers.next_reaffirm_at = None;
        timers.next_log_at = None;
        return Ok(());
    }

    let now = Instant::now();
    if let (Some(due), Some(interval)) = (timers.next_reaffirm_at, options.reaffirm_interval) {
        if now >= due {
            if let Some(position) = engine.state().position(joystick.contact_id()) {
                engine.move_contact(joystick.contact_id(), position)?;
            }
            timers.next_reaffirm_at = Some(now + interval);
        }
    }

    if let (Some(due), Some(interval), Some(started)) = (
        timers.next_log_at,
        options.hold_log_interval,
        timers.started_at,
    ) {
        if now >= due {
            let input = state.current();
            println!(
                "holding up={} left={} down={} right={} for {}ms",
                input.up,
                input.left,
                input.down,
                input.right,
                now.duration_since(started).as_millis()
            );
            timers.next_log_at = Some(now + interval);
        }
    }

    Ok(())
}

fn next_timeout(timers: &HoldTimers) -> Duration {
    [timers.next_reaffirm_at, timers.next_log_at]
        .into_iter()
        .flatten()
        .min()
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(IDLE_POLL_INTERVAL)
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
    let mut positional = Vec::new();
    let mut grab = false;
    let mut show_ui = true;
    let mut trace_android = true;
    let mut reaffirm_interval = Some(DEFAULT_REAFFIRM_INTERVAL);
    let mut hold_log_interval = Some(DEFAULT_HOLD_LOG_INTERVAL);

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--grab" => grab = true,
            "--no-ui" => show_ui = false,
            "--no-trace" => trace_android = false,
            "--no-reaffirm" => reaffirm_interval = None,
            "--no-hold-log" => hold_log_interval = None,
            "--reaffirm-ms" => {
                index += 1;
                reaffirm_interval = Some(parse_millis_arg(args.get(index), "--reaffirm-ms")?);
            }
            "--hold-log-ms" => {
                index += 1;
                hold_log_interval = Some(parse_millis_arg(args.get(index), "--hold-log-ms")?);
            }
            argument if argument.starts_with("--") => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown option: {argument}"),
                )
                .into());
            }
            argument => positional.push(argument.to_owned()),
        }
        index += 1;
    }

    let keyboard_path = positional.first().ok_or_else(|| {
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
        width: parse_dimension(positional.get(1).map(String::as_str), 1920, "width")?,
        height: parse_dimension(positional.get(2).map(String::as_str), 1080, "height")?,
        grab,
        show_ui,
        trace_android,
        reaffirm_interval,
        hold_log_interval,
    })
}

fn parse_millis_arg(value: Option<&String>, flag: &str) -> Result<Duration, Box<dyn Error>> {
    let value = value.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{flag} requires a millisecond value"),
        )
    })?;
    let millis = value.parse::<u64>().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid value for {flag}: {error}"),
        )
    })?;
    if millis == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{flag} must be greater than zero"),
        )
        .into());
    }
    Ok(Duration::from_millis(millis))
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

fn interval_label(interval: Option<Duration>) -> String {
    interval
        .map(|interval| format!("{}ms", interval.as_millis()))
        .unwrap_or_else(|| "disabled".to_owned())
}

fn print_usage() {
    println!(
        "Usage: wroid-waydroid-keyboard-smoke <keyboard-event-node> [width] [height] [--grab] [--no-ui] [--no-trace] [--reaffirm-ms N|--no-reaffirm] [--hold-log-ms N|--no-hold-log]"
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
        assert_eq!(options.reaffirm_interval, Some(DEFAULT_REAFFIRM_INTERVAL));
        assert_eq!(options.hold_log_interval, Some(DEFAULT_HOLD_LOG_INTERVAL));
    }

    #[test]
    fn parses_safety_diagnostics_and_hold_flags() {
        let options = parse_options(&[
            "/dev/input/event7".to_owned(),
            "1600".to_owned(),
            "900".to_owned(),
            "--grab".to_owned(),
            "--no-ui".to_owned(),
            "--no-trace".to_owned(),
            "--reaffirm-ms".to_owned(),
            "75".to_owned(),
            "--hold-log-ms".to_owned(),
            "250".to_owned(),
        ])
        .unwrap();

        assert_eq!(options.width, 1600);
        assert_eq!(options.height, 900);
        assert!(options.grab);
        assert!(!options.show_ui);
        assert!(!options.trace_android);
        assert_eq!(options.reaffirm_interval, Some(Duration::from_millis(75)));
        assert_eq!(options.hold_log_interval, Some(Duration::from_millis(250)));
    }

    #[test]
    fn can_disable_hold_compatibility_features() {
        let options = parse_options(&[
            "/dev/input/event7".to_owned(),
            "--no-reaffirm".to_owned(),
            "--no-hold-log".to_owned(),
        ])
        .unwrap();

        assert_eq!(options.reaffirm_interval, None);
        assert_eq!(options.hold_log_interval, None);
    }

    #[test]
    fn rejects_unknown_options() {
        let error =
            parse_options(&["/dev/input/event7".to_owned(), "--unsafe".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("unknown option"));
    }

    #[test]
    fn rejects_zero_millisecond_intervals() {
        let error = parse_options(&[
            "/dev/input/event7".to_owned(),
            "--reaffirm-ms".to_owned(),
            "0".to_owned(),
        ])
        .unwrap_err();

        assert!(error.to_string().contains("greater than zero"));
    }
}
