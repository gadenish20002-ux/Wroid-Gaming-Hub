use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use wroid_runtime::RendererKind;

use super::terminal::spawn_terminal;

const WAYDROID_CONFIG: &str = "/var/lib/waydroid/waydroid.cfg";
const GPU_SESSION_ATTEMPTS: usize = 120;
const GPU_SESSION_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostRenderer {
    pub(crate) kind: RendererKind,
    pub(crate) renderer: Option<String>,
    pub(crate) vendor: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) direct: Option<bool>,
    pub(crate) accelerated: Option<bool>,
    pub(crate) source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrmDevice {
    pub(crate) card: String,
    pub(crate) render_node: Option<String>,
    pub(crate) driver: Option<String>,
    pub(crate) vendor: Option<String>,
    pub(crate) device_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AndroidGraphics {
    pub(crate) egl: Option<String>,
    pub(crate) gralloc: Option<String>,
    pub(crate) vulkan: Option<String>,
    pub(crate) drm_device: Option<String>,
    pub(crate) abi: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DisplayProbe {
    pub(crate) name: String,
    pub(crate) resolution: String,
    pub(crate) refresh_hz: f64,
    pub(crate) primary: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FramePacingProbe {
    pub(crate) source: Option<&'static str>,
    pub(crate) target_hz: Option<f64>,
    pub(crate) presentation_feedback: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FindingSeverity {
    Info,
    Warning,
    Blocking,
}

impl FindingSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Blocking => "blocking",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphicsFinding {
    pub(crate) severity: FindingSeverity,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GraphicsReport {
    pub(crate) host: HostRenderer,
    pub(crate) drm_devices: Vec<DrmDevice>,
    pub(crate) android: AndroidGraphics,
    pub(crate) display: Option<DisplayProbe>,
    pub(crate) frame_pacing: FramePacingProbe,
    pub(crate) desktop: Option<String>,
    pub(crate) session_type: Option<String>,
    pub(crate) findings: Vec<GraphicsFinding>,
}

impl GraphicsReport {
    pub(crate) fn probe() -> Self {
        let host = probe_host_renderer();
        let drm_devices = probe_drm_devices(Path::new("/sys/class/drm"));
        let android = probe_android_graphics();
        let xrandr = command_output("xrandr", &["--current"]);
        let display = xrandr.as_deref().and_then(parse_xrandr_active_display);
        let desktop = nonempty_environment("XDG_CURRENT_DESKTOP")
            .or_else(|| nonempty_environment("XDG_SESSION_DESKTOP"));
        let session_type = nonempty_environment("XDG_SESSION_TYPE");
        let frame_pacing = probe_frame_pacing(&android, xrandr.as_deref(), session_type.as_deref());
        let findings = classify_findings(
            &host,
            &drm_devices,
            &android,
            display.as_ref(),
            &frame_pacing,
            session_type.as_deref(),
        );

        Self {
            host,
            drm_devices,
            android,
            display,
            frame_pacing,
            desktop,
            session_type,
            findings,
        }
    }

    pub(crate) fn has_blockers(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Blocking)
    }

    pub(crate) fn health(&self) -> &'static str {
        if self.has_blockers() {
            "blocked"
        } else if self
            .findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Warning)
        {
            "warning"
        } else {
            "ready"
        }
    }

    pub(crate) fn ensure_launch_ready(&self) -> Result<()> {
        let blockers = self
            .findings
            .iter()
            .filter(|finding| finding.severity == FindingSeverity::Blocking)
            .map(|finding| format!("{}: {}", finding.code, finding.message))
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            Ok(())
        } else {
            bail!(
                "performance preflight blocked the game session:\n  - {}",
                blockers.join("\n  - ")
            )
        }
    }

    pub(crate) fn as_json(&self) -> Value {
        let recommended_drm_device = self.recommended_drm_device();
        let gpu_setup_needed = self.needs_gpu_setup();
        json!({
            "health": self.health(),
            "host": {
                "kind": renderer_kind_name(self.host.kind),
                "renderer": self.host.renderer,
                "vendor": self.host.vendor,
                "version": self.host.version,
                "direct": self.host.direct,
                "accelerated": self.host.accelerated,
                "source": self.host.source,
            },
            "drmDevices": self.drm_devices.iter().map(|device| json!({
                "card": device.card,
                "renderNode": device.render_node,
                "driver": device.driver,
                "vendor": device.vendor,
                "deviceId": device.device_id,
            })).collect::<Vec<_>>(),
            "android": {
                "egl": self.android.egl,
                "gralloc": self.android.gralloc,
                "vulkan": self.android.vulkan,
                "drmDevice": self.android.drm_device,
                "abi": self.android.abi,
            },
            "gpuSetup": {
                "needed": gpu_setup_needed,
                "recommendedDevice": recommended_drm_device,
                "label": "Use active GPU",
                "detail": gpu_setup_needed.then(|| {
                    let device = recommended_drm_device.expect("GPU setup requires a recommendation");
                    format!("Configure Waydroid to use {device}, matching the active host renderer")
                }),
            },
            "display": self.display.as_ref().map(|display| json!({
                "name": display.name,
                "resolution": display.resolution,
                "refreshHz": display.refresh_hz,
                "primary": display.primary,
            })),
            "framePacing": {
                "source": self.frame_pacing.source,
                "targetHz": self.frame_pacing.target_hz,
                "presentationFeedback": self.frame_pacing.presentation_feedback,
            },
            "desktop": self.desktop,
            "sessionType": self.session_type,
            "findings": self.findings.iter().map(|finding| json!({
                "severity": finding.severity.as_str(),
                "code": finding.code,
                "message": finding.message,
            })).collect::<Vec<_>>(),
        })
    }

    fn text(&self) -> String {
        let mut output = String::new();
        output.push_str(&format!(
            "Performance preflight: {}\n",
            self.health().to_ascii_uppercase()
        ));
        output.push_str(&format!(
            "Host renderer: {} ({})\n",
            self.host.renderer.as_deref().unwrap_or("unknown"),
            renderer_kind_name(self.host.kind)
        ));
        output.push_str(&format!(
            "Graphics vendor: {}\n",
            self.host.vendor.as_deref().unwrap_or("unknown")
        ));
        let drivers = self
            .drm_devices
            .iter()
            .filter_map(|device| device.driver.as_deref())
            .collect::<BTreeSet<_>>();
        output.push_str(&format!(
            "DRM drivers: {}\n",
            if drivers.is_empty() {
                "unknown".to_owned()
            } else {
                drivers.into_iter().collect::<Vec<_>>().join(", ")
            }
        ));
        output.push_str(&format!(
            "Android graphics: EGL={} gralloc={} Vulkan={} device={} ABI={}\n",
            self.android.egl.as_deref().unwrap_or("unknown"),
            self.android.gralloc.as_deref().unwrap_or("unknown"),
            self.android.vulkan.as_deref().unwrap_or("unknown"),
            self.android.drm_device.as_deref().unwrap_or("unknown"),
            self.android.abi.as_deref().unwrap_or("unknown")
        ));
        if let Some(display) = &self.display {
            output.push_str(&format!(
                "Active display: {} {} @ {:.2} Hz\n",
                display.name, display.resolution, display.refresh_hz
            ));
        } else {
            output.push_str("Active display: unknown\n");
        }
        if let Some(source) = self.frame_pacing.source {
            let target = self
                .frame_pacing
                .target_hz
                .map(|refresh| format!(" @ {refresh:.2} Hz"))
                .unwrap_or_default();
            let feedback = match self.frame_pacing.presentation_feedback {
                Some(true) => "presentation feedback on",
                Some(false) => "presentation feedback off",
                None => "presentation feedback unknown",
            };
            output.push_str(&format!("Frame pacing: {source}{target} ({feedback})\n"));
        } else {
            output.push_str("Frame pacing: unknown while Waydroid is stopped\n");
        }
        output.push_str(&format!(
            "Desktop: {} / {}\n",
            self.desktop.as_deref().unwrap_or("unknown"),
            self.session_type.as_deref().unwrap_or("unknown")
        ));
        for finding in &self.findings {
            output.push_str(&format!(
                "[{}] {}: {}\n",
                finding.severity.as_str().to_ascii_uppercase(),
                finding.code,
                finding.message
            ));
        }
        output
    }

    pub(crate) fn recommended_drm_device(&self) -> Option<&str> {
        recommended_drm_device(&self.host, &self.drm_devices)
    }

    pub(crate) fn needs_gpu_setup(&self) -> bool {
        let Some(recommended) = self.recommended_drm_device() else {
            return false;
        };
        self.android
            .drm_device
            .as_deref()
            .is_some_and(|current| current != recommended)
            && self.drm_devices.len() > 1
    }
}

pub(crate) fn print_report(json_output: bool, setup_gpu: bool) -> Result<()> {
    if setup_gpu {
        println!("{}", open_gpu_setup()?);
        return Ok(());
    }
    let report = GraphicsReport::probe();
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report.as_json())?);
    } else {
        print!("{}", report.text());
    }
    Ok(())
}

