use std::error::Error;
use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use wroid_core::Point;
use wroid_inject::{
    ensure_container_stopped, ensure_root, remove_default_bridge, wait_for_android_input_device,
    DesktopUser, DesktopWaydroidSession, DeviceConfig, InputDeviceNode, InstalledWaydroidBridge,
    UinputTouchInjector, WROID_TOUCHSCREEN_NAME,
};
use wroid_runtime::{ContactId, TouchEngine};

const DEFAULT_WIDTH: u32 = 1920;
const DEFAULT_HEIGHT: u32 = 1080;
const DEFAULT_SAMPLES: usize = 20;
const DEFAULT_HOLD_MS: u64 = 20;
const DEFAULT_INTERVAL_MS: u64 = 50;
const DEFAULT_CAPTURE_STARTUP_MS: u64 = 1_000;
const GETEVENT_EVENTS_PER_TAP: usize = 16;
const GETEVENT_TIMEOUT_GRACE_SECONDS: u64 = 10;

fn main() -> Result<(), Box<dyn Error>> {
    ensure_root("Waydroid touch benchmark")?;

    let Some(options) = Options::parse(std::env::args().skip(1))? else {
        print_usage();
        return Ok(());
    };

    if options.cleanup {
        remove_default_bridge()?;
        println!("Removed the managed Wroid input bridge from the Waydroid LXC config.");
        return Ok(());
    }

    run_benchmark(options)
}

fn run_benchmark(options: Options) -> Result<(), Box<dyn Error>> {
    ensure_container_stopped()?;
    remove_default_bridge()?;

    let desktop_user = DesktopUser::from_sudo_environment()?;
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
        "Created {WROID_TOUCHSCREEN_NAME} at {}",
        event_node.display()
    );
    let bridge = InstalledWaydroidBridge::install_default(&input_node)?;
    println!("Installed a temporary, reversible Waydroid LXC input bridge.");

    let mut session = DesktopWaydroidSession::start(desktop_user)?;
    let benchmark_result = (|| -> Result<BenchmarkReport, Box<dyn Error>> {
        wait_for_android_input_device(WROID_TOUCHSCREEN_NAME)?;
        println!("Android detected {WROID_TOUCHSCREEN_NAME}.");
        if options.show_ui {
            session.show_full_ui()?;
            println!(
                "Requested Waydroid full UI. The benchmark does not require the window to appear."
            );
        }

        let capture = spawn_getevent_capture(&event_node, &options)?;
        println!(
            "Waiting {}ms for Android getevent capture to warm up.",
            options.capture_startup_ms
        );
        sleep(Duration::from_millis(options.capture_startup_ms));

        let mut engine = TouchEngine::new(injector);
        let frame_samples = inject_taps(&options, &mut engine)?;

        let captured = combined_output(&capture.wait_with_output()?);
        Ok(BenchmarkReport::new(frame_samples, captured))
    })();

    let stop_result = session.stop();
    let cleanup_result = bridge.cleanup();

    let report = benchmark_result?;
    stop_result?;
    cleanup_result?;

    report.print(options.print_events);
    println!("Waydroid stopped and the temporary LXC bridge was removed.");
    Ok(())
}

fn inject_taps(
    options: &Options,
    engine: &mut TouchEngine<UinputTouchInjector>,
) -> Result<Vec<Duration>, Box<dyn Error>> {
    let contact = ContactId::new(1);
    let hold = Duration::from_millis(options.hold_ms);
    let interval = Duration::from_millis(options.interval_ms);
    let mut frame_samples = Vec::with_capacity(options.samples * 2);

    println!(
        "Injecting {} tap(s): hold={}ms interval={}ms surface={}x{}",
        options.samples, options.hold_ms, options.interval_ms, options.width, options.height
    );

    for index in 0..options.samples {
        let point = tap_point(options, index);
        let down_started = Instant::now();
        engine.begin_contact(contact, point)?;
        frame_samples.push(down_started.elapsed());

        sleep(hold);

        let up_started = Instant::now();
        engine.end_contact(contact)?;
        frame_samples.push(up_started.elapsed());

        if index + 1 == 1 || (index + 1) % 10 == 0 || index + 1 == options.samples {
            println!(
                "progress: {}/{} tap(s) injected",
                index + 1,
                options.samples
            );
        }

        sleep(interval);
    }

    Ok(frame_samples)
}

fn tap_point(options: &Options, index: usize) -> Point {
    let lane = (index % 5) as u32;
    let x = options.width / 3 + lane * options.width / 15;
    let y = options.height / 2;
    Point {
        x: x.min(options.width.saturating_sub(1)),
        y: y.min(options.height.saturating_sub(1)),
    }
}

