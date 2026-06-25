use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use wroid_core::{Point, Resolution};
use wroid_input::mouse::{
    EvdevMouse, MouseButton, MouseButtonEvent, MouseButtonTransition, MouseDeviceError,
    MouseEvent, RelativeMouseMotion,
};
use wroid_runtime::{
    ContactId, MouseAim, MouseAimConfigError, MouseAimDelta, MouseAimRegion,
    MouseAimSensitivity, TouchEngine, TouchEngineError, TouchInjector,
};

use crate::waydroid_bridge::{remove_default_bridge, InputDeviceNode, InstalledWaydroidBridge};
use crate::waydroid_session::{
    ensure_container_stopped, ensure_root, spawn_android_getevent_trace, stop_child,
    wait_for_android_boot_completed, wait_for_android_input_device, DesktopUser,
    DesktopWaydroidSession, WROID_TOUCHSCREEN_NAME,
};
use crate::{DeviceConfig, UinputTouchInjector};

pub const DEFAULT_MOUSE_AIM_WIDTH: u32 = 1920;
pub const DEFAULT_MOUSE_AIM_HEIGHT: u32 = 1080;
pub const DEFAULT_MOUSE_AIM_READY_DELAY: Duration = Duration::from_millis(1_000);

const MOUSE_AIM_CONTACT_ID: ContactId = ContactId::new(3);
const IDLE_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SIGINT: i32 = 2;
const SIGNAL_ERROR: usize = usize::MAX;

static INTERRUPT_REQUESTED: AtomicBool = AtomicBool::new(false);

type SignalHandler = extern "C" fn(i32);

unsafe extern "C" {
    fn signal(signum: i32, handler: SignalHandler) -> usize;
}

type LiveMouseAimResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAimAction {
    Activated,
    Moved,
    Released,
    Cancelled,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseAimBinding {
    pub activation_button: MouseButton,
}

impl Default for MouseAimBinding {
    fn default() -> Self {
        Self {
            activation_button: MouseButton::Right,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveMouseAimOptions {
    pub mouse_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub origin: Point,
    pub region: MouseAimRegion,
    pub sensitivity: MouseAimSensitivity,
    pub binding: MouseAimBinding,
    pub grab: bool,
    pub show_ui: bool,
    pub trace_android: bool,
    pub ready_delay: Duration,
}

impl LiveMouseAimOptions {
    pub fn new(mouse_path: impl Into<PathBuf>) -> Result<Self, MouseAimConfigError> {
        Self::with_resolution(
            mouse_path,
            DEFAULT_MOUSE_AIM_WIDTH,
            DEFAULT_MOUSE_AIM_HEIGHT,
        )
    }

    pub fn with_resolution(
        mouse_path: impl Into<PathBuf>,
        width: u32,
        height: u32,
    ) -> Result<Self, MouseAimConfigError> {
        let resolution = Resolution { width, height };
        Ok(Self {
            mouse_path: mouse_path.into(),
            width,
            height,
            origin: default_mouse_aim_origin(width, height),
            region: MouseAimRegion::full_surface(resolution)?,
            sensitivity: MouseAimSensitivity::one_to_one(),
            binding: MouseAimBinding::default(),
            grab: true,
            show_ui: true,
            trace_android: true,
            ready_delay: DEFAULT_MOUSE_AIM_READY_DELAY,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveMouseAimCommand {
    Run(LiveMouseAimOptions),
    Cleanup,
}

pub struct MouseAimController {
    aim: MouseAim,
    binding: MouseAimBinding,
}

impl MouseAimController {
    pub const fn new(aim: MouseAim, binding: MouseAimBinding) -> Self {
        Self { aim, binding }
    }

    pub const fn aim(&self) -> &MouseAim {
        &self.aim
    }

    pub const fn binding(&self) -> MouseAimBinding {
        self.binding
    }

    pub fn handle_event<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        event: MouseEvent,
    ) -> Result<MouseAimAction, TouchEngineError> {
        match event {
            MouseEvent::Button(event) => self.handle_button(engine, event),
            MouseEvent::Motion(motion) => self.handle_motion(engine, motion),
            MouseEvent::Wheel(_) => Ok(MouseAimAction::Ignored),
        }
    }

    pub fn cancel<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
    ) -> Result<MouseAimAction, TouchEngineError> {
        if self.aim.cancel(engine)? {
            Ok(MouseAimAction::Cancelled)
        } else {
            Ok(MouseAimAction::Ignored)
        }
    }

    fn handle_button<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        event: MouseButtonEvent,
    ) -> Result<MouseAimAction, TouchEngineError> {
        if event.button != self.binding.activation_button {
            return Ok(MouseAimAction::Ignored);
        }

        match event.transition {
            MouseButtonTransition::Pressed => {
                if self.aim.begin(engine)? {
                    Ok(MouseAimAction::Activated)
                } else {
                    Ok(MouseAimAction::Ignored)
                }
            }
            MouseButtonTransition::Released => {
                if self.aim.end(engine)? {
                    Ok(MouseAimAction::Released)
                } else {
                    Ok(MouseAimAction::Ignored)
                }
            }
            MouseButtonTransition::Repeated => Ok(MouseAimAction::Ignored),
        }
    }

    fn handle_motion<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        motion: RelativeMouseMotion,
    ) -> Result<MouseAimAction, TouchEngineError> {
        if self
            .aim
            .move_by(engine, MouseAimDelta::new(motion.dx, motion.dy))?
        {
            Ok(MouseAimAction::Moved)
        } else {
            Ok(MouseAimAction::Ignored)
        }
    }
}

pub fn default_mouse_aim_origin(width: u32, height: u32) -> Point {
    Point {
        x: width / 2,
        y: height / 2,
    }
}

pub fn run_live_mouse_aim_cli(args: &[String], binary_name: &str) -> LiveMouseAimResult<()> {
    if args.is_empty()
        || args
            .iter()
            .any(|argument| argument == "--help" || argument == "-h")
    {
        print_live_mouse_aim_usage(binary_name);
        return Ok(());
    }

    match parse_live_mouse_aim_command(args)? {
        LiveMouseAimCommand::Cleanup => cleanup_live_mouse_aim_bridge(),
        LiveMouseAimCommand::Run(options) => run_live_mouse_aim_session(options),
    }
}

pub fn cleanup_live_mouse_aim_bridge() -> LiveMouseAimResult<()> {
    ensure_root("Waydroid live mouse aim")?;
    remove_default_bridge()?;
    println!("Removed the managed Wroid input bridge from the Waydroid LXC config.");
    Ok(())
}

pub fn run_live_mouse_aim_session(options: LiveMouseAimOptions) -> LiveMouseAimResult<()> {
    ensure_root("Waydroid live mouse aim")?;
    ensure_container_stopped()?;
    remove_default_bridge()?;
    install_interrupt_handler()?;

    let desktop_user = DesktopUser::from_sudo_environment()?;
    let mut mouse = EvdevMouse::open(&options.mouse_path)?;
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

    println!("Mouse: {} ({})", mouse.name(), mouse.path().display());
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
    let aim = MouseAim::new(
        MOUSE_AIM_CONTACT_ID,
        options.origin,
        options.region,
        resolution,
        options.sensitivity,
    )?;
    let controller = MouseAimController::new(aim, options.binding);
    let mut engine = TouchEngine::new(injector);
    let mut trace: Option<Child> = None;

    let capture_result = (|| -> LiveMouseAimResult<()> {
        wait_for_android_boot_completed()?;
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
        if !options.ready_delay.is_zero() {
            println!(
                "Waiting {}ms for Android input stack to settle before enabling controls.",
                options.ready_delay.as_millis()
            );
            thread::sleep(options.ready_delay);
        }
        if options.grab {
            mouse.grab()?;
        }

        println!(
            "Mouse aim is live: hold {} mouse button to own one persistent Android aim contact; move mouse to move it; release to lift; Ctrl+C exits. Exclusive grab: {}. Origin={},{}. Region={},{},{},{}. Ready delay: {}.",
            options.binding.activation_button.profile_name(),
            if mouse.is_grabbed() {
                "enabled"
            } else {
                "disabled"
            },
            options.origin.x,
            options.origin.y,
            options.region.left,
            options.region.top,
            options.region.right,
            options.region.bottom,
            duration_label(options.ready_delay),
        );
        let reader = MouseReader::spawn(mouse);
        let loop_result = run_mouse_aim_loop(reader.receiver(), &controller, &mut engine);
        if loop_result.is_ok() && !interrupt_requested() {
            reader.join()?;
        }
        loop_result
    })();

    let contact_cleanup_result = controller.cancel(&mut engine);
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

    println!("Mouse aim capture stopped and the persistent contact was released.");
    println!("Waydroid stopped and the temporary LXC bridge was removed.");
    Ok(())
}

struct MouseReader {
    receiver: Receiver<MouseEvent>,
    handle: JoinHandle<Result<(), MouseDeviceError>>,
}

impl MouseReader {
    fn spawn(mut mouse: EvdevMouse) -> Self {
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || loop {
            for event in mouse.next_events()? {
                if sender.send(event).is_err() {
                    return Ok(());
                }
            }
        });

        Self { receiver, handle }
    }

    fn receiver(&self) -> &Receiver<MouseEvent> {
        &self.receiver
    }

    fn join(self) -> LiveMouseAimResult<()> {
        match self.handle.join() {
            Ok(result) => result.map_err(|error| error.into()),
            Err(_) => Err(io::Error::other("mouse reader thread panicked").into()),
        }
    }
}

fn run_mouse_aim_loop(
    receiver: &Receiver<MouseEvent>,
    controller: &MouseAimController,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> LiveMouseAimResult<()> {
    loop {
        if interrupt_requested() {
            println!("Interrupt requested; shutting down native mouse aim session.");
            return Ok(());
        }

        match receiver.recv_timeout(IDLE_POLL_INTERVAL) {
            Ok(event) => {
                log_mouse_action(controller.handle_event(engine, event)?);
                while let Ok(event) = receiver.try_recv() {
                    log_mouse_action(controller.handle_event(engine, event)?);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "mouse reader stopped before Ctrl+C was requested",
                )
                .into());
            }
        }
    }
}

fn log_mouse_action(action: MouseAimAction) {
    match action {
        MouseAimAction::Activated => println!("mouse aim activated"),
        MouseAimAction::Released => println!("mouse aim released"),
        MouseAimAction::Cancelled => println!("mouse aim cancelled"),
        MouseAimAction::Moved | MouseAimAction::Ignored => {}
    }
}

pub fn parse_live_mouse_aim_command(args: &[String]) -> LiveMouseAimResult<LiveMouseAimCommand> {
    if args.iter().any(|argument| argument == "--cleanup") {
        return Ok(LiveMouseAimCommand::Cleanup);
    }

    let mut positional = Vec::new();
    let mut grab = true;
    let mut show_ui = true;
    let mut trace_android = true;
    let mut ready_delay = DEFAULT_MOUSE_AIM_READY_DELAY;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--grab" => grab = true,
            "--no-grab" => grab = false,
            "--no-ui" => show_ui = false,
            "--no-trace" => trace_android = false,
            "--ready-delay-ms" => {
                index += 1;
                ready_delay = parse_ready_delay_arg(args.get(index), "--ready-delay-ms")?;
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

    let mouse_path = positional.first().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "mouse event node is required unless --cleanup is used",
        )
    })?;
    if positional.len() > 3 {
        return Err(
            io::Error::new(io::ErrorKind::InvalidInput, "too many positional arguments").into(),
        );
    }

    let width = parse_dimension(
        positional.get(1).map(String::as_str),
        DEFAULT_MOUSE_AIM_WIDTH,
        "width",
    )?;
    let height = parse_dimension(
        positional.get(2).map(String::as_str),
        DEFAULT_MOUSE_AIM_HEIGHT,
        "height",
    )?;

    let mut options = LiveMouseAimOptions::with_resolution(mouse_path, width, height)?;
    options.grab = grab;
    options.show_ui = show_ui;
    options.trace_android = trace_android;
    options.ready_delay = ready_delay;

    Ok(LiveMouseAimCommand::Run(options))
}

fn parse_dimension(value: Option<&str>, default: u32, label: &str) -> LiveMouseAimResult<u32> {
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

fn parse_ready_delay_arg(value: Option<&String>, flag: &str) -> LiveMouseAimResult<Duration> {
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
    Ok(Duration::from_millis(millis))
}

fn install_interrupt_handler() -> io::Result<()> {
    reset_interrupt_request();
    // SAFETY: `request_interrupt` only performs an atomic store and does not
    // allocate, lock, or call into non-signal-safe Rust code.
    let previous = unsafe { signal(SIGINT, request_interrupt) };
    if previous == SIGNAL_ERROR {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

extern "C" fn request_interrupt(_signal: i32) {
    INTERRUPT_REQUESTED.store(true, Ordering::SeqCst);
}

fn interrupt_requested() -> bool {
    INTERRUPT_REQUESTED.load(Ordering::SeqCst)
}

fn reset_interrupt_request() {
    INTERRUPT_REQUESTED.store(false, Ordering::SeqCst);
}

fn duration_label(duration: Duration) -> String {
    if duration.is_zero() {
        "disabled".to_owned()
    } else {
        format!("{}ms", duration.as_millis())
    }
}

pub fn print_live_mouse_aim_usage(binary_name: &str) {
    println!(
        "Usage: {binary_name} <mouse-event-node> [width] [height] [--no-grab] [--no-ui] [--no-trace] [--ready-delay-ms N]"
    );
    println!("Example: sudo ./target/release/{binary_name} /dev/input/event4 1920 1050");
    println!("Diagnostics without exclusive mouse grab: add --no-grab");
    println!("Controls: hold right mouse button to activate aim; move mouse to aim; release to lift");
    println!("Exit: Ctrl+C during the live control loop");
    println!("Recovery: sudo ./target/release/{binary_name} --cleanup");
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_runtime::{TouchFrame, TouchInjectionError, TouchPhase};

    #[derive(Debug, Default)]
    struct RecordingInjector {
        frames: Vec<TouchFrame>,
        fail_next: bool,
    }

    impl TouchInjector for RecordingInjector {
        fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
            if self.fail_next {
                self.fail_next = false;
                return Err(TouchInjectionError::new("injected failure"));
            }
            self.frames.push(frame.clone());
            Ok(())
        }
    }

    fn controller() -> MouseAimController {
        MouseAimController::new(
            MouseAim::new(
                ContactId::new(11),
                Point { x: 960, y: 540 },
                MouseAimRegion {
                    left: 600,
                    top: 200,
                    right: 1500,
                    bottom: 900,
                },
                Resolution {
                    width: 1920,
                    height: 1080,
                },
                MouseAimSensitivity::one_to_one(),
            )
            .unwrap(),
            MouseAimBinding::default(),
        )
    }

    fn button(button: MouseButton, transition: MouseButtonTransition) -> MouseEvent {
        MouseEvent::Button(MouseButtonEvent::new(button, transition))
    }

    fn run_options(command: LiveMouseAimCommand) -> LiveMouseAimOptions {
        match command {
            LiveMouseAimCommand::Run(options) => options,
            LiveMouseAimCommand::Cleanup => panic!("expected run command"),
        }
    }

    #[test]
    fn right_button_hold_activates_moves_and_releases_aim_contact() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Activated
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    MouseEvent::Motion(RelativeMouseMotion::new(10, -5)),
                )
                .unwrap(),
            MouseAimAction::Moved
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Released),
                )
                .unwrap(),
            MouseAimAction::Released
        );

        let frames = &engine.injector().frames;
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].events()[0].phase, TouchPhase::Down);
        assert_eq!(frames[1].events()[0].phase, TouchPhase::Move);
        assert_eq!(frames[2].events()[0].phase, TouchPhase::Up);
        assert!(!engine.state().is_active(controller.aim().contact_id()));
    }

    #[test]
    fn motion_before_activation_and_other_buttons_are_ignored() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    MouseEvent::Motion(RelativeMouseMotion::new(10, 10)),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Left, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );

        assert!(engine.injector().frames.is_empty());
    }

    #[test]
    fn repeated_activation_and_zero_motion_are_ignored() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Activated
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    MouseEvent::Motion(RelativeMouseMotion::new(0, 0)),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );

        assert_eq!(engine.injector().frames.len(), 1);
    }

    #[test]
    fn focus_loss_cancels_active_aim_contact() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        controller
            .handle_event(
                &mut engine,
                button(MouseButton::Right, MouseButtonTransition::Pressed),
            )
            .unwrap();

        assert_eq!(
            controller.cancel(&mut engine).unwrap(),
            MouseAimAction::Cancelled
        );
        assert_eq!(
            controller.cancel(&mut engine).unwrap(),
            MouseAimAction::Ignored
        );
        assert_eq!(
            engine.injector().frames.last().unwrap().events()[0].phase,
            TouchPhase::Cancel
        );
    }

    #[test]
    fn backend_failure_does_not_activate_contact() {
        let controller = controller();
        let injector = RecordingInjector {
            fail_next: true,
            ..RecordingInjector::default()
        };
        let mut engine = TouchEngine::new(injector);

        let error = controller
            .handle_event(
                &mut engine,
                button(MouseButton::Right, MouseButtonTransition::Pressed),
            )
            .unwrap_err();

        assert!(matches!(error, TouchEngineError::Injection(_)));
        assert!(!engine.state().is_active(controller.aim().contact_id()));
    }

    #[test]
    fn parses_default_mouse_aim_options() {
        let options =
            run_options(parse_live_mouse_aim_command(&["/dev/input/event4".to_owned()]).unwrap());

        assert_eq!(options.mouse_path, PathBuf::from("/dev/input/event4"));
        assert_eq!(options.width, DEFAULT_MOUSE_AIM_WIDTH);
        assert_eq!(options.height, DEFAULT_MOUSE_AIM_HEIGHT);
        assert_eq!(
            options.origin,
            default_mouse_aim_origin(DEFAULT_MOUSE_AIM_WIDTH, DEFAULT_MOUSE_AIM_HEIGHT)
        );
        assert_eq!(
            options.region,
            MouseAimRegion {
                left: 0,
                top: 0,
                right: DEFAULT_MOUSE_AIM_WIDTH - 1,
                bottom: DEFAULT_MOUSE_AIM_HEIGHT - 1,
            }
        );
        assert!(options.grab);
        assert!(options.show_ui);
        assert!(options.trace_android);
        assert_eq!(options.ready_delay, DEFAULT_MOUSE_AIM_READY_DELAY);
    }

    #[test]
    fn parses_diagnostic_mouse_aim_options() {
        let options = run_options(
            parse_live_mouse_aim_command(&[
                "/dev/input/event4".to_owned(),
                "1600".to_owned(),
                "900".to_owned(),
                "--no-grab".to_owned(),
                "--no-ui".to_owned(),
                "--no-trace".to_owned(),
                "--ready-delay-ms".to_owned(),
                "0".to_owned(),
            ])
            .unwrap(),
        );

        assert_eq!(options.width, 1600);
        assert_eq!(options.height, 900);
        assert_eq!(options.origin, Point { x: 800, y: 450 });
        assert_eq!(
            options.region,
            MouseAimRegion {
                left: 0,
                top: 0,
                right: 1599,
                bottom: 899,
            }
        );
        assert!(!options.grab);
        assert!(!options.show_ui);
        assert!(!options.trace_android);
        assert_eq!(options.ready_delay, Duration::ZERO);
    }

    #[test]
    fn parses_cleanup_without_mouse_path() {
        assert_eq!(
            parse_live_mouse_aim_command(&["--cleanup".to_owned()]).unwrap(),
            LiveMouseAimCommand::Cleanup
        );
    }

    #[test]
    fn rejects_missing_mouse_path() {
        let error = parse_live_mouse_aim_command(&["--no-ui".to_owned()]).unwrap_err();

        assert!(error.to_string().contains("mouse event node is required"));
    }
}