pub(crate) fn open_gpu_setup() -> Result<String> {
    let report = GraphicsReport::probe();
    if !report.needs_gpu_setup() {
        bail!(
            "Waydroid already matches the active host GPU, or no safe recommendation is available"
        );
    }
    let device = report
        .recommended_drm_device()
        .context("active GPU render node could not be determined")?;
    let executable = env::current_exe().context("failed to locate the running Wroid executable")?;
    let command = [
        executable.into_os_string(),
        OsString::from("setup-waydroid-gpu"),
        OsString::from("--device"),
        OsString::from(device),
    ];
    let terminal = spawn_terminal(&command)?;
    Ok(format!(
        "Opened {terminal} to switch Waydroid to the active GPU ({device})"
    ))
}

pub(crate) fn setup_gpu_interactive(device: &Path) -> Result<()> {
    let result = setup_gpu_interactive_inner(device);
    if result.is_err() {
        pause_after_gpu_setup_failure();
    }
    result
}

fn setup_gpu_interactive_inner(device: &Path) -> Result<()> {
    let report = GraphicsReport::probe();
    if report.recommended_drm_device().map(Path::new) != Some(device) {
        bail!(
            "{} is not the render node recommended for the active host renderer",
            device.display()
        );
    }
    println!("Wroid GPU alignment");
    println!(
        "Waydroid: {} → {}",
        report.android.drm_device.as_deref().unwrap_or("automatic"),
        device.display()
    );
    let executable = env::current_exe().context("failed to locate the running Wroid executable")?;
    let restore_desktop = waydroid_session_running()?;
    if restore_desktop {
        println!("Stopping desktop Waydroid before its generated GPU config is replaced...");
        stop_desktop_waydroid_session()?;
    }
    println!("Enter your sudo password to apply the Waydroid configuration atomically.");
    io::stdout().flush().ok();
    let setup_result = Command::new("sudo")
        .arg(executable)
        .arg("configure-waydroid-gpu")
        .arg("--device")
        .arg(device)
        .status()
        .context("failed to start sudo for GPU setup")
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                bail!("Waydroid GPU setup exited with {status}")
            }
        });
    let restore_result = if restore_desktop {
        println!("Restoring desktop Waydroid...");
        start_desktop_waydroid_session()
    } else {
        Ok(())
    };

    if let Err(error) = setup_result {
        return match restore_result {
            Ok(()) => Err(error),
            Err(restore_error) => Err(anyhow::anyhow!(
                "{error:#}\nDesktop Waydroid restore also failed: {restore_error:#}"
            )),
        };
    }
    restore_result?;
    if restore_desktop {
        wait_for_waydroid_gpu(device)?;
    }
    println!("Waydroid now uses {}.", device.display());
    Ok(())
}