fn spawn_getevent_capture(event_node: &Path, options: &Options) -> io::Result<std::process::Child> {
    let event_path = event_node.to_str().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "event node path is not UTF-8")
    })?;
    let capture_count = options
        .samples
        .saturating_mul(GETEVENT_EVENTS_PER_TAP)
        .max(1);
    let expected_runtime_ms = options
        .samples
        .saturating_mul((options.hold_ms + options.interval_ms) as usize)
        as u64;
    let timeout_seconds = (expected_runtime_ms / 1_000 + GETEVENT_TIMEOUT_GRACE_SECONDS).max(1);

    println!(
        "Capturing up to {capture_count} Android getevent record(s), timeout={}s.",
        timeout_seconds
    );

    Command::new("timeout")
        .args([
            format!("{timeout_seconds}s"),
            "waydroid".to_owned(),
            "shell".to_owned(),
            "--".to_owned(),
            "getevent".to_owned(),
            "-lt".to_owned(),
            "-c".to_owned(),
            capture_count.to_string(),
            event_path.to_owned(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

#[derive(Debug)]
struct BenchmarkReport {
    frame_samples: Vec<Duration>,
    captured: String,
    android_downs: usize,
    android_ups: usize,
    android_tracking_updates: usize,
    android_syncs: usize,
}

impl BenchmarkReport {
    fn new(frame_samples: Vec<Duration>, captured: String) -> Self {
        let android_downs = count_any(
            &captured,
            &["0001 014a 00000001", "BTN_TOUCH            DOWN"],
        );
        let android_ups = count_any(
            &captured,
            &["0001 014a 00000000", "BTN_TOUCH            UP"],
        );
        let android_tracking_updates = count_any(&captured, &["0003 0039", "ABS_MT_TRACKING_ID"]);
        let android_syncs = count_any(&captured, &["0000 0000 00000000", "SYN_REPORT"]);
        Self {
            frame_samples,
            captured,
            android_downs,
            android_ups,
            android_tracking_updates,
            android_syncs,
        }
    }

    fn print(&self, print_events: bool) {
        println!();
        println!("Wroid Android-visible touch benchmark summary");
        println!("  host-injected frames: {}", self.frame_samples.len());
        println!("  Android BTN_TOUCH downs: {}", self.android_downs);
        println!("  Android BTN_TOUCH ups: {}", self.android_ups);
        println!(
            "  Android tracking-id updates: {}",
            self.android_tracking_updates
        );
        println!("  Android SYN_REPORT events: {}", self.android_syncs);
        print_distribution("host uinput frame injection", &self.frame_samples);

        if print_events {
            println!();
            println!(
                "Captured Android getevent output:\n{}",
                self.captured.trim()
            );
        } else {
            let preview = self
                .captured
                .lines()
                .take(20)
                .collect::<Vec<_>>()
                .join("\n");
            if !preview.trim().is_empty() {
                println!();
                println!("Captured Android getevent preview:\n{preview}");
                println!("Use --print-events to print the full getevent capture.");
            }
        }
    }
}

#[derive(Debug)]
struct Options {
    width: u32,
    height: u32,
    samples: usize,
    hold_ms: u64,
    interval_ms: u64,
    capture_startup_ms: u64,
    show_ui: bool,
    print_events: bool,
    cleanup: bool,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, Box<dyn Error>> {
        let mut options = Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
            samples: DEFAULT_SAMPLES,
            hold_ms: DEFAULT_HOLD_MS,
            interval_ms: DEFAULT_INTERVAL_MS,
            capture_startup_ms: DEFAULT_CAPTURE_STARTUP_MS,
            show_ui: true,
            print_events: false,
            cleanup: false,
        };

        let mut args = args.into_iter();
        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--cleanup" => options.cleanup = true,
                "--no-ui" => options.show_ui = false,
                "--print-events" => options.print_events = true,
                "--width" => {
                    options.width = parse_next(&mut args, "--width")?;
                    if options.width == 0 {
                        return Err(invalid_input("--width must be greater than zero"));
                    }
                }
                "--height" => {
                    options.height = parse_next(&mut args, "--height")?;
                    if options.height == 0 {
                        return Err(invalid_input("--height must be greater than zero"));
                    }
                }
                "--samples" => {
                    options.samples = parse_next(&mut args, "--samples")?;
                    if options.samples == 0 {
                        return Err(invalid_input("--samples must be greater than zero"));
                    }
                }
                "--hold-ms" => options.hold_ms = parse_next(&mut args, "--hold-ms")?,
                "--interval-ms" => options.interval_ms = parse_next(&mut args, "--interval-ms")?,
                "--capture-startup-ms" => {
                    options.capture_startup_ms = parse_next(&mut args, "--capture-startup-ms")?
                }
                value if value.starts_with("--") => {
                    return Err(invalid_input(format!("unknown option: {value}")));
                }
                value => {
                    return Err(invalid_input(format!(
                        "unexpected positional argument: {value}"
                    )))
                }
            }
        }

        Ok(Some(options))
    }
}

fn count_any(haystack: &str, patterns: &[&str]) -> usize {
    patterns
        .iter()
        .map(|pattern| haystack.matches(pattern).count())
        .max()
        .unwrap_or(0)
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

fn combined_output(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

fn print_usage() {
    println!(
        "Usage: wroid-waydroid-touch-bench [--samples N] [--width W] [--height H] [--hold-ms N] [--interval-ms N] [--capture-startup-ms N] [--no-ui] [--print-events] [--cleanup]"
    );
    println!(
        "Example: sudo ./target/release/wroid-waydroid-touch-bench --samples 20 --width 1920 --height 1080 --capture-startup-ms 1000 --no-ui"
    );
    println!("Recovery: sudo ./target/release/wroid-waydroid-touch-bench --cleanup");
}
