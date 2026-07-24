use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use wroid_core::profile_v2::{InputV2, ProfileV2};
use wroid_core::{Point, Resolution};
use wroid_input::mouse::{EvdevMouse, MouseButtonTransition, MouseEvent, RelativeMouseMotion};
use wroid_input::{EvdevKeyboard, HostKeyEvent, KeyTransition};
use wroid_inject::{
    ensure_container_stopped, ensure_root, remove_default_bridge, wait_for_android_boot_completed,
    wait_for_android_input_device, DesktopUser, DesktopWaydroidSession, DeviceConfig,
    InputDeviceNode, InstalledWaydroidBridge, UinputTouchInjector, WROID_TOUCHSCREEN_NAME,
};
use wroid_runtime::{
    ContactId, DirectionalInput, MouseAimDelta, RuntimeControlAction, RuntimeControlPlan,
    TouchEngine, TouchEvent, TouchFrame, TouchPhase,
};

const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
const IDLE_POLL: Duration = Duration::from_millis(50);

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

fn main() -> Result<()> {
    let Some(options) = Options::parse(std::env::args().skip(1))? else {
        print_usage();
        return Ok(());
    };

    if options.cleanup {
        ensure_root("Wroid unified game session cleanup")?;
        remove_default_bridge()?;
        println!("Removed the managed Wroid input bridge.");
        return Ok(());
    }

    run(options)
}

fn run(options: Options) -> Result<()> {
    ensure_root("Wroid unified game session")?;
    ensure_container_stopped()?;
    remove_default_bridge()?;

    let profile_path = required_path(&options.profile_path, "profile")?;
    let keyboard_path = required_path(&options.keyboard_path, "keyboard")?;
    let mouse_path = required_path(&options.mouse_path, "mouse")?;
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
    let mut mouse = EvdevMouse::open(mouse_path)?;
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
    let bridge = InstalledWaydroidBridge::install_default(&input_node)?;
    let desktop_user = DesktopUser::from_sudo_environment()?;
    let mut waydroid = DesktopWaydroidSession::start(desktop_user)?;

    let session_result = (|| -> Result<()> {
        wait_for_android_boot_completed()?;
        wait_for_android_input_device(WROID_TOUCHSCREEN_NAME)?;
        if options.show_ui {
            waydroid.show_full_ui()?;
        }
        if options.grab {
            keyboard.grab()?;
            mouse.grab()?;
        }

        println!("Unified game session is live.");
        println!("Profile: {} ({})", plan.profile_name, plan.package_name);
        println!("Keyboard: {} ({})", keyboard.name(), keyboard.path().display());
        println!("Mouse: {} ({})", mouse.name(), mouse.path().display());
        println!("Android touchscreen: {}", event_node.display());
        println!("Controls: {}", plan.controls.len());
        println!("Press Esc to stop. Active contacts are cancelled on shutdown.");

        let (sender, receiver) = mpsc::channel();
        spawn_keyboard_reader(keyboard, sender.clone());
        spawn_mouse_reader(mouse, sender);

        let mut runtime = UnifiedRuntime::new(plan, injector);
        let loop_result = run_event_loop(&receiver, &mut runtime);
        let cleanup_result = runtime.stop();
        loop_result?;
        cleanup_result?;
        Ok(())
    })();

    let stop_result = waydroid.stop();
    let bridge_result = bridge.cleanup();
    session_result?;
    stop_result?;
    bridge_result?;
    println!("Unified game session stopped cleanly.");
    Ok(())
}

fn required_path<'a>(path: &'a Option<PathBuf>, label: &str) -> Result<&'a Path> {
    path.as_deref()
        .ok_or_else(|| invalid_input(format!("missing {label} path")))
}

#[derive(Debug)]
enum HostEvent {
    Keyboard(HostKeyEvent),
    Mouse(MouseEvent),
    ReaderFailed(String),
}

