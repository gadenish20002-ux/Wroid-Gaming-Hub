//! Backend-independent runtime service contracts.
//!
//! These traits describe the boundaries that the production daemon, CLI, GUI,
//! Android adapters, and privileged helper share. They intentionally do not
//! expose ADB commands, Waydroid shell strings, GUI types, or root-only details.

use std::error::Error as StdError;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;
use wroid_core::{ControlProfile, Resolution};

/// Common error bound used by service traits.
pub trait ServiceError: StdError + Send + Sync + 'static {}

impl<T> ServiceError for T where T: StdError + Send + Sync + 'static {}

/// Android package format identified before dispatching an install operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageFormat {
    Apk,
    SplitApkBundle,
    Xapk,
    Apkm,
    Obb,
    Unknown,
}

/// Android package visibility category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageSource {
    User,
    System,
    Unknown,
}

/// Package discovered in the Android environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidPackage {
    pub package_name: String,
    pub label: Option<String>,
    pub source: PackageSource,
}

impl AndroidPackage {
    pub fn new(package_name: impl Into<String>) -> Self {
        Self {
            package_name: package_name.into(),
            label: None,
            source: PackageSource::Unknown,
        }
    }
}

/// Currently focused Android activity, when it can be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidActivity {
    pub package_name: String,
    pub activity_name: Option<String>,
}

/// Install request after the caller has already resolved the local artifact path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    pub path: PathBuf,
    pub format: PackageFormat,
}

impl InstallRequest {
    pub fn apk(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            format: PackageFormat::Apk,
        }
    }
}

/// Install result normalized across ADB, Waydroid, and future bundle installers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstallReport {
    pub package_name: Option<String>,
    pub warnings: Vec<String>,
}

/// Android package operations owned by an unprivileged adapter.
pub trait AndroidPackageService {
    type Error: ServiceError;

    fn list_packages(&mut self) -> Result<Vec<AndroidPackage>, Self::Error>;
    fn launch_package(&mut self, package_name: &str) -> Result<(), Self::Error>;
    fn install(&mut self, request: &InstallRequest) -> Result<InstallReport, Self::Error>;
    fn current_activity(&mut self) -> Result<Option<AndroidActivity>, Self::Error>;
}

/// Display orientation reported by the Android environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayOrientation {
    Landscape,
    Portrait,
    Unknown,
}

/// Content viewport inside the Android surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub const fn full_surface(resolution: Resolution) -> Self {
        Self {
            x: 0,
            y: 0,
            width: resolution.width,
            height: resolution.height,
        }
    }
}

/// Display properties required for profile scaling, diagnostics, and GUI preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    pub resolution: Resolution,
    pub density_dpi: Option<u32>,
    pub refresh_rate_millihz: Option<u32>,
    pub orientation: DisplayOrientation,
    pub viewport: Viewport,
}

impl DisplayInfo {
    pub fn new(resolution: Resolution) -> Self {
        Self {
            resolution,
            density_dpi: None,
            refresh_rate_millihz: None,
            orientation: DisplayOrientation::Unknown,
            viewport: Viewport::full_surface(resolution),
        }
    }
}

/// Display and viewport queries independent from the package backend.
pub trait DisplayService {
    type Error: ServiceError;

    fn display_info(&mut self) -> Result<DisplayInfo, Self::Error>;
}

/// Stable identifier for one managed runtime session.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SessionIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SessionIdError::Empty);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionIdError {
    #[error("session id must not be empty")]
    Empty,
}

/// Lifecycle state owned by the production runtime daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Stopped,
    Preparing,
    Running,
    Stopping,
    Failed,
}

/// Reason for stopping a runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    UserRequested,
    FocusLost,
    BackendFailed,
    ClientDisconnected,
    RuntimeShutdown,
}

/// Request accepted by the runtime daemon before a gaming session is prepared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRequest {
    pub session_id: SessionId,
    pub profile: ControlProfile,
    pub display: DisplayInfo,
    pub launch_package: bool,
}

/// Prepared session metadata returned before capture and injection are activated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSession {
    pub session_id: SessionId,
    pub state: SessionState,
    pub active_package: String,
}

/// Stop report returned after all contacts, leases, and temporary settings are cleaned up.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StopReport {
    pub contacts_cancelled: usize,
    pub leases_released: usize,
    pub settings_restored: bool,
}

/// Managed session lifecycle. The CLI and GUI should call this instead of owning runtime state.
pub trait SessionLifecycle {
    type Error: ServiceError;

    fn prepare(&mut self, request: SessionRequest) -> Result<PreparedSession, Self::Error>;
    fn start(&mut self, session_id: &SessionId) -> Result<(), Self::Error>;
    fn stop(&mut self, session_id: &SessionId, reason: StopReason) -> Result<StopReport, Self::Error>;
    fn state(&self, session_id: &SessionId) -> Result<SessionState, Self::Error>;
}

/// Host input device category exposed to the runtime and privileged helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDeviceKind {
    Keyboard,
    Mouse,
    Gamepad,
    Touchscreen,
    Unknown,
}

/// Host input device selected by the user or discovered by diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceDescriptor {
    pub path: PathBuf,
    pub name: String,
    pub kind: InputDeviceKind,
}

