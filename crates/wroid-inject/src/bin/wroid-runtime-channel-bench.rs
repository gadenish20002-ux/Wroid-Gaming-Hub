use std::error::Error;
use std::fmt;
use std::io;
use std::os::fd::OwnedFd;
use std::thread;
use std::time::{Duration, Instant};

use wroid_core::{Point, Resolution};
use wroid_inject::{
    runtime_socket_pair, serve_runtime_attachment, DeviceConfig, RuntimeAttachmentReport,
    RuntimeChannelClient, RuntimeChannelServer, UinputTouchInjector,
};
use wroid_runtime::{ContactId, TouchEngine, TouchEvent, TouchFrame, TouchInjector, TouchPhase};

const CANONICAL_EXTENT: u32 = 65_536;
const CONTACT_COUNT: usize = 10;
const ACKNOWLEDGED_MOVE_FRAMES: u64 = 20_000;
const MIN_ACKNOWLEDGED_FRAMES: u64 = 20_000;
const P99_BUDGET_MICROS: u128 = 5_000;

fn main() -> Result<(), Box<dyn Error>> {
    let config =
        DeviceConfig::with_slots(CANONICAL_EXTENT, CANONICAL_EXTENT, CONTACT_COUNT as u16)?;
    let engine = TouchEngine::new(UinputTouchInjector::open(config)?);
    let (client_fd, server_fd) = runtime_socket_pair()?;
    let server = thread::spawn(move || run_benchmark_server(server_fd, engine));
    let summary = run_20k_acknowledged_frames(client_fd)?;
    let report = server
        .join()
        .map_err(|_| io::Error::other("benchmark server panicked"))??;
    let gate = validate_gate(&summary, &report);
    print_summary(&summary, &report, gate.as_ref().err());
    gate?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClientSummary {
    acknowledged_frames: u64,
    released_contacts: usize,
    p50_micros: u128,
    p95_micros: u128,
    p99_micros: u128,
    max_micros: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BenchmarkGateError {
    detail: String,
}

impl BenchmarkGateError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for BenchmarkGateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for BenchmarkGateError {}

fn run_benchmark_server(
    server_fd: OwnedFd,
    mut engine: TouchEngine<UinputTouchInjector>,
) -> io::Result<RuntimeAttachmentReport> {
    let server = RuntimeChannelServer::from_owned_fd(server_fd)?;
    serve_runtime_attachment(server, benchmark_resolution(), &mut engine, || Ok(()))
}

fn run_20k_acknowledged_frames(client_fd: OwnedFd) -> Result<ClientSummary, Box<dyn Error>> {
    let mut client = RuntimeChannelClient::from_owned_fd(client_fd)?;
    client.wait_until_ready()?;

    let mut samples = Vec::with_capacity(ACKNOWLEDGED_MOVE_FRAMES as usize + 2);
    submit_acknowledged(
        &mut client,
        ten_contact_frame(TouchPhase::Down, 0),
        &mut samples,
    )?;

    for step in 0..ACKNOWLEDGED_MOVE_FRAMES {
        submit_acknowledged(&mut client, move_frame(step), &mut samples)?;
    }

    submit_acknowledged(
        &mut client,
        ten_contact_frame(TouchPhase::Up, ACKNOWLEDGED_MOVE_FRAMES + 1),
        &mut samples,
    )?;
    client.finish()?;

    Ok(summarize_latencies(
        samples.len() as u64,
        CONTACT_COUNT,
        samples,
    ))
}

fn submit_acknowledged(
    client: &mut RuntimeChannelClient,
    frame: TouchFrame,
    samples: &mut Vec<Duration>,
) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    client.inject(&frame)?;
    samples.push(started.elapsed());
    Ok(())
}

fn benchmark_resolution() -> Resolution {
    Resolution {
        width: CANONICAL_EXTENT,
        height: CANONICAL_EXTENT,
    }
}

fn ten_contact_frame(phase: TouchPhase, step: u64) -> TouchFrame {
    TouchFrame::new(
        (0..CONTACT_COUNT)
            .map(|index| TouchEvent::new(contact_id(index), phase, position_for(index, step))),
    )
}

fn move_frame(step: u64) -> TouchFrame {
    let index = (step as usize) % CONTACT_COUNT;
    TouchFrame::single(TouchEvent::new(
        contact_id(index),
        TouchPhase::Move,
        position_for(index, step + 1),
    ))
}

fn contact_id(index: usize) -> ContactId {
    ContactId::new(index as u16 + 1)
}

fn position_for(index: usize, step: u64) -> Point {
    let contact_offset = index as u32 * 4_099;
    let step = step as u32;
    Point {
        x: step
            .wrapping_mul(37)
            .wrapping_add(contact_offset)
            .wrapping_add(257)
            % CANONICAL_EXTENT,
        y: step
            .wrapping_mul(53)
            .wrapping_add(contact_offset.wrapping_mul(3))
            .wrapping_add(911)
            % CANONICAL_EXTENT,
    }
}

fn summarize_latencies(
    acknowledged_frames: u64,
    released_contacts: usize,
    mut samples: Vec<Duration>,
) -> ClientSummary {
    if samples.is_empty() {
        return ClientSummary {
            acknowledged_frames,
            released_contacts,
            p50_micros: 0,
            p95_micros: 0,
            p99_micros: 0,
            max_micros: 0,
        };
    }

    samples.sort_unstable();
    ClientSummary {
        acknowledged_frames,
        released_contacts,
        p50_micros: percentile_micros(&samples, 50),
        p95_micros: percentile_micros(&samples, 95),
        p99_micros: percentile_micros(&samples, 99),
        max_micros: samples.last().copied().unwrap_or_default().as_micros(),
    }
}

fn percentile_micros(sorted: &[Duration], percent: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * percent)
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[rank].as_micros()
}