fn spawn_keyboard_reader(mut keyboard: EvdevKeyboard, sender: Sender<HostEvent>) {
    thread::spawn(move || loop {
        match keyboard.next_host_key_events() {
            Ok(events) => {
                for event in events {
                    if sender.send(HostEvent::Keyboard(event)).is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(HostEvent::ReaderFailed(error.to_string()));
                return;
            }
        }
    });
}

fn spawn_mouse_reader(mut mouse: EvdevMouse, sender: Sender<HostEvent>) {
    thread::spawn(move || loop {
        match mouse.next_events() {
            Ok(events) => {
                for event in events {
                    if sender.send(HostEvent::Mouse(event)).is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = sender.send(HostEvent::ReaderFailed(error.to_string()));
                return;
            }
        }
    });
}

fn run_event_loop(receiver: &Receiver<HostEvent>, runtime: &mut UnifiedRuntime) -> Result<()> {
    loop {
        match receiver.recv_timeout(IDLE_POLL) {
            Ok(HostEvent::Keyboard(event)) => {
                if event.key.profile_name() == "esc" && event.transition == KeyTransition::Pressed {
                    return Ok(());
                }
                runtime.handle_keyboard(event)?;
            }
            Ok(HostEvent::Mouse(event)) => runtime.handle_mouse(event)?,
            Ok(HostEvent::ReaderFailed(message)) => {
                return Err(io::Error::other(format!("input reader failed: {message}")).into());
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

struct UnifiedRuntime {
    plan: RuntimeControlPlan,
    engine: TouchEngine<UinputTouchInjector>,
    directions: BTreeMap<String, DirectionalInput>,
    tap_contacts: BTreeMap<String, ContactId>,
}

impl UnifiedRuntime {
    fn new(plan: RuntimeControlPlan, injector: UinputTouchInjector) -> Self {
        let mut next_contact = plan
            .controls
            .iter()
            .filter_map(|control| match &control.action {
                RuntimeControlAction::VirtualJoystick { joystick } => {
                    Some(joystick.contact_id().get())
                }
                RuntimeControlAction::MouseAim { aim } => Some(aim.contact_id().get()),
                RuntimeControlAction::Tap { .. } => None,
            })
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut tap_contacts = BTreeMap::new();
        for control in &plan.controls {
            if matches!(control.action, RuntimeControlAction::Tap { .. }) {
                tap_contacts.insert(control.name.clone(), ContactId::new(next_contact));
                next_contact = next_contact.saturating_add(1);
            }
        }
        Self {
            plan,
            engine: TouchEngine::new(injector),
            directions: BTreeMap::new(),
            tap_contacts,
        }
    }

    fn handle_keyboard(&mut self, event: HostKeyEvent) -> Result<()> {
        let key = event.key.profile_name();
        let pressed = match event.transition {
            KeyTransition::Pressed => true,
            KeyTransition::Released => false,
            KeyTransition::Repeated => return Ok(()),
        };

        for control in self.plan.controls.clone() {
            match (&control.input, &control.action) {
                (InputV2::Key { key: binding_key }, RuntimeControlAction::Tap { point })
                    if pressed && key_eq(binding_key, key) =>
                {
                    self.tap(&control.name, *point)?;
                }
                (
                    InputV2::KeyCluster {
                        up,
                        left,
                        down,
                        right,
                    },
                    RuntimeControlAction::VirtualJoystick { joystick },
                ) => {
                    let direction = self.directions.entry(control.name.clone()).or_default();
                    let mut changed = false;
                    changed |= update_direction(&mut direction.up, up, key, pressed);
                    changed |= update_direction(&mut direction.left, left, key, pressed);
                    changed |= update_direction(&mut direction.down, down, key, pressed);
                    changed |= update_direction(&mut direction.right, right, key, pressed);
                    if changed {
                        joystick.apply(&mut self.engine, *direction)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_mouse(&mut self, event: MouseEvent) -> Result<()> {
        match event {
            MouseEvent::Motion(RelativeMouseMotion { dx, dy }) if dx != 0 || dy != 0 => {
                for control in self.plan.controls.clone() {
                    if let (InputV2::MouseMove, RuntimeControlAction::MouseAim { aim }) =
                        (&control.input, &control.action)
                    {
                        aim.begin(&mut self.engine)?;
                        aim.move_by(&mut self.engine, MouseAimDelta::new(dx, dy))?;
                    }
                }
            }
            MouseEvent::Button(button_event)
                if button_event.transition == MouseButtonTransition::Pressed =>
            {
                let button = button_event.button.profile_name();
                for control in self.plan.controls.clone() {
                    if let (
                        InputV2::MouseButton { button: binding_button },
                        RuntimeControlAction::Tap { point },
                    ) = (&control.input, &control.action)
                    {
                        if key_eq(binding_button, button) {
                            self.tap(&control.name, *point)?;
                        }
                    }
                }
            }
            MouseEvent::Motion(_) | MouseEvent::Button(_) | MouseEvent::Wheel(_) => {}
        }
        Ok(())
    }

    fn tap(&mut self, binding: &str, point: Point) -> Result<()> {
        let contact_id = *self
            .tap_contacts
            .get(binding)
            .ok_or_else(|| io::Error::other(format!("missing tap contact for {binding}")))?;
        self.engine.submit(TouchFrame::single(TouchEvent::new(
            contact_id,
            TouchPhase::Down,
            point,
        )))?;
        self.engine.submit(TouchFrame::single(TouchEvent::new(
            contact_id,
            TouchPhase::Up,
            point,
        )))?;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        self.directions.clear();
        self.engine.cancel_all()?;
        Ok(())
    }
}

fn update_direction(state: &mut bool, binding_key: &str, event_key: &str, pressed: bool) -> bool {
    if !key_eq(binding_key, event_key) || *state == pressed {
        return false;
    }
    *state = pressed;
    true
}

fn key_eq(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

#[derive(Debug)]
struct Options {
    profile_path: Option<PathBuf>,
    keyboard_path: Option<PathBuf>,
    mouse_path: Option<PathBuf>,
    width: u32,
    height: u32,
    grab: bool,
    show_ui: bool,
    cleanup: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>> {
        let mut positional = Vec::new();
        let mut options = Self {
            profile_path: None,
            keyboard_path: None,
            mouse_path: None,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            grab: true,
            show_ui: true,
            cleanup: false,
        };
        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--cleanup" => options.cleanup = true,
                "--no-grab" => options.grab = false,
                "--no-ui" => options.show_ui = false,
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
        if positional.len() != 3 {
            return Err(invalid_input(
                "expected <profile-v2.json> <keyboard-event-node> <mouse-event-node>",
            ));
        }
        options.profile_path = Some(positional.remove(0));
        options.keyboard_path = Some(positional.remove(0));
        options.mouse_path = Some(positional.remove(0));
        Ok(Some(options))
    }
}

fn parse_next<T>(args: &mut impl Iterator<Item = String>, label: &str) -> Result<T>
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

fn print_usage() {
    println!(
        "Usage: wroid-waydroid-game-session <profile-v2.json> <keyboard-event-node> <mouse-event-node> [--width W] [--height H] [--no-grab] [--no-ui]"
    );
    println!(
        "Example: sudo ./target/release/wroid-waydroid-game-session profiles/examples/shooter-v2.json /dev/input/event7 /dev/input/event9 --width 1920 --height 1080"
    );
    println!("Recovery: sudo ./target/release/wroid-waydroid-game-session --cleanup");
}
