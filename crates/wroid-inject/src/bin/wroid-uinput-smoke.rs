use std::error::Error;
use std::io;
use std::thread::sleep;
use std::time::Duration;

use wroid_core::Point;
use wroid_inject::{DeviceConfig, UinputTouchInjector};
use wroid_runtime::{ContactId, TouchEngine};

fn main() -> Result<(), Box<dyn Error>> {
    let width = parse_dimension(1, 1920, "width")?;
    let height = parse_dimension(2, 1080, "height")?;
    let config = DeviceConfig::new(width, height)?;
    let injector = UinputTouchInjector::open(config)?;
    let mut engine = TouchEngine::new(injector);
    let contact = ContactId::new(1);

    wait_for_enter(
        "Wroid Gaming Touchscreen is active. Start evtest in another terminal, then press Enter.",
    )?;

    let start = Point {
        x: width / 3,
        y: height / 2,
    };
    let end = Point {
        x: width * 2 / 3,
        y: height / 2,
    };

    engine.begin_contact(contact, start)?;
    sleep(Duration::from_millis(250));
    engine.move_contact(contact, end)?;
    sleep(Duration::from_millis(250));
    engine.end_contact(contact)?;

    wait_for_enter(
        "Emitted one down/move/up sequence. Press Enter to destroy the virtual device.",
    )?;
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

fn wait_for_enter(message: &str) -> io::Result<()> {
    println!("{message}");
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(())
}