fn validate_gate(
    summary: &ClientSummary,
    report: &RuntimeAttachmentReport,
) -> Result<(), BenchmarkGateError> {
    if summary.acknowledged_frames < MIN_ACKNOWLEDGED_FRAMES {
        return Err(BenchmarkGateError::new(format!(
            "expected at least 20000 acknowledged frames, got {}",
            summary.acknowledged_frames
        )));
    }
    if report.frames_submitted != summary.acknowledged_frames {
        return Err(BenchmarkGateError::new(format!(
            "client acknowledged {} frames but server submitted {}",
            summary.acknowledged_frames, report.frames_submitted
        )));
    }
    if report.peak_contacts != CONTACT_COUNT {
        return Err(BenchmarkGateError::new(format!(
            "expected peak contacts {CONTACT_COUNT}, got {}",
            report.peak_contacts
        )));
    }
    if summary.released_contacts != CONTACT_COUNT {
        return Err(BenchmarkGateError::new(format!(
            "released {}/{CONTACT_COUNT} contacts",
            summary.released_contacts
        )));
    }
    if report.contacts_cancelled != 0 {
        return Err(BenchmarkGateError::new(format!(
            "daemon finished with {} active contacts",
            report.contacts_cancelled
        )));
    }
    if summary.p99_micros >= P99_BUDGET_MICROS {
        return Err(BenchmarkGateError::new(format!(
            "runtime channel p99 {} us must be below {P99_BUDGET_MICROS} us",
            summary.p99_micros
        )));
    }
    Ok(())
}

fn print_summary(
    summary: &ClientSummary,
    report: &RuntimeAttachmentReport,
    gate_error: Option<&BenchmarkGateError>,
) {
    println!("runtime_channel_frames={}", summary.acknowledged_frames);
    println!("runtime_channel_server_frames={}", report.frames_submitted);
    println!("runtime_channel_peak_contacts={}", report.peak_contacts);
    println!(
        "runtime_channel_released_contacts={}",
        summary.released_contacts
    );
    println!(
        "runtime_channel_active_contacts={}",
        report.contacts_cancelled
    );
    println!("runtime_channel_p50_micros={}", summary.p50_micros);
    println!("runtime_channel_p95_micros={}", summary.p95_micros);
    println!("runtime_channel_p99_micros={}", summary.p99_micros);
    println!("runtime_channel_max_micros={}", summary.max_micros);
    println!(
        "runtime_channel_result={}",
        if gate_error.is_none() { "PASS" } else { "FAIL" }
    );
    if let Some(error) = gate_error {
        println!("runtime_channel_error={error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use wroid_inject::RuntimeAttachmentReport;

    fn report(
        frames_submitted: u64,
        peak_contacts: usize,
        contacts_cancelled: usize,
    ) -> RuntimeAttachmentReport {
        RuntimeAttachmentReport {
            frames_submitted,
            peak_contacts,
            contacts_cancelled,
        }
    }

    fn summary(
        acknowledged_frames: u64,
        released_contacts: usize,
        p99_micros: u128,
    ) -> ClientSummary {
        ClientSummary {
            acknowledged_frames,
            released_contacts,
            p50_micros: 50,
            p95_micros: 95,
            p99_micros,
            max_micros: p99_micros,
        }
    }

    #[test]
    fn summarizes_latency_percentiles_as_integer_microseconds() {
        let samples = (1..=100).map(Duration::from_micros).collect::<Vec<_>>();

        let summary = summarize_latencies(12_345, 10, samples);

        assert_eq!(summary.acknowledged_frames, 12_345);
        assert_eq!(summary.released_contacts, 10);
        assert_eq!(summary.p50_micros, 50);
        assert_eq!(summary.p95_micros, 95);
        assert_eq!(summary.p99_micros, 99);
        assert_eq!(summary.max_micros, 100);
    }

    #[test]
    fn gate_requires_acknowledged_frames_release_and_empty_daemon_state() {
        validate_gate(&summary(20_002, 10, 4_999), &report(20_002, 10, 0)).unwrap();

        let too_few_frames =
            validate_gate(&summary(19_999, 10, 100), &report(19_999, 10, 0)).unwrap_err();
        assert!(too_few_frames
            .to_string()
            .contains("at least 20000 acknowledged frames"));

        let incomplete_release =
            validate_gate(&summary(20_002, 9, 100), &report(20_002, 10, 0)).unwrap_err();
        assert!(incomplete_release.to_string().contains("released 9/10"));

        let leaked_daemon_contacts =
            validate_gate(&summary(20_002, 10, 100), &report(20_002, 10, 1)).unwrap_err();
        assert!(leaked_daemon_contacts
            .to_string()
            .contains("active contacts"));

        let slow_tail =
            validate_gate(&summary(20_002, 10, 5_000), &report(20_002, 10, 0)).unwrap_err();
        assert!(slow_tail.to_string().contains("p99"));
    }
}
