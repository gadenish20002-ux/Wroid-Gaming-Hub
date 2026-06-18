use std::error::Error;
use std::io;
use std::path::Path;

use wroid_core::{Point, Resolution};
use wroid_inject::{DeviceConfig, UinputTouchInjector};
use wroid_input::{DirectionalKeyState, EvdevKeyboard, KeyboardAction};
use wroid_runtime::{ContactId, TouchEngine, VirtualJoystick};

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() || args.iter().any(|argument| argument == "--help") {
        print_usage();
        return Ok(());
    }

    let grab = args.iter().any(|argument| argument == "--grab");
    let positional = args
        .iter()
        .filter(|argument| argument.as_str() != "--grab")
        .collect::<Vec<_>>();
    let keyboard_path = positional
        .first()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "keyboard event node is required"))?;
    let width = parse_dimension(positional.get(1).map(String::as_str), 1920, "width")?;
    let height = parse_dimension(positional.get(2).map(String::as_str), 1080, "height")?;
    if positional.len() > 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "too many positional arguments",
        )
        .into());
    }

    let mut keyboard = EvdevKeyboard::open(keyboard_path)?;
    if grab {
        keyboard.grab()?;
    }

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

    let resolution = Resolution { width, height };
    let center = Point {
        x: width / 5,
        y: height.saturating_mul(4) / 5,
    };
    let radius = width.min(height).max(10) / 10;
    let joystick = VirtualJoystick::new(ContactId::new(1), center, radius, resolution)?;
    let mut engine = TouchEngine::new(injector);
    let mut state = DirectionalKeyState::default();

    println!("Keyboard: {} ({})", keyboard.name(), keyboard.path().display());
    println!("Virtual touchscreen: {}", event_node.display());
    println!(
        "Controls: W/A/S/D move the persistent touch contact; Esc exits. Exclusive grab: {}.",
        if keyboard.is_grabbed() { "enabled" } else { "disabled" }
    );
    println!("Attach evtest to the virtual touchscreen in a second terminal if desired.");

    'capture: loop {
        for event in keyboard.next_events()? {
            match state.apply(event) {
                KeyboardAction::DirectionChanged(input) => {
                    joystick.apply(&mut engine, input)?;
                    println!(
                        "direction up={} left={} down={} right={}",
                        input.up, input.left, input.down, input.right
                    );
                }
                KeyboardAction::ExitRequested => break 'capture,
                KeyboardAction::Ignored => {}
            }
        }
    }

    if let Some(neutral) = state.release_all() {
        joystick.apply(&mut engine, neutral)?;
    } else {
        joystick.cancel(&mut engine)?;
    }
    keyboard.ungrab()?;
    println!("Keyboard released and virtual touchscreen destroyed.");
    Ok(())
}

fn parse_dimension(
    value: Option<&str>,
    default: u32,
    label: &str,
) -> Result<u32, Box<dyn Error>> {
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
        "Usage: wroid-keyboard-joystick-smoke <keyboard-event-node> [width] [height] [--grab]"
    );
    println!("Example: sudo ./target/release/wroid-keyboard-joystick-smoke /dev/input/event7 1920 1050 --grab");
}