pub(crate) fn configure_waydroid_gpu(device: &Path) -> Result<()> {
    ensure_root("Waydroid GPU configuration")?;
    wroid_inject::ensure_container_stopped()
        .context("Waydroid must be stopped before changing its generated GPU configuration")?;
    let devices = probe_drm_devices(Path::new("/sys/class/drm"));
    let selected = devices
        .iter()
        .find(|candidate| candidate.render_node.as_deref().map(Path::new) == Some(device))
        .with_context(|| format!("{} is not a discovered DRM render node", device.display()))?;
    if matches!(selected.driver.as_deref(), Some("nvidia") | None) {
        bail!(
            "{} does not use a supported Mesa DRM driver",
            device.display()
        );
    }

    let config_path = Path::new(WAYDROID_CONFIG);
    let original = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let updated = set_ini_value(
        &original,
        "waydroid",
        "drm_device",
        &device.display().to_string(),
    )?;
    let changed = updated != original;

    let backup_path = PathBuf::from(format!("{WAYDROID_CONFIG}.wroid-backup"));
    if !backup_path.exists() {
        fs::copy(config_path, &backup_path)
            .with_context(|| format!("failed to create {}", backup_path.display()))?;
    }
    let mode = fs::metadata(config_path)?.permissions().mode() & 0o7777;
    if changed {
        write_atomic(config_path, &updated, mode)?;
    }

    let status = Command::new("waydroid")
        .args(["upgrade", "-o"])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to apply the Waydroid GPU configuration")?;
    if !status.success() {
        if changed {
            write_atomic(config_path, &original, mode)
                .context("Waydroid GPU setup failed and config rollback also failed")?;
        }
        let _ = Command::new("waydroid")
            .args(["upgrade", "-o"])
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status();
        bail!("waydroid upgrade -o failed with {status}; configuration was rolled back");
    }

    println!(
        "Configured drm_device={} (backup: {}).",
        device.display(),
        backup_path.display()
    );
    Ok(())
}

fn waydroid_session_running() -> Result<bool> {
    let status = wroid_waydroid::status().context("failed to inspect the Waydroid session")?;
    Ok(session_is_running(&status))
}

fn session_is_running(status: &str) -> bool {
    status.lines().any(|line| {
        line.split_once(':')
            .is_some_and(|(name, value)| name.trim() == "Session" && value.trim() == "RUNNING")
    })
}

fn stop_desktop_waydroid_session() -> Result<()> {
    let status = Command::new("waydroid")
        .args(["session", "stop"])
        .stdin(Stdio::null())
        .status()
        .context("failed to stop desktop Waydroid for GPU setup")?;
    if !status.success() {
        bail!("waydroid session stop exited with {status}");
    }
    Ok(())
}

