use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::io::{self, BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use crate::{
    ensure_container_stopped, ensure_root, remove_default_bridge, wait_for_android_input_device,
    BridgeHelperCommand, DesktopUser, DesktopWaydroidSession, DeviceConfig, InputDeviceNode,
    InstalledWaydroidBridge, PrivilegedBridgeHelper, UinputTouchInjector, WaydroidBridgeLease,
    WROID_TOUCHSCREEN_NAME,
};
use wroid_core::profile_v2::{InputV2, JoystickMode, ProfileV2};
use wroid_core::{Point, Resolution};
use wroid_input::mouse::{EvdevMouse, MouseButtonTransition, MouseEvent, RelativeMouseMotion};
use wroid_input::{EvdevKeyboard, HostKey, HostKeyEvent, KeyTransition};
use wroid_runtime::{
    ContactId, DirectionalInput, MouseAimController, MouseAimDelta, MouseAimUpdate,
    RuntimeControlAction, RuntimeControlPlan, TouchEngine, TouchEvent, TouchFrame,
    TouchInjectionError, TouchInjector, TouchPhase,
};

const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
const IDLE_POLL: Duration = Duration::from_millis(50);
const INPUT_READER_POLL: Duration = Duration::from_millis(1);
const INPUT_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const FOCUS_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;
const SIGNAL_ERROR: usize = usize::MAX;

static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);

type SignalHandler = extern "C" fn(i32);

unsafe extern "C" {
    fn signal(signum: i32, handler: SignalHandler) -> usize;
}

pub type GameSessionResult<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LatencyMetrics {
    pub samples: u64,
    pub p50_micros: u64,
    pub p95_micros: u64,
    pub p99_micros: u64,
    pub max_micros: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GameSessionReport {
    pub frames_submitted: u64,
    pub peak_simultaneous_contacts: u64,
    pub mouse_aim_recenters: u64,
    pub reader_to_inject: LatencyMetrics,
    pub kernel_to_inject: Option<LatencyMetrics>,
    pub rejected_kernel_timestamps: u64,
}

pub fn run_game_session_cli(args: impl IntoIterator<Item = String>) -> GameSessionResult<()> {
    let Some(options) = GameSessionOptions::parse(args)? else {
        print_usage();
        return Ok(());
    };

    if options.cleanup {
        ensure_root("Wroid unified game session cleanup")?;
        let _bridge_lease = WaydroidBridgeLease::acquire_default("game session recovery")?;
        remove_default_bridge()?;
        println!("Removed the managed Wroid input bridge.");
        return Ok(());
    }

    run_game_session(options).map(|_| ())
}

pub fn run_game_session(options: GameSessionOptions) -> GameSessionResult<GameSessionReport> {
    let is_root = ensure_root("Wroid unified game session").is_ok();
    let lease_owner = format!(
        "game session {}",
        options
            .profile_path
            .as_deref()
            .map_or_else(|| "<unknown>".into(), |path| path.display().to_string())
    );
    let _bridge_lease = is_root
        .then(|| WaydroidBridgeLease::acquire_default(&lease_owner))
        .transpose()?;
    install_interrupt_handler()?;
    ensure_container_stopped()?;
    if is_root {
        remove_default_bridge()?;
    }

    let profile_path = required_path(&options.profile_path, "profile")?;
    let keyboard_path = required_path(&options.keyboard_path, "keyboard")?;
    let resolution = Resolution {
        width: options.width,
        height: options.height,
    };

    let profile = ProfileV2::load_from_path(profile_path)?;
    if let Err(error) = profile.validate() {
        return Err(invalid_input(format!(
            "invalid profile v2: {}",
            error.errors.join("; ")
        )));
    }
    let plan = RuntimeControlPlan::from_profile_v2(&profile, resolution)?;

    let mut keyboard = EvdevKeyboard::open(keyboard_path)?;
    let mut mouse = options
        .mouse_path
        .as_deref()
        .map(EvdevMouse::open)
        .transpose()?;
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
    let mut bridge = if is_root {
        SessionBridge::InProcess(InstalledWaydroidBridge::install_default(&input_node)?)
    } else {
        let helper = options.bridge_helper.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "rootless game session requires the typed Wroid bridge helper",
            )
        })?;
        SessionBridge::Helper(PrivilegedBridgeHelper::start(helper, &event_node)?)
    };
    let desktop_user = DesktopUser::from_session_environment()?;
    let mut waydroid = DesktopWaydroidSession::start(desktop_user)?;

    let session_result = (|| -> GameSessionResult<GameSessionReport> {
        waydroid.wait_until_android_ready()?;
        if waydroid.configure_resolution(options.width, options.height)? {
            waydroid.restart()?;
            waydroid.wait_until_android_ready()?;
        }
        waydroid.confirm_resolution(options.width, options.height)?;
        bridge.verify_android_input()?;
        if options.show_ui {
            waydroid.show_full_ui()?;
        }
        if options.launch_package {
            waydroid.launch_package(&plan.package_name)?;
            println!("Launched Android package {}.", plan.package_name);
        }
        let focus_connection = if options.grab {
            options
                .focus_socket
                .as_deref()
                .and_then(|path| match FocusConnection::connect(path) {
                    Ok(connection) => Some(connection),
                    Err(error) => {
                        eprintln!(
                            "Focus protection unavailable: could not connect to {}: {error}",
                            path.display()
                        );
                        eprintln!(
                            "Use F12 to release or recapture input; Ctrl+Esc stops the session."
                        );
                        None
                    }
                })
        } else {
            None
        };
        let focus_protected = focus_connection.is_some();
        let initially_focused = focus_connection
            .as_ref()
            .is_none_or(|connection| connection.focused);

        keyboard.set_nonblocking(true)?;
        if let Some(mouse) = mouse.as_mut() {
            mouse.set_nonblocking(true)?;
        }
        if options.grab && initially_focused {
            keyboard.grab()?;
            if let Some(mouse) = mouse.as_mut() {
                mouse.grab()?;
            }
        }

        println!("Unified game session is live.");
        println!("Profile: {} ({})", plan.profile_name, plan.package_name);
        println!(
            "Keyboard: {} ({})",
            keyboard.name(),
            keyboard.path().display()
        );
        if let Some(mouse) = mouse.as_ref() {
            println!("Mouse: {} ({})", mouse.name(), mouse.path().display());
        } else {
            println!("Mouse: not required by this profile");
        }
        println!("Android touchscreen: {}", event_node.display());
        println!("Controls: {}", plan.controls.len());
        if options.trace_input {
            println!("Input tracing: enabled");
        }
        if focus_protected {
            println!(
                "Focus protection: {}.",
                if initially_focused {
                    "Waydroid focused; devices captured"
                } else {
                    "Waydroid is not focused; devices released"
                }
            );
        } else if options.grab {
            println!("Focus protection: compositor fallback; F12 controls capture manually.");
        } else {
            println!("Focus protection: capture disabled by --no-grab.");
        }
        println!("Press F12 to release/reacquire input. Press Ctrl+Esc to stop.");

        let (sender, receiver) = mpsc::channel();
        let input_active = !focus_protected || initially_focused;
        let keyboard_control = spawn_keyboard_reader(keyboard, sender.clone(), input_active);
        let mut mouse_control = None;
        if let Some(mouse) = mouse {
            mouse_control = Some(spawn_mouse_reader(mouse, sender.clone(), input_active));
        }
        let focus_receiver = focus_connection.map(|connection| {
            let (focus_sender, focus_receiver) = mpsc::channel();
            spawn_focus_reader(connection.reader, focus_sender);
            focus_receiver
        });
        let input_readers = InputReaderControls {
            keyboard: keyboard_control,
            mouse: mouse_control,
        };

        let mut runtime = UnifiedRuntime::new(
            plan,
            SessionMetricsInjector::new(injector),
            options.trace_input,
        )?;
        if initially_focused {
            runtime.start()?;
        }
        let loop_result = run_event_loop(
            &receiver,
            &input_readers,
            &mut runtime,
            EventLoopOptions {
                trace_input: options.trace_input,
                exit_after: options.exit_after,
                focus_protected,
                focused: initially_focused,
                focus_receiver: focus_receiver.as_ref(),
            },
        );
        let cleanup_result = runtime.stop();
        let exit = loop_result?;
        cleanup_result?;
        let report = runtime.report();
        UnifiedRuntime::print_report(&report);
        match exit {
            EventLoopExit::ExitHotkeyRequested => println!("Stop requested by Ctrl+Esc."),
            EventLoopExit::InterruptRequested => {
                println!("Stop requested by Ctrl+C or a termination signal.")
            }
            EventLoopExit::TimeLimitReached => println!("Diagnostic time limit reached."),
        }
        Ok(report)
    })();

    let stop_result = waydroid.stop();
    let bridge_result = bridge.cleanup(stop_result.is_ok());
    let report = combine_session_results(session_result, stop_result, bridge_result)?;
    println!("Unified game session stopped cleanly.");
    Ok(report)
}

