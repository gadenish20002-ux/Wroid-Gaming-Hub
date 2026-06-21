use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use evdev::{Device, EventSummary, KeyCode};

const DEFAULT_TIMEOUT_SECONDS: u64 = 10;
const DEFAULT_GRAB_DELAY_MS: u64 = 750;
const POLL_INTERVAL: Duration = Duration::from_millis(1);

fn main() -> Result<(), Box<dyn Error>> {
    let Some(options) = Options::parse(env::args().skip(1))? else {
        print_usage();
        return Ok(());
    };

    let mut device = Device::open(&options.path)?;
    let name = device.name().unwrap_or("Unnamed evdev device").to_owned();
    println!("Device: {name} ({})", options.path.display());
    println!(
        "Dumping raw evdev events for up to {}s. Press W/A/S/D/Esc on the target keyboard.",
        options.timeout.as_secs()
    );

    if options.grab {
        println!(
            "Exclusive grab requested; arming in {}ms. Release Enter now.",
            options.grab_delay.as_millis()
        );
        thread::sleep(options.grab_delay);
        device.grab()?;
        println!("Exclusive grab: enabled");
    } else {
        println!("Exclusive grab: disabled");
    }

    device.set_nonblocking(true)?;

    let started = Instant::now();
    let mut polls = 0_u64;
    let mut empty_polls = 0_u64;
    let mut raw_events = 0_u64;
    let mut movement_events = 0_u64;

    while started.elapsed() < options.timeout {
        polls += 1;
        match device.fetch_events() {
            Ok(events) => {
                let mut saw_event = false;
                for event in events {
                    saw_event = true;
                    raw_events += 1;
                    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
                    let summary = event.destructure();
                    if is_movement_summary(&summary) {
                        movement_events += 1;
                        println!("{elapsed_ms:>10.3}ms MOVEMENT {summary:?}");
                    } else {
                        println!("{elapsed_ms:>10.3}ms          {summary:?}");
                    }
                }
                if !saw_event {
                    empty_polls += 1;
                    thread::sleep(POLL_INTERVAL);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                empty_polls += 1;
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) => return Err(Box::new(error)),
        }
    }

    if options.grab {
        let _ = device.ungrab();
    }

    println!();
    println!("Wroid evdev dump summary");
    println!("  polls: {polls}");
    println!("  empty polls: {empty_polls}");
    println!("  raw events: {raw_events}");
    println!("  W/A/S/D/Esc key events: {movement_events}");

    Ok(())
}

#[derive(Debug)]
struct Options {
    path: PathBuf,
    timeout: Duration,
    grab: bool,
    grab_delay: Duration,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, Box<dyn Error>> {
        let mut path = None;
        let mut timeout = Duration::from_secs(DEFAULT_TIMEOUT_SECONDS);
        let mut grab = false;
        let mut grab_delay = Duration::from_millis(DEFAULT_GRAB_DELAY_MS);

        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--grab" => grab = true,
                "--timeout-seconds" => {
                    let seconds: u64 = parse_next(&mut args, "--timeout-seconds")?;
                    if seconds == 0 {
                        return Err(invalid_input("--timeout-seconds must be greater than zero"));
                    }
                    timeout = Duration::from_secs(seconds);
                }
                "--grab-delay-ms" => {
                    let milliseconds: u64 = parse_next(&mut args, "--grab-delay-ms")?;
                    grab_delay = Duration::from_millis(milliseconds);
                }
                value if value.starts_with("--") => {
                    return Err(invalid_input(format!("unknown option: {value}")));
                }
                value => {
                    if path.replace(PathBuf::from(value)).is_some() {
                        return Err(invalid_input("only one event node is supported"));
                    }
                }
            }
        }

        let path = path.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing event node; pass /dev/input/eventN",
            )
        })?;

        Ok(Some(Self {
            path,
            timeout,
            grab,
            grab_delay,
        }))
    }
}

fn is_movement_summary(summary: &EventSummary) -> bool {
    matches!(
        summary,
        EventSummary::Key(
            _,
            KeyCode::KEY_W | KeyCode::KEY_A | KeyCode::KEY_S | KeyCode::KEY_D | KeyCode::KEY_ESC,
            _
        )
    )
}

fn parse_next<T>(args: &mut impl Iterator<Item = String>, option: &str) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    let value = args.next().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("missing {option} value"),
        )
    })?;
    value.parse::<T>().map_err(|source| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {option} value '{value}': {source}"),
        )
        .into()
    })
}

fn invalid_input(message: impl Into<String>) -> Box<dyn Error> {
    io::Error::new(io::ErrorKind::InvalidInput, message.into()).into()
}

fn print_usage() {
    println!(
        "Usage: wroid-evdev-dump <event-node> [--timeout-seconds N] [--grab-delay-ms N] [--grab]"
    );
    println!(
        "Example: sudo ./target/release/wroid-evdev-dump /dev/input/event7 --timeout-seconds 5 --grab-delay-ms 1500 --grab"
    );
}
