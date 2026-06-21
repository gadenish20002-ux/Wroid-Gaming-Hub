use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use wroid_core::{Point, Resolution};
use wroid_input::{DirectionalKeyState, EvdevKeyboard, KeyboardAction};
use wroid_runtime::{
    ContactId, TouchEngine, TouchFrame, TouchInjectionError, TouchInjector, VirtualJoystick,
};

const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
const DEFAULT_SAMPLES: usize = 200;

fn main() -> Result<(), Box<dyn Error>> {
    let Some(options) = Options::parse(env::args().skip(1))? else {
        print_usage();
        return Ok(());
    };

    let mut keyboard = EvdevKeyboard::open(&options.keyboard_path)?;
    if options.grab {
        keyboard.grab()?;
    }

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
    let mut engine = TouchEngine::new(RecordingInjector::default());
    let mut state = DirectionalKeyState::default();
    let mut stats = BenchStats::default();

    println!(
        "Keyboard: {} ({})",
        keyboard.name(),
        keyboard.path().display()
    );
    println!(
        "Collecting up to {} direction-change samples. Press/release W/A/S/D; press Esc to stop.",
        options.samples
    );
    println!(
        "Exclusive grab: {}. This benchmark measures host capture/normalization/runtime overhead; it does not include Android getevent timing.",
        if keyboard.is_grabbed() { "enabled" } else { "disabled" }
    );

    while stats.pipeline_samples.len() < options.samples && !stats.exit_requested {
        let read_started = Instant::now();
        let events = keyboard.next_events()?;
        stats.read_wait_samples.push(read_started.elapsed());

        for event in events {
            let pipeline_started = Instant::now();
            match state.apply(event) {
                KeyboardAction::DirectionChanged(input) => {
                    let submitted = joystick.apply(&mut engine, input)?;
                    stats.pipeline_samples.push(pipeline_started.elapsed());
                    if submitted {
                        stats.submitted_runtime_frames += 1;
                    }
                }
                KeyboardAction::ExitRequested => {
                    stats.exit_requested = true;
                    break;
                }
                KeyboardAction::Ignored => {
                    stats.ignored_events += 1;
                }
            }
        }
    }

    let injector = engine.injector();
    println!();
    println!("Wroid host input benchmark summary");
    println!("  direction-change samples: {}", stats.pipeline_samples.len());
    println!("  evdev read calls: {}", stats.read_wait_samples.len());
    println!("  ignored/repeat events: {}", stats.ignored_events);
    println!("  submitted runtime frames: {}", stats.submitted_runtime_frames);
    println!("  recorded injector frames: {}", injector.frames);
    println!("  recorded touch events: {}", injector.touch_events);
    print_distribution("host pipeline", &stats.pipeline_samples);
    print_distribution("evdev blocking read", &stats.read_wait_samples);

    Ok(())
}

#[derive(Debug)]
struct Options {
    keyboard_path: PathBuf,
    samples: usize,
    width: u32,
    height: u32,
    grab: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, Box<dyn Error>> {
        let mut keyboard_path = None;
        let mut samples = DEFAULT_SAMPLES;
        let mut width = DEFAULT_WIDTH;
        let mut height = DEFAULT_HEIGHT;
        let mut grab = false;

        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--grab" => grab = true,
                "--samples" => {
                    samples = parse_next(&mut args, "--samples")?;
                    if samples == 0 {
                        return Err(invalid_input("--samples must be greater than zero"));
                    }
                }
                "--width" => {
                    width = parse_next(&mut args, "--width")?;
                    if width == 0 {
                        return Err(invalid_input("--width must be greater than zero"));
                    }
                }
                "--height" => {
                    height = parse_next(&mut args, "--height")?;
                    if height == 0 {
                        return Err(invalid_input("--height must be greater than zero"));
                    }
                }
                value if value.starts_with("--") => {
                    return Err(invalid_input(format!("unknown option: {value}")));
                }
                value => {
                    if keyboard_path.replace(PathBuf::from(value)).is_some() {
                        return Err(invalid_input("only one keyboard event node is supported"));
                    }
                }
            }
        }

        let keyboard_path = keyboard_path.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing keyboard event node; pass /dev/input/eventN",
            )
        })?;

        Ok(Some(Self {
            keyboard_path,
            samples,
            width,
            height,
            grab,
        }))
    }
}

#[derive(Debug, Default)]
struct RecordingInjector {
    frames: u64,
    touch_events: u64,
}

impl TouchInjector for RecordingInjector {
    fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
        self.frames += 1;
        self.touch_events += frame.events().len() as u64;
        Ok(())
    }
}

#[derive(Debug, Default)]
struct BenchStats {
    pipeline_samples: Vec<Duration>,
    read_wait_samples: Vec<Duration>,
    ignored_events: u64,
    submitted_runtime_frames: u64,
    exit_requested: bool,
}

fn parse_next<T>(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: Error + Send + Sync + 'static,
{
    let value = args
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("missing {option} value")))?;
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

fn print_distribution(label: &str, samples: &[Duration]) {
    if samples.is_empty() {
        println!("  {label}: no samples");
        return;
    }

    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    println!(
        "  {label}: min={}us p50={}us p95={}us p99={}us max={}us",
        micros(sorted[0]),
        micros(percentile(&sorted, 50)),
        micros(percentile(&sorted, 95)),
        micros(percentile(&sorted, 99)),
        micros(*sorted.last().expect("non-empty samples have a last value")),
    );
}

fn percentile(sorted: &[Duration], percentile: usize) -> Duration {
    debug_assert!(!sorted.is_empty());
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted[index]
}

fn micros(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1_000_000.0)
}

fn print_usage() {
    println!(
        "Usage: wroid-bench-host <keyboard-event-node> [--samples N] [--width W] [--height H] [--grab]"
    );
    println!("Example: sudo ./target/release/wroid-bench-host /dev/input/event7 --samples 200 --grab");
    println!("Without --grab, the compositor and terminal can still receive keyboard input during diagnostics.");
}