fn start_desktop_waydroid_session() -> Result<()> {
    let mut command = Command::new("waydroid");
    command
        .args(["session", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.process_group(0);
    let mut child = command
        .spawn()
        .context("failed to restore desktop Waydroid after GPU setup")?;

    let mut last_status = String::new();
    for _ in 0..GPU_SESSION_ATTEMPTS {
        match wroid_waydroid::status() {
            Ok(status) => {
                last_status = status;
                if session_is_running(&last_status) {
                    return Ok(());
                }
            }
            Err(error) => last_status = error.to_string(),
        }
        if let Some(status) = child
            .try_wait()
            .context("failed to monitor desktop Waydroid restoration")?
        {
            bail!("waydroid session start exited with {status}\n{last_status}");
        }
        thread::sleep(GPU_SESSION_INTERVAL);
    }
    bail!("desktop Waydroid did not return to RUNNING state\n{last_status}")
}

fn wait_for_waydroid_gpu(device: &Path) -> Result<()> {
    let expected = device.display().to_string();
    let mut current = None;
    for _ in 0..GPU_SESSION_ATTEMPTS {
        current = waydroid_property("gralloc.gbm.device");
        if current.as_deref() == Some(expected.as_str()) {
            return Ok(());
        }
        thread::sleep(GPU_SESSION_INTERVAL);
    }
    bail!(
        "Waydroid restarted, but gralloc.gbm.device is {}; expected {expected}",
        current.as_deref().unwrap_or("unavailable")
    )
}

fn ensure_root(operation: &str) -> Result<()> {
    let status = fs::read_to_string("/proc/self/status")?;
    let effective_uid = status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))
        .and_then(|uids| uids.split_whitespace().nth(1))
        .and_then(|uid| uid.parse::<u32>().ok())
        .context("could not read the effective uid")?;
    if effective_uid != 0 {
        bail!("{operation} requires root; run it through wroid performance --setup-gpu");
    }
    Ok(())
}

fn set_ini_value(content: &str, section: &str, key: &str, value: &str) -> Result<String> {
    let mut lines = content.lines().map(str::to_owned).collect::<Vec<_>>();
    let header = format!("[{section}]");
    let start = lines
        .iter()
        .position(|line| line.trim() == header)
        .with_context(|| format!("missing {header} in Waydroid configuration"))?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| {
            let line = line.trim();
            line.starts_with('[') && line.ends_with(']')
        })
        .map(|offset| start + 1 + offset)
        .unwrap_or(lines.len());
    if let Some(index) = (start + 1..end).find(|index| {
        lines[*index]
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == key)
    }) {
        lines[index] = format!("{key} = {value}");
    } else {
        let insert_at = (start + 1..end)
            .rev()
            .find(|index| !lines[*index].trim().is_empty())
            .map(|index| index + 1)
            .unwrap_or(start + 1);
        lines.insert(insert_at, format!("{key} = {value}"));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn write_atomic(path: &Path, content: &str, mode: u32) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("Waydroid config filename is not valid UTF-8")?;
    let temporary = path.with_file_name(format!(".{file_name}.wroid-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn pause_after_gpu_setup_failure() {
    if !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return;
    }
    eprint!("GPU setup did not finish. Press Enter to close this terminal… ");
    io::stderr().flush().ok();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}

fn probe_host_renderer() -> HostRenderer {
    if let Some(output) = command_output("glxinfo", &["-B"]) {
        return parse_renderer_output(&output, "glxinfo");
    }
    if let Some(output) = command_output("eglinfo", &["-B"]) {
        return parse_renderer_output(&output, "eglinfo");
    }
    HostRenderer {
        kind: RendererKind::Unknown,
        renderer: None,
        vendor: None,
        version: None,
        direct: None,
        accelerated: None,
        source: None,
    }
}

fn parse_renderer_output(output: &str, source: &str) -> HostRenderer {
    let renderer = field_value(
        output,
        &[
            "OpenGL renderer string:",
            "OpenGL core profile renderer:",
            "OpenGL ES profile renderer:",
        ],
    );
    let vendor = field_value(
        output,
        &[
            "OpenGL vendor string:",
            "OpenGL core profile vendor:",
            "Vendor:",
        ],
    );
    let version = field_value(
        output,
        &[
            "OpenGL core profile version string:",
            "OpenGL version string:",
            "OpenGL core profile version:",
        ],
    );
    let direct = field_value(output, &["direct rendering:"])
        .as_deref()
        .and_then(parse_yes_no);
    let accelerated = field_value(output, &["Accelerated:"])
        .as_deref()
        .and_then(parse_yes_no);
    let software = renderer.as_deref().is_some_and(contains_software_renderer);
    let kind = if software {
        RendererKind::Software
    } else if renderer.is_some()
        && !matches!(direct, Some(false))
        && !matches!(accelerated, Some(false))
    {
        RendererKind::Hardware
    } else {
        RendererKind::Unknown
    };

    HostRenderer {
        kind,
        renderer,
        vendor,
        version,
        direct,
        accelerated,
        source: Some(source.to_owned()),
    }
}

fn field_value(output: &str, labels: &[&str]) -> Option<String> {
    labels.iter().find_map(|label| {
        output.lines().find_map(|line| {
            line.trim()
                .strip_prefix(label)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
    })
}

fn parse_yes_no(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}

fn contains_software_renderer(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    [
        "llvmpipe",
        "softpipe",
        "swrast",
        "software rasterizer",
        "lavapipe",
        "swiftshader",
    ]
    .iter()
    .any(|marker| value.contains(marker))
}

fn probe_drm_devices(root: &Path) -> Vec<DrmDevice> {
    let mut devices = fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let card = entry.file_name().to_str()?.to_owned();
            let suffix = card.strip_prefix("card")?;
            if suffix.is_empty() || !suffix.chars().all(|character| character.is_ascii_digit()) {
                return None;
            }
            let device = entry.path().join("device");
            Some(DrmDevice {
                card,
                render_node: render_node_for_device(&device),
                driver: canonical_file_name(&device.join("driver")),
                vendor: read_trimmed(&device.join("vendor")).map(|id| vendor_name(&id)),
                device_id: read_trimmed(&device.join("device")),
            })
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| left.card.cmp(&right.card));
    devices
}

fn render_node_for_device(device: &Path) -> Option<String> {
    let mut nodes = fs::read_dir(device.join("drm"))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let suffix = name.strip_prefix("renderD")?;
            (!suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()))
                .then_some(name)
        })
        .collect::<Vec<_>>();
    nodes.sort();
    nodes.first().map(|node| format!("/dev/dri/{node}"))
}