enum SessionBridge {
    InProcess(InstalledWaydroidBridge),
    Helper(PrivilegedBridgeHelper),
}

impl SessionBridge {
    fn verify_android_input(&mut self) -> io::Result<()> {
        match self {
            Self::InProcess(_) => wait_for_android_input_device(WROID_TOUCHSCREEN_NAME),
            Self::Helper(helper) => helper.verify_android_input(),
        }
    }

    fn cleanup(self, waydroid_stopped: bool) -> io::Result<()> {
        match self {
            Self::InProcess(bridge) => bridge.cleanup(),
            Self::Helper(helper) => helper.finish(waydroid_stopped),
        }
    }
}

fn combine_session_results<T>(
    session_result: GameSessionResult<T>,
    stop_result: io::Result<()>,
    bridge_result: io::Result<()>,
) -> GameSessionResult<T> {
    let (session_value, session_error) = match session_result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    };
    let stop_error = stop_result.err();
    let bridge_error = bridge_result.err();
    let error_count = usize::from(session_error.is_some())
        + usize::from(stop_error.is_some())
        + usize::from(bridge_error.is_some());

    if error_count == 0 {
        return Ok(session_value.expect("an error-free session has a value"));
    }
    if error_count == 1 {
        if let Some(error) = session_error {
            return Err(error);
        }
        if let Some(error) = stop_error {
            return Err(error.into());
        }
        return Err(bridge_error
            .expect("one error exists and it is the bridge error")
            .into());
    }

    let mut failures = Vec::with_capacity(error_count);
    if let Some(error) = session_error {
        failures.push(format!("game session failed: {error}"));
    }
    if let Some(error) = stop_error {
        failures.push(format!("Waydroid shutdown failed: {error}"));
    }
    if let Some(error) = bridge_error {
        failures.push(format!("input bridge cleanup failed: {error}"));
    }
    Err(io::Error::other(failures.join("\nAdditionally, ")).into())
}

fn required_path<'a>(path: &'a Option<PathBuf>, label: &str) -> GameSessionResult<&'a Path> {
    path.as_deref()
        .ok_or_else(|| invalid_input(format!("missing {label} path")))
}

#[derive(Debug)]
enum HostEvent {
    Keyboard {
        events: Vec<HostKeyEvent>,
        received_at: Instant,
        kernel_timestamp: Option<SystemTime>,
    },
    Mouse {
        events: Vec<MouseEvent>,
        received_at: Instant,
        kernel_timestamp: Option<SystemTime>,
    },
    ReaderFailed {
        reader: &'static str,
        message: String,
    },
}