impl InputDeviceDescriptor {
    pub fn new(path: impl Into<PathBuf>, name: impl Into<String>, kind: InputDeviceKind) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            kind,
        }
    }
}

/// Stable identifier for an input lease held by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputLeaseId(String);

impl InputLeaseId {
    pub fn new(value: impl Into<String>) -> Result<Self, InputLeaseIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(InputLeaseIdError::Empty);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InputLeaseIdError {
    #[error("input lease id must not be empty")]
    Empty,
}

/// Request for explicit, time-bounded host input access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputLeaseRequest {
    pub session_id: SessionId,
    pub device: InputDeviceDescriptor,
    pub exclusive_grab: bool,
    pub timeout: Duration,
}

/// Active host input lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputLease {
    pub lease_id: InputLeaseId,
    pub device: InputDeviceDescriptor,
    pub exclusive_grab: bool,
    pub expires_after: Duration,
}

/// Privileged-helper-facing input access contract.
pub trait InputLeaseService {
    type Error: ServiceError;

    fn list_input_devices(&mut self) -> Result<Vec<InputDeviceDescriptor>, Self::Error>;
    fn acquire_input_lease(&mut self, request: InputLeaseRequest) -> Result<InputLease, Self::Error>;
    fn release_input_lease(&mut self, lease_id: &InputLeaseId) -> Result<(), Self::Error>;
}

/// Renderer classification used by performance diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererKind {
    Hardware,
    Software,
    Unknown,
}

/// Renderer details normalized across host and Android probes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RendererInfo {
    pub kind: RendererKind,
    pub renderer: Option<String>,
    pub vendor: Option<String>,
    pub version: Option<String>,
}

impl RendererInfo {
    pub fn unknown() -> Self {
        Self {
            kind: RendererKind::Unknown,
            renderer: None,
            vendor: None,
            version: None,
        }
    }
}

/// Diagnostic finding severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Blocking,
}

/// One diagnostic finding suitable for CLI and GUI rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticFinding {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
}

/// Runtime diagnostics used by first-run validation and per-game troubleshooting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostics {
    pub backend_name: String,
    pub renderer: RendererInfo,
    pub display: Option<DisplayInfo>,
    pub findings: Vec<DiagnosticFinding>,
}

impl RuntimeDiagnostics {
    pub fn has_blockers(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == DiagnosticSeverity::Blocking)
    }
}

/// Diagnostics contract for doctor, first-run validation, and GUI health checks.
pub trait DiagnosticsProvider {
    type Error: ServiceError;

    fn runtime_diagnostics(&mut self) -> Result<RuntimeDiagnostics, Self::Error>;
}

/// Helper for callers that still need to validate a local path before creating an install request.
pub fn install_request_for_path(path: impl AsRef<Path>) -> InstallRequest {
    let path = path.as_ref().to_path_buf();
    let format = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("apk") => PackageFormat::Apk,
        Some("xapk") => PackageFormat::Xapk,
        Some("apkm") => PackageFormat::Apkm,
        Some("obb") => PackageFormat::Obb,
        _ => PackageFormat::Unknown,
    };

    InstallRequest { path, format }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_core::Resolution;

    #[test]
    fn display_info_defaults_to_full_surface_viewport() {
        let resolution = Resolution {
            width: 1920,
            height: 1080,
        };
        let display = DisplayInfo::new(resolution);

        assert_eq!(display.resolution, resolution);
        assert_eq!(display.viewport, Viewport::full_surface(resolution));
        assert_eq!(display.orientation, DisplayOrientation::Unknown);
    }

    #[test]
    fn session_ids_reject_empty_values() {
        assert_eq!(SessionId::new("  ").unwrap_err(), SessionIdError::Empty);
        assert_eq!(SessionId::new("game-1").unwrap().as_str(), "game-1");
    }

    #[test]
    fn input_lease_ids_reject_empty_values() {
        assert_eq!(
            InputLeaseId::new("").unwrap_err(),
            InputLeaseIdError::Empty
        );
        assert_eq!(InputLeaseId::new("lease-1").unwrap().as_str(), "lease-1");
    }

    #[test]
    fn install_request_detects_common_package_formats() {
        assert_eq!(
            install_request_for_path("/tmp/game.apk").format,
            PackageFormat::Apk
        );
        assert_eq!(
            install_request_for_path("/tmp/game.XAPK").format,
            PackageFormat::Xapk
        );
        assert_eq!(
            install_request_for_path("/tmp/main.1.com.example.obb").format,
            PackageFormat::Obb
        );
        assert_eq!(
            install_request_for_path("/tmp/download.bin").format,
            PackageFormat::Unknown
        );
    }

    #[test]
    fn diagnostics_report_blocking_findings() {
        let diagnostics = RuntimeDiagnostics {
            backend_name: "test".to_owned(),
            renderer: RendererInfo::unknown(),
            display: None,
            findings: vec![DiagnosticFinding {
                severity: DiagnosticSeverity::Blocking,
                code: "software-renderer".to_owned(),
                message: "software rendering is active".to_owned(),
            }],
        };

        assert!(diagnostics.has_blockers());
    }
}