fn canonical_file_name(path: &Path) -> Option<String> {
    path.canonicalize()
        .ok()?
        .file_name()?
        .to_str()
        .map(str::to_owned)
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn vendor_name(id: &str) -> String {
    match id.trim().to_ascii_lowercase().as_str() {
        "0x1002" => "AMD".to_owned(),
        "0x10de" => "NVIDIA".to_owned(),
        "0x8086" => "Intel".to_owned(),
        _ => id.to_owned(),
    }
}

fn probe_android_graphics() -> AndroidGraphics {
    AndroidGraphics {
        egl: waydroid_property("ro.hardware.egl"),
        gralloc: waydroid_property("ro.hardware.gralloc"),
        vulkan: waydroid_property("ro.hardware.vulkan"),
        drm_device: waydroid_property("gralloc.gbm.device"),
        abi: waydroid_property("ro.product.cpu.abi"),
    }
}

fn probe_frame_pacing(
    android: &AndroidGraphics,
    xrandr: Option<&str>,
    session_type: Option<&str>,
) -> FramePacingProbe {
    let android_online = [
        android.egl.as_ref(),
        android.gralloc.as_ref(),
        android.vulkan.as_ref(),
        android.drm_device.as_ref(),
        android.abi.as_ref(),
    ]
    .into_iter()
    .any(|value| value.is_some());
    if !android_online || !session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland")) {
        return FramePacingProbe::default();
    }

    let presentation_feedback = match waydroid_property("persist.waydroid.no_presentation")
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("true" | "1") => Some(false),
        Some("false" | "0") | None => Some(true),
        Some(_) => None,
    };
    FramePacingProbe {
        source: Some("Wayland compositor"),
        target_hz: xrandr.and_then(parse_xrandr_max_active_refresh),
        presentation_feedback,
    }
}

fn waydroid_property(name: &str) -> Option<String> {
    let output = Command::new("waydroid")
        .args(["prop", "get", name])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_waydroid_property(&output.stdout)
}

fn parse_waydroid_property(stdout: &[u8]) -> Option<String> {
    let value = String::from_utf8_lossy(stdout).trim().to_owned();
    if value.is_empty()
        || value.lines().count() != 1
        || value.starts_with('[')
        || value.to_ascii_lowercase().contains("waydroid session")
    {
        None
    } else {
        Some(value)
    }
}

fn parse_xrandr_active_display(output: &str) -> Option<DisplayProbe> {
    let mut current: Option<(String, bool)> = None;
    let mut displays = Vec::new();
    for line in output.lines() {
        if !line.starts_with(char::is_whitespace) && line.contains(" connected") {
            let name = line.split_whitespace().next()?.to_owned();
            current = Some((name, line.contains(" connected primary ")));
            continue;
        }
        let Some((name, primary)) = current.as_ref() else {
            continue;
        };
        let mut parts = line.split_whitespace();
        let Some(resolution) = parts.next() else {
            continue;
        };
        let Some(refresh) = parts.find(|part| part.contains('*')) else {
            continue;
        };
        let refresh_hz = refresh
            .trim_matches(|character: char| character == '*' || character == '+')
            .parse::<f64>()
            .ok()?;
        if !resolution.contains('x') {
            continue;
        }
        displays.push(DisplayProbe {
            name: name.clone(),
            resolution: resolution.to_owned(),
            refresh_hz,
            primary: *primary,
        });
    }
    displays
        .iter()
        .find(|display| display.primary)
        .cloned()
        .or_else(|| displays.into_iter().next())
}

