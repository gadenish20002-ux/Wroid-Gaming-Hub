//! Measure the cost of the touch-injection hot path itself.
//!
//! Every gameplay input ends in a `TouchEngine` submit that writes a
//! synchronized frame to the virtual touchscreen. This benchmark isolates that
//! write from evdev capture and from Android, so a regression in the injector,
//! the runtime state commit, or the uinput protocol shows up directly as
//! per-frame latency.
//!
//! It needs no root, no Waydroid session, and no physical device grab: it only
//! creates the same virtual touchscreen production sessions use.

use std::error::Error;
use std::time::{Duration, Instant};

use wroid_core::Point;
use wroid_inject::{DeviceConfig, UinputTouchInjector};
use wroid_runtime::{ContactId, TouchEngine};

const DEFAULT_SAMPLES: usize = 20_000;
const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
/// Per-frame budget from docs/performance-budget.md.
const BUDGET: Duration = Duration::from_millis(5);

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::from_args()?;
    let config = DeviceConfig::new(options.width, options.height)?;
    let injector = UinputTouchInjector::open(config)?;
    let mut engine = TouchEngine::new(injector);
    let contact = ContactId::new(1);

    let origin = Point {
        x: options.width / 2,
        y: options.height / 2,
    };
    engine.begin_contact(contact, origin)?;

    // Warm up so first-touch allocation and kernel setup stay out of the
    // reported distribution.
    for step in 0..256 {
        engine.move_contact(contact, sweep(origin, step, options.width, options.height))?;
    }

    let mut samples = Vec::with_capacity(options.samples);
    for step in 0..options.samples {
        let target = sweep(origin, step as u32, options.width, options.height);
        let started = Instant::now();
        engine.move_contact(contact, target)?;
        samples.push(started.elapsed());
    }
    engine.end_contact(contact)?;

    report(&mut samples, options.samples);
    verify_simultaneous_contacts(&mut engine, options.width, options.height)?;
    Ok(())
}

/// Drive every slot the virtual touchscreen advertises at once, on the real
/// device rather than a recording sink.
///
/// Popular shooters place movement, aim, fire, and several HUD controls under
/// simultaneous fingers, so losing a slot silently breaks gameplay. Holding all
/// of them through the production injector proves the kernel accepted the
/// advertised slot count.
fn verify_simultaneous_contacts<I: wroid_runtime::TouchInjector>(
    engine: &mut TouchEngine<I>,
    width: u32,
    height: u32,
) -> Result<(), Box<dyn Error>> {
    let expected = usize::from(wroid_inject::DEFAULT_SLOT_COUNT);
    for slot in 0..expected {
        engine.begin_contact(
            ContactId::new(slot as u16 + 1),
            Point {
                x: width / 4 + (slot as u32 * width / 32),
                y: height / 2,
            },
        )?;
    }

    let active = engine.state().active_contact_count();
    println!("simultaneous contacts: {active}/{expected}");
    if active != expected {
        return Err(format!("expected {expected} simultaneous contacts, held {active}").into());
    }

    for slot in 0..expected {
        engine.end_contact(ContactId::new(slot as u16 + 1))?;
    }
    if engine.state().active_contact_count() != 0 {
        return Err("contacts remained active after release".into());
    }
    println!("  all slots released cleanly");
    Ok(())
}

/// Walk the contact around the surface so consecutive frames always carry a
/// real coordinate change and cannot be skipped as a no-op.
fn sweep(origin: Point, step: u32, width: u32, height: u32) -> Point {
    let span_x = width / 4;
    let span_y = height / 4;
    Point {
        x: origin.x - span_x / 2 + step % span_x,
        y: origin.y - span_y / 2 + (step / span_x) % span_y,
    }
}

fn report(samples: &mut [Duration], requested: usize) {
    samples.sort_unstable();
    let total: Duration = samples.iter().sum();
    println!("touch-frame injection latency over {requested} frames");
    println!("  mean  {:>9.1} us", micros(total / samples.len() as u32));
    println!("  p50   {:>9.1} us", micros(percentile(samples, 50.0)));
    println!("  p95   {:>9.1} us", micros(percentile(samples, 95.0)));
    println!("  p99   {:>9.1} us", micros(percentile(samples, 99.0)));
    println!("  max   {:>9.1} us", micros(*samples.last().unwrap()));

    let over_budget = samples.iter().filter(|sample| **sample > BUDGET).count();
    if over_budget == 0 {
        println!("  all frames stayed inside the {BUDGET:?} budget");
    } else {
        println!("  WARNING: {over_budget} frames exceeded the {BUDGET:?} budget");
    }
}

fn percentile(sorted: &[Duration], percent: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = (percent / 100.0 * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn micros(value: Duration) -> f64 {
    value.as_secs_f64() * 1_000_000.0
}

struct Options {
    samples: usize,
    width: u32,
    height: u32,
}

impl Options {
    fn from_args() -> Result<Self, Box<dyn Error>> {
        let mut options = Self {
            samples: DEFAULT_SAMPLES,
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--samples" => options.samples = next_value(&mut args, "--samples")?,
                "--width" => options.width = next_value(&mut args, "--width")?,
                "--height" => options.height = next_value(&mut args, "--height")?,
                "--help" | "-h" => {
                    println!("Usage: wroid-inject-latency [--samples N] [--width W] [--height H]");
                    std::process::exit(0);
                }
                other => return Err(format!("unrecognized argument '{other}'").into()),
            }
        }
        if options.samples == 0 {
            return Err("--samples must be greater than zero".into());
        }
        Ok(options)
    }
}

fn next_value<T>(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<T, Box<dyn Error>>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let raw = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
    raw.parse()
        .map_err(|error| format!("invalid {flag} value '{raw}': {error}").into())
}