#[derive(Debug)]
enum FocusEvent {
    Changed(bool),
    Unavailable(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventLoopExit {
    ExitHotkeyRequested,
    InterruptRequested,
    TimeLimitReached,
}

enum InputReaderCommand {
    SetCapture {
        enabled: bool,
        reply: Sender<Result<(), String>>,
    },
}

struct InputReaderControls {
    keyboard: Sender<InputReaderCommand>,
    mouse: Option<Sender<InputReaderCommand>>,
}

impl InputReaderControls {
    fn set_capture(&self, enabled: bool) -> GameSessionResult<()> {
        set_reader_capture(&self.keyboard, "keyboard", enabled)?;
        if let Some(mouse) = self.mouse.as_ref() {
            set_reader_capture(mouse, "mouse", enabled)?;
        }
        Ok(())
    }
}

fn set_reader_capture(
    control: &Sender<InputReaderCommand>,
    reader: &'static str,
    enabled: bool,
) -> GameSessionResult<()> {
    let (reply, response) = mpsc::channel();
    control
        .send(InputReaderCommand::SetCapture { enabled, reply })
        .map_err(|_| io::Error::other(format!("{reader} input reader disconnected")))?;
    response
        .recv_timeout(INPUT_CONTROL_TIMEOUT)
        .map_err(|error| {
            io::Error::other(format!(
                "{reader} input reader did not update capture state: {error}"
            ))
        })?
        .map_err(|message| io::Error::other(format!("{reader} capture failed: {message}")))?;
    Ok(())
}

fn spawn_keyboard_reader(
    mut keyboard: EvdevKeyboard,
    sender: Sender<HostEvent>,
    mut captured: bool,
) -> Sender<InputReaderCommand> {
    let (control, commands) = mpsc::channel();
    thread::spawn(move || loop {
        match commands.try_recv() {
            Ok(InputReaderCommand::SetCapture { enabled, reply }) => {
                let result = update_keyboard_capture(&mut keyboard, enabled);
                if result.is_ok() {
                    captured = enabled;
                }
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            Err(mpsc::TryRecvError::Disconnected) => return,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let had_events = match keyboard.next_host_key_batch() {
            Ok(batch) => {
                let had_events = !batch.events.is_empty();
                let forwarded = keyboard_events_for_capture(batch.events, captured);
                if !forwarded.is_empty()
                    && sender
                        .send(HostEvent::Keyboard {
                            events: forwarded,
                            received_at: Instant::now(),
                            kernel_timestamp: batch.kernel_timestamp,
                        })
                        .is_err()
                {
                    return;
                }
                had_events
            }
            Err(error) if error.is_would_block() => false,
            Err(error) => {
                let _ = sender.send(HostEvent::ReaderFailed {
                    reader: "keyboard",
                    message: error.to_string(),
                });
                return;
            }
        };
        if had_events {
            continue;
        }
        match commands.recv_timeout(INPUT_READER_POLL) {
            Ok(InputReaderCommand::SetCapture { enabled, reply }) => {
                let result = update_keyboard_capture(&mut keyboard, enabled);
                if result.is_ok() {
                    captured = enabled;
                }
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    });
    control
}

fn keyboard_events_for_capture(events: Vec<HostKeyEvent>, captured: bool) -> Vec<HostKeyEvent> {
    if captured {
        return events;
    }
    events
        .into_iter()
        .filter(|event| event.key == HostKey::F12)
        .collect()
}

fn spawn_mouse_reader(
    mut mouse: EvdevMouse,
    sender: Sender<HostEvent>,
    mut captured: bool,
) -> Sender<InputReaderCommand> {
    let (control, commands) = mpsc::channel();
    thread::spawn(move || loop {
        match commands.try_recv() {
            Ok(InputReaderCommand::SetCapture { enabled, reply }) => {
                let result = update_mouse_capture(&mut mouse, enabled);
                if result.is_ok() {
                    captured = enabled;
                }
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            Err(mpsc::TryRecvError::Disconnected) => return,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let had_events = match mouse.next_event_batch() {
            Ok(batch) => {
                let had_events = !batch.events.is_empty();
                if captured
                    && had_events
                    && sender
                        .send(HostEvent::Mouse {
                            events: batch.events,
                            received_at: Instant::now(),
                            kernel_timestamp: batch.kernel_timestamp,
                        })
                        .is_err()
                {
                    return;
                }
                had_events
            }
            Err(error) if error.is_would_block() => false,
            Err(error) => {
                let _ = sender.send(HostEvent::ReaderFailed {
                    reader: "mouse",
                    message: error.to_string(),
                });
                return;
            }
        };
        if had_events {
            continue;
        }
        match commands.recv_timeout(INPUT_READER_POLL) {
            Ok(InputReaderCommand::SetCapture { enabled, reply }) => {
                let result = update_mouse_capture(&mut mouse, enabled);
                if result.is_ok() {
                    captured = enabled;
                }
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
    });
    control
}

fn update_keyboard_capture(
    keyboard: &mut EvdevKeyboard,
    enabled: bool,
) -> Result<(), wroid_input::KeyboardDeviceError> {
    if enabled {
        drain_keyboard(keyboard)?;
        keyboard.grab()
    } else {
        keyboard.ungrab()
    }
}

fn update_mouse_capture(
    mouse: &mut EvdevMouse,
    enabled: bool,
) -> Result<(), wroid_input::mouse::MouseDeviceError> {
    if enabled {
        drain_mouse(mouse)?;
        mouse.grab()
    } else {
        mouse.ungrab()
    }
}

fn drain_keyboard(keyboard: &mut EvdevKeyboard) -> Result<(), wroid_input::KeyboardDeviceError> {
    for _ in 0..64 {
        match keyboard.next_host_key_events() {
            Ok(_) => {}
            Err(error) if error.is_would_block() => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn drain_mouse(mouse: &mut EvdevMouse) -> Result<(), wroid_input::mouse::MouseDeviceError> {
    for _ in 0..64 {
        match mouse.next_events() {
            Ok(_) => {}
            Err(error) if error.is_would_block() => {
                mouse.clear_pending_report();
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    mouse.clear_pending_report();
    Ok(())
}

struct FocusConnection {
    reader: BufReader<UnixStream>,
    focused: bool,
}

impl FocusConnection {
    fn connect(path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(FOCUS_CONNECT_TIMEOUT))?;
        let mut reader = BufReader::new(stream);
        let mut state = String::new();
        if reader.read_line(&mut state)? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "focus relay closed before reporting initial state",
            ));
        }
        let focused = parse_focus_state(&state)?;
        reader.get_ref().set_read_timeout(None)?;
        Ok(Self { reader, focused })
    }
}

fn parse_focus_state(state: &str) -> io::Result<bool> {
    match state.trim() {
        "focused" => Ok(true),
        "unfocused" => Ok(false),
        value => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid focus relay state: {value}"),
        )),
    }
}

fn spawn_focus_reader(mut reader: BufReader<UnixStream>, sender: Sender<FocusEvent>) {
    thread::spawn(move || loop {
        let mut state = String::new();
        match reader.read_line(&mut state) {
            Ok(0) => {
                let _ = sender.send(FocusEvent::Unavailable(
                    "desktop focus relay disconnected".to_owned(),
                ));
                return;
            }
            Ok(_) => match parse_focus_state(&state) {
                Ok(focused) => {
                    if sender.send(FocusEvent::Changed(focused)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(FocusEvent::Unavailable(error.to_string()));
                    return;
                }
            },
            Err(error) => {
                let _ = sender.send(FocusEvent::Unavailable(format!(
                    "desktop focus relay failed: {error}"
                )));
                return;
            }
        }
    });
}

struct EventLoopOptions<'a> {
    trace_input: bool,
    exit_after: Option<Duration>,
    focus_protected: bool,
    focused: bool,
    focus_receiver: Option<&'a Receiver<FocusEvent>>,
}

fn run_event_loop<I: TouchInjector>(
    receiver: &Receiver<HostEvent>,
    input_readers: &InputReaderControls,
    runtime: &mut UnifiedRuntime<I>,
    options: EventLoopOptions<'_>,
) -> GameSessionResult<EventLoopExit> {
    let trace_input = options.trace_input;
    let exit_after = options.exit_after;
    let mut focus_protected = options.focus_protected;
    let mut waydroid_focused = options.focused;
    let mut input_active = options.focused;
    let mut manually_released = false;
    let loop_started_at = Instant::now();
    let mut control_pressed = false;
    loop {
        if interrupt_requested() {
            return Ok(EventLoopExit::InterruptRequested);
        }
        if let Some(focus_receiver) = options.focus_receiver {
            while let Ok(event) = focus_receiver.try_recv() {
                match event {
                    FocusEvent::Changed(next_focused) if focus_protected => {
                        waydroid_focused = next_focused;
                        control_pressed = false;
                        if next_focused {
                            if manually_released {
                                println!(
                                    "Focus protection: Waydroid focused; press F12 to recapture input."
                                );
                            } else if !input_active {
                                input_readers.set_capture(true)?;
                                runtime.start()?;
                                input_active = true;
                                println!("Focus protection: Waydroid focused; input captured.");
                            }
                        } else if input_active {
                            input_active = false;
                            input_readers.set_capture(false)?;
                            runtime.suspend()?;
                            println!("Focus protection: Waydroid unfocused; input released.");
                        }
                    }
                    FocusEvent::Unavailable(message) if focus_protected => {
                        focus_protected = false;
                        waydroid_focused = false;
                        manually_released = true;
                        control_pressed = false;
                        if input_active {
                            input_active = false;
                            input_readers.set_capture(false)?;
                            runtime.suspend()?;
                        }
                        eprintln!("Focus protection stopped: {message}");
                        eprintln!(
                            "Input was released. Press F12 to recapture with manual protection."
                        );
                    }
                    FocusEvent::Changed(_) | FocusEvent::Unavailable(_) => {}
                }
            }
        }
        match receiver.recv_timeout(IDLE_POLL) {
            Ok(HostEvent::Keyboard {
                events,
                received_at,
                kernel_timestamp,
            }) => {
                let mut submitted = false;
                for event in events {
                    if trace_input {
                        println!(
                            "[trace] host keyboard key={} transition={:?}",
                            event.key.profile_name(),
                            event.transition
                        );
                    }
                    if event.key == HostKey::F12 {
                        if event.transition == KeyTransition::Pressed {
                            control_pressed = false;
                            if input_active {
                                manually_released = true;
                                input_active = false;
                                input_readers.set_capture(false)?;
                                runtime.suspend()?;
                                println!(
                                    "Manual capture: input released. Alt+Tab is available; focus Waydroid and press F12 to recapture."
                                );
                            } else if manually_released && (waydroid_focused || !focus_protected) {
                                input_readers.set_capture(true)?;
                                runtime.start()?;
                                input_active = true;
                                manually_released = false;
                                println!("Manual capture: Waydroid input recaptured.");
                            } else if manually_released {
                                println!("Manual capture: focus Waydroid before pressing F12.");
                            }
                        }
                        continue;
                    }
                    if input_active {
                        if event.key.profile_name() == "ctrl" {
                            control_pressed = event.transition != KeyTransition::Released;
                        }
                        if control_pressed
                            && event.key.profile_name() == "esc"
                            && event.transition == KeyTransition::Pressed
                        {
                            return Ok(EventLoopExit::ExitHotkeyRequested);
                        }
                        if control_pressed
                            && event.key.profile_name() == "c"
                            && event.transition == KeyTransition::Pressed
                        {
                            return Ok(EventLoopExit::InterruptRequested);
                        }
                        submitted |= runtime.handle_keyboard(event)?;
                    }
                }
                if submitted {
                    runtime.record_pipeline_latency(received_at.elapsed(), kernel_timestamp);
                }
            }
            Ok(HostEvent::Mouse {
                events,
                received_at,
                kernel_timestamp,
            }) => {
                if input_active {
                    let mut submitted = false;
                    for event in events {
                        if trace_input {
                            println!("[trace] host mouse {event:?}");
                        }
                        submitted |= runtime.handle_mouse(event)?;
                    }
                    if submitted {
                        runtime.record_pipeline_latency(received_at.elapsed(), kernel_timestamp);
                    }
                }
            }
            Ok(HostEvent::ReaderFailed { reader, message }) => {
                return Err(
                    io::Error::other(format!("{reader} input reader failed: {message}")).into(),
                );
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other(
                    "all input readers disconnected unexpectedly while the game session was running",
                )
                .into());
            }
        }
        if input_active {
            runtime.tick()?;
        }
        if exit_after.is_some_and(|limit| loop_started_at.elapsed() >= limit) {
            return Ok(EventLoopExit::TimeLimitReached);
        }
    }
}

struct UnifiedRuntime<I: TouchInjector> {
    plan: RuntimeControlPlan,
    engine: TouchEngine<I>,
    directions: BTreeMap<String, DirectionalInput>,
    point_contacts: BTreeMap<String, ContactId>,
    aim_controllers: BTreeMap<String, MouseAimController>,
    last_joystick_frame: BTreeMap<String, Duration>,
    started_at: Instant,
    pipeline_latencies: Vec<Duration>,
    kernel_latencies: Vec<Duration>,
    rejected_kernel_timestamps: u64,
    trace_input: bool,
}

impl<I: TouchInjector> UnifiedRuntime<I> {
    fn new(plan: RuntimeControlPlan, injector: I, trace_input: bool) -> GameSessionResult<Self> {
        let mut next_contact = plan
            .controls
            .iter()
            .filter_map(|control| match &control.action {
                RuntimeControlAction::VirtualJoystick { joystick, .. } => {
                    Some(joystick.contact_id().get())
                }
                RuntimeControlAction::MouseAim { aim, settings } => Some(
                    aim.contact_id()
                        .get()
                        .max(settings.alternate_contact_id.get()),
                ),
                RuntimeControlAction::Tap { .. } | RuntimeControlAction::Hold { .. } => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut point_contacts = BTreeMap::new();
        let mut aim_controllers = BTreeMap::new();
        for control in &plan.controls {
            if matches!(
                control.action,
                RuntimeControlAction::Tap { .. } | RuntimeControlAction::Hold { .. }
            ) {
                point_contacts.insert(control.name.clone(), ContactId::new(next_contact));
                next_contact = next_contact.saturating_add(1);
            }
            if let RuntimeControlAction::MouseAim { aim, settings } = &control.action {
                aim_controllers.insert(
                    control.name.clone(),
                    MouseAimController::new(aim.clone(), settings.clone())?,
                );
            }
        }
        Ok(Self {
            plan,
            engine: TouchEngine::new(injector),
            directions: BTreeMap::new(),
            point_contacts,
            aim_controllers,
            last_joystick_frame: BTreeMap::new(),
            started_at: Instant::now(),
            pipeline_latencies: Vec::new(),
            kernel_latencies: Vec::new(),
            rejected_kernel_timestamps: 0,
            trace_input,
        })
    }

    fn start(&mut self) -> GameSessionResult<()> {
        let now = self.now();
        for controller in self.aim_controllers.values_mut() {
            if controller.settings().toggle_key.is_none() {
                controller.activate(&mut self.engine, now)?;
            }
        }
        Ok(())
    }

    fn handle_keyboard(&mut self, event: HostKeyEvent) -> GameSessionResult<bool> {
        let key = event.key.profile_name();
        let pressed = match event.transition {
            KeyTransition::Pressed => true,
            KeyTransition::Released => false,
            KeyTransition::Repeated => return Ok(false),
        };
        let now = self.now();
        let mut submitted = false;

        if pressed {
            for (name, controller) in &mut self.aim_controllers {
                if controller
                    .settings()
                    .toggle_key
                    .as_deref()
                    .is_some_and(|toggle| key_eq(toggle, key))
                {
                    let update = controller.toggle(&mut self.engine, now)?;
                    submitted |= update != MouseAimUpdate::Ignored;
                    if self.trace_input {
                        println!("[trace] runtime aim binding={name} toggle={update:?}");
                    }
                }
            }
        }

        let controls = &self.plan.controls;
        let directions = &mut self.directions;
        let point_contacts = &self.point_contacts;
        let engine = &mut self.engine;
        let last_joystick_frame = &mut self.last_joystick_frame;
        let trace_input = self.trace_input;
        for control in controls {
            match (&control.input, &control.action) {
                (InputV2::Key { key: binding_key }, RuntimeControlAction::Tap { point })
                    if pressed && key_eq(binding_key, key) =>
                {
                    tap_binding(engine, point_contacts, &control.name, *point, trace_input)?;
                    submitted = true;
                }
                (InputV2::Key { key: binding_key }, RuntimeControlAction::Hold { point })
                    if key_eq(binding_key, key) =>
                {
                    submitted |= set_hold_binding(
                        engine,
                        point_contacts,
                        &control.name,
                        *point,
                        pressed,
                        trace_input,
                    )?;
                }
                (
                    InputV2::KeyCluster {
                        up,
                        left,
                        down,
                        right,
                    },
                    RuntimeControlAction::VirtualJoystick { joystick, mode, .. },
                ) => {
                    let relevant = [up, left, down, right]
                        .into_iter()
                        .any(|binding| key_eq(binding, key));
                    if !relevant {
                        continue;
                    }
                    if !directions.contains_key(control.name.as_str()) {
                        directions.insert(control.name.clone(), DirectionalInput::default());
                    }
                    let direction = directions
                        .get_mut(control.name.as_str())
                        .expect("joystick direction was initialized");
                    let mut changed = false;
                    match mode {
                        JoystickMode::Hold => {
                            changed |= update_direction(&mut direction.up, up, key, pressed);
                            changed |= update_direction(&mut direction.left, left, key, pressed);
                            changed |= update_direction(&mut direction.down, down, key, pressed);
                            changed |= update_direction(&mut direction.right, right, key, pressed);
                        }
                        JoystickMode::Toggle if pressed => {
                            changed |= toggle_direction(&mut direction.up, up, key);
                            changed |= toggle_direction(&mut direction.left, left, key);
                            changed |= toggle_direction(&mut direction.down, down, key);
                            changed |= toggle_direction(&mut direction.right, right, key);
                        }
                        JoystickMode::Toggle => {}
                    }
                    if changed {
                        let frame_submitted = joystick.apply(engine, *direction)?;
                        submitted |= frame_submitted;
                        if frame_submitted {
                            record_joystick_frame(last_joystick_frame, &control.name, now);
                        }
                        if trace_input {
                            println!(
                                "[trace] runtime joystick binding={} contact={} direction={direction:?} submitted={} position={:?}",
                                control.name,
                                joystick.contact_id().get(),
                                frame_submitted,
                                engine.state().position(joystick.contact_id())
                            );
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(submitted)
    }

    fn handle_mouse(&mut self, event: MouseEvent) -> GameSessionResult<bool> {
        let now = self.now();
        let mut submitted = false;
        match event {
            MouseEvent::Motion(RelativeMouseMotion { dx, dy }) if dx != 0 || dy != 0 => {
                for (name, controller) in &mut self.aim_controllers {
                    let update =
                        controller.move_by(&mut self.engine, MouseAimDelta::new(dx, dy), now)?;
                    submitted |= update != MouseAimUpdate::Ignored;
                    if self.trace_input {
                        println!(
                            "[trace] runtime aim binding={name} delta=({dx},{dy}) update={update:?} recenter_count={}",
                            controller.recenter_count(),
                        );
                    }
                }
            }
            MouseEvent::Button(button_event) => {
                let button = button_event.button.profile_name();
                let pressed = match button_event.transition {
                    MouseButtonTransition::Pressed => true,
                    MouseButtonTransition::Released => false,
                    MouseButtonTransition::Repeated => return Ok(false),
                };
                if button == "right" {
                    for controller in self.aim_controllers.values_mut() {
                        controller.set_ads_active(pressed);
                    }
                }
                let controls = &self.plan.controls;
                let point_contacts = &self.point_contacts;
                let engine = &mut self.engine;
                let trace_input = self.trace_input;
                for control in controls {
                    let InputV2::MouseButton {
                        button: binding_button,
                    } = &control.input
                    else {
                        continue;
                    };
                    if key_eq(binding_button, button) {
                        match &control.action {
                            RuntimeControlAction::Tap { point } if pressed => {
                                tap_binding(
                                    engine,
                                    point_contacts,
                                    &control.name,
                                    *point,
                                    trace_input,
                                )?;
                                submitted = true;
                            }
                            RuntimeControlAction::Hold { point } => {
                                submitted |= set_hold_binding(
                                    engine,
                                    point_contacts,
                                    &control.name,
                                    *point,
                                    pressed,
                                    trace_input,
                                )?;
                            }
                            _ => {}
                        }
                    }
                }
            }
            MouseEvent::Motion(_) | MouseEvent::Wheel(_) => {}
        }
        Ok(submitted)
    }

    fn tick(&mut self) -> GameSessionResult<()> {
        let now = self.now();
        for controller in self.aim_controllers.values_mut() {
            controller.tick(&mut self.engine, now)?;
        }
        let controls = &self.plan.controls;
        let engine = &mut self.engine;
        let last_joystick_frame = &mut self.last_joystick_frame;
        for control in controls {
            let RuntimeControlAction::VirtualJoystick {
                joystick,
                reaffirm_interval: Some(interval),
                ..
            } = &control.action
            else {
                continue;
            };
            let Some(position) = engine.state().position(joystick.contact_id()) else {
                continue;
            };
            if last_joystick_frame
                .get(&control.name)
                .is_some_and(|last| now.saturating_sub(*last) < *interval)
            {
                continue;
            }
            engine.move_contact(joystick.contact_id(), position)?;
            record_joystick_frame(last_joystick_frame, &control.name, now);
        }
        Ok(())
    }

    fn suspend(&mut self) -> GameSessionResult<()> {
        let active_before = self.engine.state().active_contact_count();
        self.directions.clear();
        self.last_joystick_frame.clear();
        for controller in self.aim_controllers.values_mut() {
            controller.cancel(&mut self.engine)?;
        }
        let cancelled = self.engine.cancel_all()?;
        if self.trace_input {
            println!(
                "[trace] runtime suspend active_before={} cancel_frame_submitted={cancelled}",
                active_before
            );
        }
        Ok(())
    }

    fn stop(&mut self) -> GameSessionResult<()> {
        self.suspend()
    }

    fn now(&self) -> Duration {
        self.started_at.elapsed()
    }

    fn record_pipeline_latency(
        &mut self,
        reader_latency: Duration,
        kernel_timestamp: Option<SystemTime>,
    ) {
        const MAX_SAMPLES: usize = 100_000;
        if self.pipeline_latencies.len() < MAX_SAMPLES {
            self.pipeline_latencies.push(reader_latency);
        }
        if let Some(timestamp) = kernel_timestamp {
            if let Some(latency) = kernel_event_age(timestamp) {
                if self.kernel_latencies.len() < MAX_SAMPLES {
                    self.kernel_latencies.push(latency);
                }
            } else {
                self.rejected_kernel_timestamps = self.rejected_kernel_timestamps.saturating_add(1);
            }
        }
    }
}

fn tap_binding<I: TouchInjector>(
    engine: &mut TouchEngine<I>,
    point_contacts: &BTreeMap<String, ContactId>,
    binding: &str,
    point: Point,
    trace_input: bool,
) -> GameSessionResult<()> {
    let contact_id = *point_contacts
        .get(binding)
        .ok_or_else(|| io::Error::other(format!("missing point contact for {binding}")))?;
    engine.submit(TouchFrame::single(TouchEvent::new(
        contact_id,
        TouchPhase::Down,
        point,
    )))?;
    engine.submit(TouchFrame::single(TouchEvent::new(
        contact_id,
        TouchPhase::Up,
        point,
    )))?;
    if trace_input {
        println!(
            "[trace] runtime tap binding={binding} contact={} point=({},{}) submitted=down+up",
            contact_id.get(),
            point.x,
            point.y
        );
    }
    Ok(())
}

fn set_hold_binding<I: TouchInjector>(
    engine: &mut TouchEngine<I>,
    point_contacts: &BTreeMap<String, ContactId>,
    binding: &str,
    point: Point,
    pressed: bool,
    trace_input: bool,
) -> GameSessionResult<bool> {
    let contact_id = *point_contacts
        .get(binding)
        .ok_or_else(|| io::Error::other(format!("missing hold contact for {binding}")))?;
    let active = engine.state().position(contact_id).is_some();
    if active == pressed {
        return Ok(false);
    }
    let phase = if pressed {
        TouchPhase::Down
    } else {
        TouchPhase::Up
    };
    engine.submit(TouchFrame::single(TouchEvent::new(
        contact_id, phase, point,
    )))?;
    if trace_input {
        println!(
            "[trace] runtime hold binding={binding} contact={} point=({},{}) phase={phase:?}",
            contact_id.get(),
            point.x,
            point.y
        );
    }
    Ok(true)
}

fn record_joystick_frame(
    last_frames: &mut BTreeMap<String, Duration>,
    binding: &str,
    now: Duration,
) {
    if let Some(last) = last_frames.get_mut(binding) {
        *last = now;
    } else {
        last_frames.insert(binding.to_owned(), now);
    }
}

impl<I: TouchInjector> Drop for UnifiedRuntime<I> {
    fn drop(&mut self) {
        let _ = self.engine.cancel_all();
    }
}

impl UnifiedRuntime<SessionMetricsInjector<UinputTouchInjector>> {
    fn report(&self) -> GameSessionReport {
        let injector = self.engine.injector();
        let recenter_count = self
            .aim_controllers
            .values()
            .map(MouseAimController::recenter_count)
            .sum::<u64>();
        let reader_to_inject = latency_metrics(&self.pipeline_latencies);
        let kernel_to_inject = latency_metrics(&self.kernel_latencies);
        GameSessionReport {
            frames_submitted: injector.frames_submitted,
            peak_simultaneous_contacts: injector.peak_contacts as u64,
            mouse_aim_recenters: recenter_count,
            reader_to_inject,
            kernel_to_inject: (kernel_to_inject.samples > 0).then_some(kernel_to_inject),
            rejected_kernel_timestamps: self.rejected_kernel_timestamps,
        }
    }

    fn print_report(report: &GameSessionReport) {
        println!("Session report:");
        println!("  frames submitted: {}", report.frames_submitted);
        println!(
            "  peak simultaneous contacts: {}",
            report.peak_simultaneous_contacts
        );
        println!("  mouse aim recenters: {}", report.mouse_aim_recenters);
        let latency = report.reader_to_inject;
        println!(
            "  reader-to-inject p50/p95/p99/max: {}/{}/{}/{} us ({} batch samples)",
            latency.p50_micros,
            latency.p95_micros,
            latency.p99_micros,
            latency.max_micros,
            latency.samples
        );
        if let Some(kernel_latency) = report.kernel_to_inject {
            println!(
                "  kernel-to-inject p50/p95/p99/max: {}/{}/{}/{} us ({} batch samples)",
                kernel_latency.p50_micros,
                kernel_latency.p95_micros,
                kernel_latency.p99_micros,
                kernel_latency.max_micros,
                kernel_latency.samples
            );
        } else {
            println!("  kernel-to-inject latency: unavailable (no valid evdev timestamps)");
        }
        if report.rejected_kernel_timestamps > 0 {
            println!(
                "  rejected kernel timestamps: {} (clock mismatch or older than 60 s)",
                report.rejected_kernel_timestamps
            );
        }
    }
}

struct SessionMetricsInjector<I> {
    inner: I,
    active_contacts: BTreeSet<ContactId>,
    frames_submitted: u64,
    peak_contacts: usize,
}

impl<I> SessionMetricsInjector<I> {
    fn new(inner: I) -> Self {
        Self {
            inner,
            active_contacts: BTreeSet::new(),
            frames_submitted: 0,
            peak_contacts: 0,
        }
    }
}

impl<I: TouchInjector> TouchInjector for SessionMetricsInjector<I> {
    fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
        self.inner.inject(frame)?;
        for event in frame.events() {
            match event.phase {
                TouchPhase::Down => {
                    self.active_contacts.insert(event.contact_id);
                }
                TouchPhase::Up | TouchPhase::Cancel => {
                    self.active_contacts.remove(&event.contact_id);
                }
                TouchPhase::Move => {}
            }
        }
        self.frames_submitted = self.frames_submitted.saturating_add(1);
        self.peak_contacts = self.peak_contacts.max(self.active_contacts.len());
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LatencySummary {
    samples: usize,
    p50: u128,
    p95: u128,
    p99: u128,
    max: u128,
}

fn kernel_event_age(timestamp: SystemTime) -> Option<Duration> {
    const MAX_REASONABLE_AGE: Duration = Duration::from_secs(60);
    SystemTime::now()
        .duration_since(timestamp)
        .ok()
        .filter(|latency| *latency <= MAX_REASONABLE_AGE)
}

fn latency_summary_micros(samples: &[Duration]) -> LatencySummary {
    if samples.is_empty() {
        return LatencySummary::default();
    }
    let mut micros = samples.iter().map(Duration::as_micros).collect::<Vec<_>>();
    micros.sort_unstable();
    LatencySummary {
        samples: micros.len(),
        p50: percentile_from_sorted(&micros, 50),
        p95: percentile_from_sorted(&micros, 95),
        p99: percentile_from_sorted(&micros, 99),
        max: *micros.last().expect("non-empty samples have a maximum"),
    }
}

fn latency_metrics(samples: &[Duration]) -> LatencyMetrics {
    let summary = latency_summary_micros(samples);
    LatencyMetrics {
        samples: summary.samples.try_into().unwrap_or(u64::MAX),
        p50_micros: summary.p50.try_into().unwrap_or(u64::MAX),
        p95_micros: summary.p95.try_into().unwrap_or(u64::MAX),
        p99_micros: summary.p99.try_into().unwrap_or(u64::MAX),
        max_micros: summary.max.try_into().unwrap_or(u64::MAX),
    }
}

fn percentile_from_sorted(micros: &[u128], percentile: usize) -> u128 {
    let rank = micros.len().saturating_mul(percentile).saturating_add(99) / 100;
    let index = rank.saturating_sub(1).min(micros.len() - 1);
    micros[index]
}

fn update_direction(state: &mut bool, binding_key: &str, event_key: &str, pressed: bool) -> bool {
    if !key_eq(binding_key, event_key) || *state == pressed {
        return false;
    }
    *state = pressed;
    true
}

fn toggle_direction(state: &mut bool, binding_key: &str, event_key: &str) -> bool {
    if !key_eq(binding_key, event_key) {
        return false;
    }
    *state = !*state;
    true
}

fn key_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[derive(Debug)]
pub struct GameSessionOptions {
    pub profile_path: Option<PathBuf>,
    pub keyboard_path: Option<PathBuf>,
    pub mouse_path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub grab: bool,
    pub show_ui: bool,
    pub launch_package: bool,
    pub trace_input: bool,
    pub exit_after: Option<Duration>,
    pub focus_socket: Option<PathBuf>,
    pub bridge_helper: Option<BridgeHelperCommand>,
    pub cleanup: bool,
}

impl GameSessionOptions {
    pub fn new(
        profile_path: PathBuf,
        keyboard_path: PathBuf,
        mouse_path: Option<PathBuf>,
        width: u32,
        height: u32,
    ) -> GameSessionResult<Self> {
        if width == 0 || height == 0 {
            return Err(invalid_input("width and height must be greater than zero"));
        }
        Ok(Self {
            profile_path: Some(profile_path),
            keyboard_path: Some(keyboard_path),
            mouse_path,
            width,
            height,
            grab: true,
            show_ui: true,
            launch_package: true,
            trace_input: false,
            exit_after: None,
            focus_socket: None,
            bridge_helper: None,
            cleanup: false,
        })
    }

    fn parse(args: impl IntoIterator<Item = String>) -> GameSessionResult<Option<Self>> {
        let mut positional = Vec::new();
        let mut options = Self {
            profile_path: None,
            keyboard_path: None,
            mouse_path: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            grab: true,
            show_ui: true,
            launch_package: true,
            trace_input: false,
            exit_after: None,
            focus_socket: None,
            bridge_helper: None,
            cleanup: false,
        };
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--cleanup" => options.cleanup = true,
                "--no-grab" => options.grab = false,
                "--no-ui" => options.show_ui = false,
                "--no-launch" => options.launch_package = false,
                "--trace-input" => options.trace_input = true,
                "--exit-after-ms" => {
                    let milliseconds: u64 = parse_next(&mut args, "--exit-after-ms")?;
                    if milliseconds == 0 {
                        return Err(invalid_input("--exit-after-ms must be greater than zero"));
                    }
                    options.exit_after = Some(Duration::from_millis(milliseconds));
                }
                "--focus-socket" => {
                    options.focus_socket = Some(PathBuf::from(parse_next::<String>(
                        &mut args,
                        "--focus-socket",
                    )?));
                }
                "--width" => options.width = parse_next(&mut args, "--width")?,
                "--height" => options.height = parse_next(&mut args, "--height")?,
                value if value.starts_with("--") => {
                    return Err(invalid_input(format!("unknown option: {value}")))
                }
                value => positional.push(PathBuf::from(value)),
            }
        }
        if options.cleanup {
            return Ok(Some(options));
        }
        if options.width == 0 || options.height == 0 {
            return Err(invalid_input("width and height must be greater than zero"));
        }
        if !(2..=3).contains(&positional.len()) {
            return Err(invalid_input(
                "expected <profile-v2.json> <keyboard-event-node> [mouse-event-node]",
            ));
        }
        options.profile_path = Some(positional.remove(0));
        options.keyboard_path = Some(positional.remove(0));
        options.mouse_path = (!positional.is_empty()).then(|| positional.remove(0));
        Ok(Some(options))
    }
}

fn parse_next<T>(args: &mut impl Iterator<Item = String>, label: &str) -> GameSessionResult<T>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    let value = args
        .next()
        .ok_or_else(|| invalid_input(format!("missing {label}")))?;
    value
        .parse::<T>()
        .map_err(|source| invalid_input(format!("invalid {label} '{value}': {source}")))
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error + Send + Sync> {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

fn install_interrupt_handler() -> io::Result<()> {
    INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
    for signal_number in [SIGHUP, SIGINT, SIGTERM] {
        // SAFETY: the handler only performs a lock-free atomic store.
        let previous = unsafe { signal(signal_number, request_interrupt) };
        if previous == SIGNAL_ERROR {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

extern "C" fn request_interrupt(_signal: i32) {
    INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
}

fn interrupt_requested() -> bool {
    INTERRUPT_REQUESTED.load(Ordering::SeqCst)
}

fn print_usage() {
    println!(
        "Usage: wroid-waydroid-game-session <profile-v2.json> <keyboard-event-node> [mouse-event-node] [--width W] [--height H] [--no-grab] [--no-ui] [--no-launch] [--trace-input] [--exit-after-ms N]"
    );
    println!(
        "Example: sudo ./target/release/wroid-waydroid-game-session profiles/examples/shooter-v2.json /dev/input/event7 /dev/input/event9 --width 1920 --height 1080 --trace-input"
    );
    println!("Recovery: sudo ./target/release/wroid-waydroid-game-session --cleanup");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    use wroid_core::profile_v2::{ActionV2, BindingV2, JoystickMode, NormalizedPoint, ProfileV2};
    use wroid_runtime::{TouchInjectionError, TouchInjector};

    #[derive(Default)]
    struct RecordingInjector {
        frames: Vec<TouchFrame>,
    }

    impl TouchInjector for RecordingInjector {
        fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
            self.frames.push(frame.clone());
            Ok(())
        }
    }

    #[test]
    fn parses_trace_input_flag() {
        let options = GameSessionOptions::parse([
            "profile.json".to_owned(),
            "/dev/input/event7".to_owned(),
            "/dev/input/event3".to_owned(),
            "--trace-input".to_owned(),
            "--no-grab".to_owned(),
        ])
        .unwrap()
        .unwrap();

        assert!(options.trace_input);
        assert!(!options.grab);
        assert_eq!(options.profile_path, Some(PathBuf::from("profile.json")));
    }

    #[test]
    fn parses_internal_focus_socket() {
        let options = GameSessionOptions::parse([
            "profile.json".to_owned(),
            "/dev/input/event7".to_owned(),
            "--focus-socket".to_owned(),
            "/run/user/1000/wroid/focus-test.sock".to_owned(),
        ])
        .unwrap()
        .unwrap();

        assert_eq!(
            options.focus_socket,
            Some(PathBuf::from("/run/user/1000/wroid/focus-test.sock"))
        );
        assert!(parse_focus_state("focused\n").unwrap());
        assert!(!parse_focus_state("unfocused\n").unwrap());
        assert!(parse_focus_state("unknown\n").is_err());
    }

    #[test]
    fn released_keyboard_forwards_only_the_capture_hotkey() {
        let events = vec![
            HostKeyEvent::new(wroid_input::HostKey::W, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::F12, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::F12, KeyTransition::Released),
        ];
        assert_eq!(
            keyboard_events_for_capture(events.clone(), false),
            events[1..]
        );
        assert_eq!(keyboard_events_for_capture(events.clone(), true), events);
    }

    #[test]
    fn cleanup_does_not_require_positional_paths() {
        let options = GameSessionOptions::parse(["--cleanup".to_owned()])
            .unwrap()
            .unwrap();

        assert!(options.cleanup);
        assert!(options.profile_path.is_none());
        assert!(options.keyboard_path.is_none());
        assert!(options.mouse_path.is_none());
    }

    #[test]
    fn reports_game_and_all_cleanup_failures_together() {
        let result = combine_session_results::<()>(
            Err(io::Error::other("runtime injection failed").into()),
            Err(io::Error::other("container did not stop")),
            Err(io::Error::other("managed include could not be removed")),
        );

        let error = result.unwrap_err().to_string();
        assert!(error.contains("game session failed: runtime injection failed"));
        assert!(error.contains("Waydroid shutdown failed: container did not stop"));
        assert!(error.contains("input bridge cleanup failed: managed include could not be removed"));
    }

    #[test]
    fn keyboard_only_profile_does_not_require_mouse_path() {
        let options =
            GameSessionOptions::parse(["profile.json".to_owned(), "/dev/input/event7".to_owned()])
                .unwrap()
                .unwrap();

        assert_eq!(
            options.keyboard_path,
            Some(PathBuf::from("/dev/input/event7"))
        );
        assert!(options.mouse_path.is_none());
    }

    #[test]
    fn two_joysticks_and_tap_coexist_as_three_contacts() {
        let profile = ProfileV2 {
            schema_version: 2,
            name: "Brawl test".to_owned(),
            package_name: "com.supercell.brawlstars".to_owned(),
            orientation: Default::default(),
            bindings: vec![
                joystick_binding("movement", "w", "a", "s", "d", 0.2),
                joystick_binding("attack", "up", "left", "down", "right", 0.8),
                BindingV2 {
                    name: "super".to_owned(),
                    input: InputV2::Key {
                        key: "space".to_owned(),
                    },
                    action: ActionV2::Tap {
                        point: NormalizedPoint { x: 0.7, y: 0.8 },
                    },
                },
            ],
        };
        let plan = RuntimeControlPlan::from_profile_v2(
            &profile,
            Resolution {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap();
        let mut runtime = UnifiedRuntime::new(plan, RecordingInjector::default(), false).unwrap();
        runtime.start().unwrap();

        runtime
            .handle_keyboard(HostKeyEvent::new(
                wroid_input::HostKey::W,
                KeyTransition::Pressed,
            ))
            .unwrap();
        runtime
            .handle_keyboard(HostKeyEvent::new(
                wroid_input::HostKey::ArrowRight,
                KeyTransition::Pressed,
            ))
            .unwrap();
        runtime
            .handle_keyboard(HostKeyEvent::new(
                wroid_input::HostKey::Space,
                KeyTransition::Pressed,
            ))
            .unwrap();

        assert_eq!(peak_contacts(&runtime.engine.injector().frames), 3);
        assert_eq!(runtime.engine.state().active_contact_count(), 2);
        runtime.stop().unwrap();
        assert_eq!(runtime.engine.state().active_contact_count(), 0);
        assert_eq!(
            runtime
                .engine
                .injector()
                .frames
                .last()
                .unwrap()
                .events()
                .len(),
            2
        );
    }

    #[test]
    fn mouse_hold_keeps_contact_down_until_release() {
        let profile = ProfileV2 {
            schema_version: 2,
            name: "Automatic fire test".to_owned(),
            package_name: "com.example.shooter".to_owned(),
            orientation: Default::default(),
            bindings: vec![BindingV2 {
                name: "fire".to_owned(),
                input: InputV2::MouseButton {
                    button: "left".to_owned(),
                },
                action: ActionV2::Hold {
                    point: NormalizedPoint { x: 0.9, y: 0.5 },
                },
            }],
        };
        let plan = RuntimeControlPlan::from_profile_v2(
            &profile,
            Resolution {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap();
        let mut runtime = UnifiedRuntime::new(plan, RecordingInjector::default(), false).unwrap();
        runtime.start().unwrap();
        let press = MouseEvent::Button(wroid_input::mouse::MouseButtonEvent::new(
            wroid_input::mouse::MouseButton::Left,
            MouseButtonTransition::Pressed,
        ));
        let release = MouseEvent::Button(wroid_input::mouse::MouseButtonEvent::new(
            wroid_input::mouse::MouseButton::Left,
            MouseButtonTransition::Released,
        ));

        assert!(runtime.handle_mouse(press).unwrap());
        assert!(!runtime.handle_mouse(press).unwrap());
        assert_eq!(runtime.engine.state().active_contact_count(), 1);
        assert_eq!(runtime.engine.injector().frames.len(), 1);
        assert_eq!(
            runtime.engine.injector().frames[0].events()[0].phase,
            TouchPhase::Down
        );

        assert!(runtime.handle_mouse(release).unwrap());
        assert_eq!(runtime.engine.state().active_contact_count(), 0);
        assert_eq!(runtime.engine.injector().frames.len(), 2);
        assert_eq!(
            runtime.engine.injector().frames[1].events()[0].phase,
            TouchPhase::Up
        );
    }

    #[test]
    fn focus_loss_suspends_runtime_and_releases_capture() {
        let profile = ProfileV2 {
            schema_version: 2,
            name: "Focus test".to_owned(),
            package_name: "com.example.focus".to_owned(),
            orientation: Default::default(),
            bindings: vec![joystick_binding("movement", "w", "a", "s", "d", 0.2)],
        };
        let plan = RuntimeControlPlan::from_profile_v2(
            &profile,
            Resolution {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap();
        let mut runtime = UnifiedRuntime::new(plan, RecordingInjector::default(), false).unwrap();
        runtime.start().unwrap();
        runtime
            .handle_keyboard(HostKeyEvent::new(
                wroid_input::HostKey::W,
                KeyTransition::Pressed,
            ))
            .unwrap();
        assert_eq!(runtime.engine.state().active_contact_count(), 1);

        let (host_sender, host_receiver) = mpsc::channel();
        let (focus_sender, focus_receiver) = mpsc::channel();
        focus_sender.send(FocusEvent::Changed(false)).unwrap();
        let (reader_control, reader_commands) = mpsc::channel();
        let reader_thread = thread::spawn(move || {
            if let Ok(InputReaderCommand::SetCapture { enabled, reply }) =
                reader_commands.recv_timeout(Duration::from_secs(1))
            {
                assert!(!enabled);
                let _ = reply.send(Ok(()));
            }
        });
        let controls = InputReaderControls {
            keyboard: reader_control,
            mouse: None,
        };

        let exit = run_event_loop(
            &host_receiver,
            &controls,
            &mut runtime,
            EventLoopOptions {
                trace_input: false,
                exit_after: Some(Duration::from_millis(1)),
                focus_protected: true,
                focused: true,
                focus_receiver: Some(&focus_receiver),
            },
        )
        .unwrap();
        drop(host_sender);
        reader_thread.join().unwrap();

        assert_eq!(exit, EventLoopExit::TimeLimitReached);
        assert_eq!(runtime.engine.state().active_contact_count(), 0);
        assert!(runtime.directions.is_empty());
    }

    #[test]
    fn f12_releases_and_recaptures_before_gameplay_resumes() {
        let profile = ProfileV2 {
            schema_version: 2,
            name: "Capture toggle test".to_owned(),
            package_name: "com.example.capture".to_owned(),
            orientation: Default::default(),
            bindings: vec![joystick_binding("movement", "w", "a", "s", "d", 0.2)],
        };
        let plan = RuntimeControlPlan::from_profile_v2(
            &profile,
            Resolution {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap();
        let mut runtime = UnifiedRuntime::new(plan, RecordingInjector::default(), false).unwrap();
        runtime.start().unwrap();

        let (host_sender, host_receiver) = mpsc::channel();
        for event in [
            HostKeyEvent::new(wroid_input::HostKey::F12, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::F12, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::W, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::LeftCtrl, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::Esc, KeyTransition::Pressed),
        ] {
            host_sender
                .send(HostEvent::Keyboard {
                    events: vec![event],
                    received_at: Instant::now(),
                    kernel_timestamp: None,
                })
                .unwrap();
        }
        let (reader_control, reader_commands) = mpsc::channel();
        let (capture_state_sender, capture_states) = mpsc::channel();
        let reader_thread = thread::spawn(move || {
            for _ in 0..2 {
                let InputReaderCommand::SetCapture { enabled, reply } = reader_commands
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap();
                capture_state_sender.send(enabled).unwrap();
                let _ = reply.send(Ok(()));
            }
        });
        let controls = InputReaderControls {
            keyboard: reader_control,
            mouse: None,
        };

        let exit = run_event_loop(
            &host_receiver,
            &controls,
            &mut runtime,
            EventLoopOptions {
                trace_input: false,
                exit_after: None,
                focus_protected: true,
                focused: true,
                focus_receiver: None,
            },
        )
        .unwrap();
        reader_thread.join().unwrap();

        assert_eq!(exit, EventLoopExit::ExitHotkeyRequested);
        assert_eq!(capture_states.try_iter().collect::<Vec<_>>(), [false, true]);
        assert_eq!(runtime.engine.state().active_contact_count(), 1);
        assert_eq!(runtime.pipeline_latencies.len(), 1);
        assert!(runtime.kernel_latencies.is_empty());
        runtime.stop().unwrap();
    }

    #[test]
    fn plain_escape_reaches_profile_before_ctrl_escape_stops_session() {
        let profile = ProfileV2 {
            schema_version: 2,
            name: "Escape binding test".to_owned(),
            package_name: "com.example.escape".to_owned(),
            orientation: Default::default(),
            bindings: vec![BindingV2 {
                name: "menu".to_owned(),
                input: InputV2::Key {
                    key: "esc".to_owned(),
                },
                action: ActionV2::Tap {
                    point: NormalizedPoint { x: 0.5, y: 0.5 },
                },
            }],
        };
        let plan = RuntimeControlPlan::from_profile_v2(
            &profile,
            Resolution {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap();
        let mut runtime = UnifiedRuntime::new(plan, RecordingInjector::default(), false).unwrap();
        runtime.start().unwrap();

        let (host_sender, host_receiver) = mpsc::channel();
        for event in [
            HostKeyEvent::new(wroid_input::HostKey::Esc, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::LeftCtrl, KeyTransition::Pressed),
            HostKeyEvent::new(wroid_input::HostKey::Esc, KeyTransition::Pressed),
        ] {
            host_sender
                .send(HostEvent::Keyboard {
                    events: vec![event],
                    received_at: Instant::now(),
                    kernel_timestamp: None,
                })
                .unwrap();
        }
        let (reader_control, _reader_commands) = mpsc::channel();
        let controls = InputReaderControls {
            keyboard: reader_control,
            mouse: None,
        };

        let exit = run_event_loop(
            &host_receiver,
            &controls,
            &mut runtime,
            EventLoopOptions {
                trace_input: false,
                exit_after: None,
                focus_protected: true,
                focused: true,
                focus_receiver: None,
            },
        )
        .unwrap();

        assert_eq!(exit, EventLoopExit::ExitHotkeyRequested);
        assert_eq!(runtime.engine.injector().frames.len(), 2);
        assert_eq!(runtime.pipeline_latencies.len(), 1);
        assert_eq!(runtime.engine.state().active_contact_count(), 0);
    }

    fn joystick_binding(
        name: &str,
        up: &str,
        left: &str,
        down: &str,
        right: &str,
        center_x: f64,
    ) -> BindingV2 {
        BindingV2 {
            name: name.to_owned(),
            input: InputV2::KeyCluster {
                up: up.to_owned(),
                left: left.to_owned(),
                down: down.to_owned(),
                right: right.to_owned(),
            },
            action: ActionV2::VirtualJoystick {
                center: NormalizedPoint {
                    x: center_x,
                    y: 0.8,
                },
                radius: 0.1,
                dead_zone: 0.02,
                mode: JoystickMode::Hold,
                reaffirm_ms: Some(50),
            },
        }
    }

    fn peak_contacts(frames: &[TouchFrame]) -> usize {
        let mut active = BTreeSet::new();
        let mut peak = 0;
        for frame in frames {
            for event in frame.events() {
                match event.phase {
                    TouchPhase::Down => {
                        active.insert(event.contact_id);
                    }
                    TouchPhase::Up | TouchPhase::Cancel => {
                        active.remove(&event.contact_id);
                    }
                    TouchPhase::Move => {}
                }
            }
            peak = peak.max(active.len());
        }
        peak
    }

    #[test]
    fn session_metrics_track_frames_peak_contacts_and_percentiles() {
        let mut injector = SessionMetricsInjector::new(RecordingInjector::default());
        injector
            .inject(&TouchFrame::new([
                TouchEvent::new(ContactId::new(1), TouchPhase::Down, Point { x: 10, y: 10 }),
                TouchEvent::new(ContactId::new(2), TouchPhase::Down, Point { x: 20, y: 20 }),
            ]))
            .unwrap();
        injector
            .inject(&TouchFrame::new([
                TouchEvent::new(
                    ContactId::new(1),
                    TouchPhase::Cancel,
                    Point { x: 10, y: 10 },
                ),
                TouchEvent::new(
                    ContactId::new(2),
                    TouchPhase::Cancel,
                    Point { x: 20, y: 20 },
                ),
            ]))
            .unwrap();

        assert_eq!(injector.frames_submitted, 2);
        assert_eq!(injector.peak_contacts, 2);
        assert_eq!(
            latency_summary_micros(&[
                Duration::from_micros(10),
                Duration::from_micros(20),
                Duration::from_micros(30),
            ]),
            LatencySummary {
                samples: 3,
                p50: 20,
                p95: 30,
                p99: 30,
                max: 30,
            }
        );
        assert_eq!(
            latency_metrics(&[
                Duration::from_micros(10),
                Duration::from_micros(20),
                Duration::from_micros(30),
            ]),
            LatencyMetrics {
                samples: 3,
                p50_micros: 20,
                p95_micros: 30,
                p99_micros: 30,
                max_micros: 30,
            }
        );

        let recent = SystemTime::now() - Duration::from_millis(5);
        assert!(kernel_event_age(recent).is_some_and(|age| age >= Duration::from_millis(5)));
        assert!(kernel_event_age(SystemTime::UNIX_EPOCH).is_none());
        assert!(kernel_event_age(SystemTime::now() + Duration::from_secs(1)).is_none());
    }
}