fn parse_xrandr_max_active_refresh(output: &str) -> Option<f64> {
    output
        .lines()
        .filter_map(|line| {
            let refresh = line.split_whitespace().find(|part| part.contains('*'))?;
            refresh
                .trim_matches(|character: char| character == '*' || character == '+')
                .parse::<f64>()
                .ok()
        })
        .max_by(f64::total_cmp)
}

fn classify_findings(
    host: &HostRenderer,
    drm_devices: &[DrmDevice],
    android: &AndroidGraphics,
    display: Option<&DisplayProbe>,
    frame_pacing: &FramePacingProbe,
    session_type: Option<&str>,
) -> Vec<GraphicsFinding> {
    let mut findings = Vec::new();
    match host.kind {
        RendererKind::Software => findings.push(GraphicsFinding {
            severity: FindingSeverity::Blocking,
            code: "software-renderer",
            message: format!(
                "{} is a CPU software renderer; install/fix the GPU driver before gaming",
                host.renderer.as_deref().unwrap_or("Detected renderer")
            ),
        }),
        RendererKind::Hardware => findings.push(GraphicsFinding {
            severity: FindingSeverity::Info,
            code: "hardware-renderer",
            message: format!(
                "{} is hardware accelerated",
                host.renderer.as_deref().unwrap_or("The OpenGL renderer")
            ),
        }),
        RendererKind::Unknown => findings.push(GraphicsFinding {
            severity: FindingSeverity::Warning,
            code: "renderer-unknown",
            message: "Could not identify the active renderer; install glxinfo or eglinfo"
                .to_owned(),
        }),
    }
    if matches!(host.direct, Some(false)) || matches!(host.accelerated, Some(false)) {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Blocking,
            code: "graphics-not-accelerated",
            message: "The active OpenGL context is not directly hardware accelerated".to_owned(),
        });
    }
    if drm_devices.is_empty() {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Warning,
            code: "drm-devices-missing",
            message: "No DRM GPU devices were discovered under /sys/class/drm".to_owned(),
        });
    }
    let android_values = [
        android.egl.as_deref(),
        android.gralloc.as_deref(),
        android.vulkan.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if android_values
        .iter()
        .any(|value| contains_software_renderer(value))
    {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Blocking,
            code: "android-software-renderer",
            message: format!(
                "Waydroid reports a software graphics component: {}",
                android_values.join(", ")
            ),
        });
    } else if android_values.is_empty() {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Warning,
            code: "android-graphics-unknown",
            message: "Waydroid EGL/Vulkan properties could not be read".to_owned(),
        });
    } else {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Info,
            code: "android-graphics-ready",
            message: format!(
                "Waydroid graphics components: {}",
                android_values.join(", ")
            ),
        });
    }
    if let Some(mismatch) = gpu_mismatch_finding(host, drm_devices, android) {
        findings.push(mismatch);
    }
    match frame_pacing.presentation_feedback {
        Some(true) => findings.push(GraphicsFinding {
            severity: FindingSeverity::Info,
            code: "waydroid-frame-pacing",
            message: frame_pacing
                .target_hz
                .map(|refresh| {
                    format!(
                        "Waydroid follows the Wayland compositor at up to {refresh:.2} Hz with presentation feedback"
                    )
                })
                .unwrap_or_else(|| {
                    "Waydroid uses Wayland compositor timing with presentation feedback".to_owned()
                }),
        }),
        Some(false) => findings.push(GraphicsFinding {
            severity: FindingSeverity::Warning,
            code: "waydroid-presentation-disabled",
            message: "Waydroid presentation feedback is disabled; unset persist.waydroid.no_presentation for accurate frame pacing".to_owned(),
        }),
        None => {}
    }
    if let Some(display) = display {
        findings.push(GraphicsFinding {
            severity: if display.refresh_hz < 59.0 {
                FindingSeverity::Warning
            } else {
                FindingSeverity::Info
            },
            code: "display-refresh",
            message: format!(
                "{} runs at {:.2} Hz on {}",
                display.resolution, display.refresh_hz, display.name
            ),
        });
    } else {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Warning,
            code: "display-refresh-unknown",
            message: "Could not detect active display refresh rate".to_owned(),
        });
    }
    if session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland")) {
        findings.push(GraphicsFinding {
            severity: FindingSeverity::Info,
            code: "wayland-session",
            message: "Wayland desktop session detected".to_owned(),
        });
    }
    findings
}

fn recommended_drm_device<'a>(
    host: &HostRenderer,
    drm_devices: &'a [DrmDevice],
) -> Option<&'a str> {
    let host_vendor = host.vendor.as_deref().or_else(|| {
        host.renderer.as_deref().and_then(|renderer| {
            let renderer = renderer.to_ascii_lowercase();
            if renderer.contains("amd") || renderer.contains("radeon") {
                Some("AMD")
            } else if renderer.contains("intel") {
                Some("Intel")
            } else {
                None
            }
        })
    })?;
    drm_devices
        .iter()
        .find(|device| {
            device
                .vendor
                .as_deref()
                .is_some_and(|vendor| vendor.eq_ignore_ascii_case(host_vendor))
                && !matches!(device.driver.as_deref(), Some("nvidia"))
        })
        .and_then(|device| device.render_node.as_deref())
}

fn gpu_mismatch_finding(
    host: &HostRenderer,
    drm_devices: &[DrmDevice],
    android: &AndroidGraphics,
) -> Option<GraphicsFinding> {
    let recommended = recommended_drm_device(host, drm_devices)?;
    let current = android.drm_device.as_deref()?;
    if current == recommended || drm_devices.len() < 2 {
        return None;
    }
    let current_vendor = drm_devices
        .iter()
        .find(|device| device.render_node.as_deref() == Some(current))
        .and_then(|device| device.vendor.as_deref())
        .unwrap_or("another GPU");
    Some(GraphicsFinding {
        severity: FindingSeverity::Warning,
        code: "waydroid-gpu-mismatch",
        message: format!(
            "Waydroid uses {current_vendor} {current}, while the active host renderer matches {recommended}"
        ),
    })
}

fn renderer_kind_name(kind: RendererKind) -> &'static str {
    match kind {
        RendererKind::Hardware => "hardware",
        RendererKind::Software => "software",
        RendererKind::Unknown => "unknown",
    }
}

fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Some(text)
}

fn nonempty_environment(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HARDWARE_GLX: &str = r#"
direct rendering: Yes
    Vendor: AMD (0x1002)
    Accelerated: yes
OpenGL vendor string: AMD
OpenGL renderer string: AMD Radeon RX 6600 XT (radeonsi, navi23)
OpenGL core profile version string: 4.6 Mesa 26.1.5
"#;

    const SOFTWARE_GLX: &str = r#"
direct rendering: Yes
    Accelerated: no
OpenGL vendor string: Mesa/X.org
OpenGL renderer string: llvmpipe (LLVM 19.1.7, 256 bits)
OpenGL version string: 4.5 Mesa
"#;

    #[test]
    fn parses_hardware_renderer_details() {
        let renderer = parse_renderer_output(HARDWARE_GLX, "fixture");
        assert_eq!(renderer.kind, RendererKind::Hardware);
        assert_eq!(renderer.vendor.as_deref(), Some("AMD"));
        assert_eq!(
            renderer.renderer.as_deref(),
            Some("AMD Radeon RX 6600 XT (radeonsi, navi23)")
        );
        assert_eq!(renderer.direct, Some(true));
        assert_eq!(renderer.accelerated, Some(true));
    }

    #[test]
    fn software_renderer_creates_a_launch_blocker() {
        let host = parse_renderer_output(SOFTWARE_GLX, "fixture");
        let report = GraphicsReport {
            findings: classify_findings(
                &host,
                &[DrmDevice {
                    card: "card0".to_owned(),
                    render_node: Some("/dev/dri/renderD128".to_owned()),
                    driver: Some("amdgpu".to_owned()),
                    vendor: Some("AMD".to_owned()),
                    device_id: None,
                }],
                &AndroidGraphics {
                    egl: Some("mesa".to_owned()),
                    ..AndroidGraphics::default()
                },
                None,
                &FramePacingProbe::default(),
                Some("wayland"),
            ),
            host,
            drm_devices: Vec::new(),
            android: AndroidGraphics::default(),
            display: None,
            frame_pacing: FramePacingProbe::default(),
            desktop: None,
            session_type: None,
        };
        assert!(report.has_blockers());
        assert!(report.ensure_launch_ready().is_err());
    }

    #[test]
    fn parses_primary_xrandr_mode_and_refresh() {
        let output = r#"
Screen 0: current 3840 x 1080
DP-2 connected primary 1920x1080+0+0
   1920x1080    239.66*+
HDMI-A-4 connected 1920x1080+1920+0
   1920x1080     59.96*+
"#;
        let display = parse_xrandr_active_display(output).unwrap();
        assert_eq!(display.name, "DP-2");
        assert_eq!(display.resolution, "1920x1080");
        assert!((display.refresh_hz - 239.66).abs() < f64::EPSILON);
        assert!(display.primary);
        assert_eq!(parse_xrandr_max_active_refresh(output), Some(239.66));
    }

    #[test]
    fn disabled_presentation_feedback_is_a_frame_pacing_warning() {
        let host = parse_renderer_output(HARDWARE_GLX, "fixture");
        let frame_pacing = FramePacingProbe {
            source: Some("Wayland compositor"),
            target_hz: Some(239.66),
            presentation_feedback: Some(false),
        };
        let findings = classify_findings(
            &host,
            &[],
            &AndroidGraphics {
                egl: Some("mesa".to_owned()),
                ..AndroidGraphics::default()
            },
            None,
            &frame_pacing,
            Some("wayland"),
        );
        let finding = findings
            .iter()
            .find(|finding| finding.code == "waydroid-presentation-disabled")
            .unwrap();
        assert_eq!(finding.severity, FindingSeverity::Warning);
    }

    #[test]
    fn gpu_setup_lifecycle_uses_session_state_not_container_state() {
        assert!(session_is_running(
            "Session:\tRUNNING\nContainer:\tFROZEN\n"
        ));
        assert!(!session_is_running(
            "Session:\tSTOPPED\nContainer:\tRUNNING\n"
        ));
    }

    #[test]
    fn software_markers_cover_common_opengl_and_vulkan_fallbacks() {
        for renderer in [
            "llvmpipe",
            "softpipe",
            "Mesa software rasterizer",
            "lavapipe",
            "SwiftShader Device",
        ] {
            assert!(contains_software_renderer(renderer), "{renderer}");
        }
        assert!(!contains_software_renderer("AMD Radeon RX 6600 XT"));
    }

    #[test]
    fn dual_gpu_mismatch_recommends_the_active_renderer_device() {
        let host = parse_renderer_output(HARDWARE_GLX, "fixture");
        let devices = vec![
            DrmDevice {
                card: "card0".to_owned(),
                render_node: Some("/dev/dri/renderD128".to_owned()),
                driver: Some("i915".to_owned()),
                vendor: Some("Intel".to_owned()),
                device_id: None,
            },
            DrmDevice {
                card: "card1".to_owned(),
                render_node: Some("/dev/dri/renderD129".to_owned()),
                driver: Some("amdgpu".to_owned()),
                vendor: Some("AMD".to_owned()),
                device_id: None,
            },
        ];
        let android = AndroidGraphics {
            egl: Some("mesa".to_owned()),
            gralloc: Some("gbm".to_owned()),
            vulkan: Some("intel".to_owned()),
            drm_device: Some("/dev/dri/renderD128".to_owned()),
            abi: Some("x86_64".to_owned()),
        };

        assert_eq!(
            recommended_drm_device(&host, &devices),
            Some("/dev/dri/renderD129")
        );
        let finding = gpu_mismatch_finding(&host, &devices, &android).unwrap();
        assert_eq!(finding.code, "waydroid-gpu-mismatch");
        assert_eq!(finding.severity, FindingSeverity::Warning);
    }

    #[test]
    fn offline_waydroid_diagnostics_are_not_graphics_properties() {
        assert_eq!(parse_waydroid_property(b""), None);
        assert_eq!(
            parse_waydroid_property(b"[10:35:29] WayDroid session is stopped\n"),
            None
        );
        assert_eq!(parse_waydroid_property(b"mesa\n"), Some("mesa".to_owned()));
    }

    #[test]
    fn unknown_waydroid_gpu_does_not_offer_configuration() {
        let host = parse_renderer_output(HARDWARE_GLX, "fixture");
        let devices = vec![
            DrmDevice {
                card: "card0".to_owned(),
                render_node: Some("/dev/dri/renderD128".to_owned()),
                driver: Some("i915".to_owned()),
                vendor: Some("Intel".to_owned()),
                device_id: None,
            },
            DrmDevice {
                card: "card1".to_owned(),
                render_node: Some("/dev/dri/renderD129".to_owned()),
                driver: Some("amdgpu".to_owned()),
                vendor: Some("AMD".to_owned()),
                device_id: None,
            },
        ];
        let report = GraphicsReport {
            findings: classify_findings(
                &host,
                &devices,
                &AndroidGraphics::default(),
                None,
                &FramePacingProbe::default(),
                Some("wayland"),
            ),
            host,
            drm_devices: devices,
            android: AndroidGraphics::default(),
            display: None,
            frame_pacing: FramePacingProbe::default(),
            desktop: None,
            session_type: Some("wayland".to_owned()),
        };

        assert!(!report.needs_gpu_setup());
        assert_eq!(report.as_json()["gpuSetup"]["detail"], Value::Null);
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.code == "waydroid-gpu-mismatch"));
    }

    #[test]
    fn ini_gpu_update_preserves_other_waydroid_settings() {
        let original = "[waydroid]\narch = x86_64\nvendor_type = MAINLINE\n\n[properties]\n";
        let updated =
            set_ini_value(original, "waydroid", "drm_device", "/dev/dri/renderD129").unwrap();
        assert!(updated.contains("arch = x86_64"));
        assert!(updated.contains("drm_device = /dev/dri/renderD129\n\n[properties]"));

        let replaced =
            set_ini_value(&updated, "waydroid", "drm_device", "/dev/dri/renderD128").unwrap();
        assert_eq!(replaced.matches("drm_device").count(), 1);
        assert!(replaced.contains("drm_device = /dev/dri/renderD128"));
    }
}
