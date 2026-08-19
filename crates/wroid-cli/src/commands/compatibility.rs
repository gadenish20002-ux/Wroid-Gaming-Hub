use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::game_catalog::{family_for_package, installed_variant, GameFamily, GAME_FAMILIES};
use super::terminal::spawn_terminal;

const WAYDROID_COMMUNITY_URL: &str = "https://docs.waydro.id/faq/community-projects-we-like";
const APP_LIST_READY_TIMEOUT: Duration = Duration::from_secs(30);
const APP_LIST_READY_INTERVAL: Duration = Duration::from_millis(250);
const WAYDROID_CONFIG: &str = "/var/lib/waydroid/waydroid.cfg";
const MAGISK_OVERLAY: &str = "/var/lib/waydroid/overlay/system/etc/init/magisk";
const MAGISK_PACKAGES: [&str; 2] = ["io.github.huskydg.magisk", "com.topjohnwu.magisk"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    Info,
    Warning,
    Action,
}

impl Severity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Action => "action",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootMarkerProbe {
    Present,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootAccessState {
    Detected,
    NotDetected,
    Unknown,
}

impl RootAccessState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::NotDetected => "not_detected",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootAccess {
    state: RootAccessState,
    evidence: Option<&'static str>,
}

impl RootAccess {
    fn detail(&self) -> &'static str {
        match (self.state, self.evidence) {
            (RootAccessState::Detected, Some("magisk_overlay")) => {
                "Active Magisk system overlay detected; remove Magisk with `sudo waydroid-extras remove magisk`, restart Waydroid, then refresh Wroid"
            }
            (RootAccessState::Detected, Some("magisk_package")) => {
                "Active Magisk package detected; remove Magisk with `sudo waydroid-extras remove magisk`, restart Waydroid, then refresh Wroid"
            }
            (RootAccessState::Detected, _) => "Active Android root access detected",
            (RootAccessState::NotDetected, _) => "No active Magisk signals detected",
            (RootAccessState::Unknown, _) => {
                "Android root state could not be fully verified; Wroid will not claim the environment is clean"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    severity: Severity,
    code: &'static str,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GameCompatibility {
    pub(crate) name: &'static str,
    pub(crate) package: &'static str,
    pub(crate) installed: Option<bool>,
    pub(crate) installed_package: Option<String>,
    pub(crate) state: &'static str,
    pub(crate) detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompatibilityReport {
    host_arch: String,
    waydroid_running: bool,
    android_version: Option<String>,
    primary_abi: Option<String>,
    abi_list: Vec<String>,
    native_bridge: Option<String>,
    arm_translation: Option<bool>,
    play_store: Option<bool>,
    installed_packages: Option<Vec<String>>,
    root_access: RootAccess,
    games: Vec<GameCompatibility>,
    findings: Vec<Finding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetupRoute {
    pub(crate) kind: &'static str,
    pub(crate) label: &'static str,
    pub(crate) detail: String,
}

impl CompatibilityReport {
    pub(crate) fn probe() -> Self {
        let running = wroid_waydroid::is_available()
            && wroid_waydroid::status()
                .is_ok_and(|status| status.contains("Session:") && status.contains("RUNNING"));
        let installed_packages = running.then(probe_ready_package_list).flatten();
        Self::probe_with(running, installed_packages.as_deref(), None)
    }

    pub(crate) fn android_abis(&self) -> Vec<String> {
        if self.abi_list.is_empty() {
            self.primary_abi.iter().cloned().collect()
        } else {
            self.abi_list.clone()
        }
    }

    pub(crate) const fn arm_translation_status(&self) -> Option<bool> {
        self.arm_translation
    }

    pub(crate) fn probe_with(
        waydroid_running: bool,
        installed_packages: Option<&[String]>,
        known_primary_abi: Option<String>,
    ) -> Self {
        let primary_abi = known_primary_abi
            .or_else(|| waydroid_property("ro.product.cpu.abi"))
            .or_else(|| configured_waydroid_property("ro.product.cpu.abi"));
        let abi_list_property = waydroid_property("ro.product.cpu.abilist")
            .or_else(|| configured_waydroid_property("ro.product.cpu.abilist"));
        let abi_list = abi_list_property
            .map(|value| {
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let native_bridge = waydroid_property("ro.dalvik.vm.native.bridge")
            .or_else(|| configured_waydroid_property("ro.dalvik.vm.native.bridge"));
        let android_version = waydroid_property("ro.build.version.release");
        let arm_translation = arm_support_status(
            std::env::consts::ARCH,
            (!abi_list.is_empty()).then_some(abi_list.as_slice()),
            native_bridge.as_deref(),
        );
        let play_store = installed_packages.map(|packages| {
            packages
                .iter()
                .any(|package| package == "com.android.vending")
        });
        Self::from_probe(ProbeData {
            host_arch: std::env::consts::ARCH.to_owned(),
            waydroid_running,
            android_version,
            primary_abi,
            abi_list,
            native_bridge,
            arm_translation,
            play_store,
            installed_packages,
            root_marker: probe_magisk_overlay(Path::new(MAGISK_OVERLAY)),
        })
    }

    fn from_probe(data: ProbeData<'_>) -> Self {
        let mut findings = Vec::new();
        let root_access = classify_root_access(data.root_marker, data.installed_packages);
        if !data.waydroid_running {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "waydroid-offline",
                message: "Start Waydroid to verify Android ABI, Play Store, and installed games"
                    .to_owned(),
            });
        }
        if data.waydroid_running && data.play_store == Some(false) {
            findings.push(Finding {
                severity: Severity::Action,
                code: "play-store-missing",
                message: "Google Play Store is not installed; use a Waydroid GAPPS image"
                    .to_owned(),
            });
        } else if data.play_store == Some(true) {
            findings.push(Finding {
                severity: Severity::Info,
                code: "play-store-ready",
                message:
                    "Google Play Store is available; account sign-in remains local to Waydroid"
                        .to_owned(),
            });
        }
        if data.waydroid_running && data.installed_packages.is_none() {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "android-package-list-unknown",
                message:
                    "Android package inventory did not become ready before the diagnostic deadline"
                        .to_owned(),
            });
        }

        match root_access.state {
            RootAccessState::Detected => findings.push(Finding {
                severity: Severity::Action,
                code: "android-root-detected",
                message: root_access.detail().to_owned(),
            }),
            RootAccessState::Unknown => findings.push(Finding {
                severity: Severity::Warning,
                code: "android-root-unknown",
                message: root_access.detail().to_owned(),
            }),
            RootAccessState::NotDetected => {}
        }

        match data.arm_translation {
            Some(false) if is_x86_host(&data.host_arch) => findings.push(Finding {
                severity: Severity::Action,
                code: "arm-translation-missing",
                message: "ARM native translation is not enabled; many Android game builds will be hidden or fail on x86_64"
                    .to_owned(),
            }),
            Some(true) => findings.push(Finding {
                severity: Severity::Info,
                code: "arm-translation-ready",
                message: format!(
                    "ARM application support is available{}",
                    data.native_bridge
                        .as_deref()
                        .filter(|value| native_bridge_enabled(value))
                        .map(|value| format!(" through {value}"))
                        .unwrap_or_default()
                ),
            }),
            None if is_x86_host(&data.host_arch) => findings.push(Finding {
                severity: Severity::Warning,
                code: "arm-translation-unknown",
                message:
                    "ARM translation could not be verified while Waydroid and its saved properties are unavailable"
                        .to_owned(),
            }),
            Some(false) | None => {}
        }

        if data.primary_abi.is_none() {
            findings.push(Finding {
                severity: Severity::Warning,
                code: "android-abi-unknown",
                message: "Android ABI could not be read while Waydroid is offline".to_owned(),
            });
        }

        let games = GAME_FAMILIES
            .iter()
            .map(|family| {
                game_compatibility(
                    *family,
                    data.waydroid_running,
                    data.play_store,
                    data.arm_translation,
                    data.installed_packages,
                )
            })
            .collect();

        Self {
            host_arch: data.host_arch,
            waydroid_running: data.waydroid_running,
            android_version: data.android_version,
            primary_abi: data.primary_abi,
            abi_list: data.abi_list,
            native_bridge: data.native_bridge,
            arm_translation: data.arm_translation,
            play_store: data.play_store,
            installed_packages: data.installed_packages.map(<[String]>::to_vec),
            root_access,
            games,
            findings,
        }
    }

    pub(crate) fn health(&self) -> &'static str {
        if self
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Action)
        {
            "action_required"
        } else if self
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Warning)
        {
            "warning"
        } else {
            "ready"
        }
    }

    pub(crate) fn game(&self, package: &str) -> Option<&GameCompatibility> {
        let family = family_for_package(package)?;
        self.games
            .iter()
            .find(|game| game.package == family.canonical_package)
    }

    pub(crate) fn ensure_package_installed_if_known(&self, package: &str) -> Result<()> {
        let Some(game) = self.game(package) else {
            return Ok(());
        };
        let installed = self.installed_packages.as_ref().map(|packages| {
            packages
                .iter()
                .any(|installed| installed.as_str() == package)
        });
        if installed == Some(false) {
            if is_x86_host(&self.host_arch) && self.arm_translation == Some(false) {
                bail!(
                    "{} is not installed and ARM translation is missing; enable libndk or libhoudini, then install {} from Google Play",
                    game.name,
                    package
                );
            }
            bail!(
                "{} is not installed; install {} from Google Play and refresh Wroid",
                game.name,
                package
            );
        }
        Ok(())
    }

    pub(crate) fn ensure_known_game_launch_ready(&self, package: &str) -> Result<()> {
        if family_for_package(package).is_none() {
            return Ok(());
        }
        if self.root_access.state == RootAccessState::Detected {
            bail!(self.root_access.detail());
        }
        self.ensure_package_installed_if_known(package)
    }

    pub(crate) fn as_json(&self) -> Value {
        let setup = setup_route();
        json!({
            "health": self.health(),
            "hostArch": self.host_arch,
            "waydroidRunning": self.waydroid_running,
            "androidVersion": self.android_version,
            "primaryAbi": self.primary_abi,
            "abiList": self.abi_list,
            "nativeBridge": self.native_bridge,
            "armTranslation": self.arm_translation,
            "playStore": self.play_store,
            "rootAccess": {
                "state": self.root_access.state.as_str(),
                "evidence": self.root_access.evidence,
                "detail": self.root_access.detail(),
            },
            "setup": {
                "kind": setup.kind,
                "label": setup.label,
                "detail": setup.detail,
            },
            "games": self.games.iter().map(|game| json!({
                "name": game.name,
                "package": game.package,
                "installed": game.installed,
                "installedPackage": game.installed_package,
                "state": game.state,
                "detail": game.detail,
            })).collect::<Vec<_>>(),
            "findings": self.findings.iter().map(|finding| json!({
                "severity": finding.severity.as_str(),
                "code": finding.code,
                "message": finding.message,
            })).collect::<Vec<_>>(),
        })
    }

    fn text(&self) -> String {
        let mut output = format!(
            "Game compatibility: {}\nHost: {} · Android {} · ABI {}\n",
            self.health().to_ascii_uppercase(),
            self.host_arch,
            self.android_version.as_deref().unwrap_or("unknown"),
            self.primary_abi.as_deref().unwrap_or("unknown")
        );
        output.push_str(&format!(
            "ABI list: {}\nNative bridge: {}\nARM translation: {}\nPlay Store: {}\nAndroid root: {}\n",
            if self.abi_list.is_empty() {
                "unknown".to_owned()
            } else {
                self.abi_list.join(", ")
            },
            self.native_bridge.as_deref().unwrap_or("disabled"),
            match self.arm_translation {
                Some(true) => "available",
                Some(false) => "missing",
                None => "unknown",
            },
            optional_status(self.play_store),
            self.root_access.state.as_str()
        ));
        for finding in &self.findings {
            output.push_str(&format!(
                "[{}] {}: {}\n",
                finding.severity.as_str().to_ascii_uppercase(),
                finding.code,
                finding.message
            ));
        }
        output.push_str("Games:\n");
        for game in &self.games {
            output.push_str(&format!(
                "  {} ({}) — {}: {}\n",
                game.name,
                game.package,
                game_state_label(game.state),
                game.detail
            ));
        }
        output
    }
}

fn probe_ready_package_list() -> Option<Vec<String>> {
    let expect_play_store = waydroid_config_expects_play_store();
    probe_ready_package_list_with(
        expect_play_store,
        APP_LIST_READY_TIMEOUT,
        APP_LIST_READY_INTERVAL,
        wroid_waydroid::app_list_packages_with_timeout,
    )
}

fn probe_ready_package_list_with<F>(
    expect_play_store: bool,
    timeout: Duration,
    interval: Duration,
    mut app_list: F,
) -> Option<Vec<String>>
where
    F: FnMut(Duration) -> Result<Vec<String>>,
{
    let started = Instant::now();
    let mut first_attempt = true;
    loop {
        if !first_attempt && started.elapsed() >= timeout {
            return None;
        }
        first_attempt = false;
        let remaining = timeout.saturating_sub(started.elapsed());
        if let Ok(packages) = app_list(remaining) {
            if started.elapsed() < timeout && package_list_is_ready(&packages, expect_play_store) {
                return Some(packages);
            }
        }
        let elapsed = started.elapsed();
        if elapsed >= timeout {
            return None;
        }
        thread::sleep(interval.min(timeout - elapsed));
    }
}

fn package_list_is_ready(packages: &[String], expect_play_store: bool) -> bool {
    let has_settings = packages
        .iter()
        .any(|package| package == "com.android.settings");
    let has_expected_store = !expect_play_store
        || packages
            .iter()
            .any(|package| package == "com.android.vending");
    has_settings && has_expected_store
}

fn probe_magisk_overlay(path: &Path) -> RootMarkerProbe {
    match fs::symlink_metadata(path) {
        Ok(_) => RootMarkerProbe::Present,
        Err(error) if error.kind() == io::ErrorKind::NotFound => RootMarkerProbe::Absent,
        Err(_) => RootMarkerProbe::Unknown,
    }
}

fn classify_root_access(
    marker: RootMarkerProbe,
    installed_packages: Option<&[String]>,
) -> RootAccess {
    if marker == RootMarkerProbe::Present {
        return RootAccess {
            state: RootAccessState::Detected,
            evidence: Some("magisk_overlay"),
        };
    }
    if installed_packages.is_some_and(|packages| {
        packages
            .iter()
            .any(|package| MAGISK_PACKAGES.contains(&package.as_str()))
    }) {
        return RootAccess {
            state: RootAccessState::Detected,
            evidence: Some("magisk_package"),
        };
    }
    if marker == RootMarkerProbe::Unknown || installed_packages.is_none() {
        return RootAccess {
            state: RootAccessState::Unknown,
            evidence: None,
        };
    }
    RootAccess {
        state: RootAccessState::NotDetected,
        evidence: None,
    }
}

fn waydroid_config_expects_play_store() -> bool {
    fs::read_to_string(WAYDROID_CONFIG).is_ok_and(|config| {
        config.lines().any(|line| {
            line.split_once('=').is_some_and(|(name, value)| {
                name.trim() == "system_ota" && value.to_ascii_lowercase().contains("gapps")
            })
        })
    })
}

struct ProbeData<'a> {
    host_arch: String,
    waydroid_running: bool,
    android_version: Option<String>,
    primary_abi: Option<String>,
    abi_list: Vec<String>,
    native_bridge: Option<String>,
    arm_translation: Option<bool>,
    play_store: Option<bool>,
    installed_packages: Option<&'a [String]>,
    root_marker: RootMarkerProbe,
}

fn game_compatibility(
    family: GameFamily,
    waydroid_running: bool,
    play_store: Option<bool>,
    arm_translation: Option<bool>,
    installed_packages: Option<&[String]>,
) -> GameCompatibility {
    let detected = installed_packages.and_then(|packages| installed_variant(&family, packages));
    let installed = installed_packages.map(|_| detected.is_some());
    let (state, detail) = if installed == Some(true) {
        (
            "installed",
            format!(
                "Detected {} ({}); Wroid controls profile is ready",
                detected.map_or(family.name, |variant| variant.name),
                detected.map_or(family.canonical_package, |variant| variant.package)
            ),
        )
    } else if !waydroid_running {
        (
            "unknown",
            "Start Waydroid and refresh compatibility".to_owned(),
        )
    } else if installed_packages.is_none() {
        (
            "runtime_not_ready",
            "Android package inventory is still starting; refresh compatibility".to_owned(),
        )
    } else if play_store == Some(false) {
        (
            "store_missing",
            "Install a GAPPS Waydroid image before using Google Play".to_owned(),
        )
    } else if is_x86_host(std::env::consts::ARCH) && arm_translation == Some(false) {
        (
            "arm_translation_needed",
            "Enable libndk or libhoudini before installing this game".to_owned(),
        )
    } else if arm_translation.is_none() {
        (
            "compatibility_unknown",
            "ARM application support could not be verified yet".to_owned(),
        )
    } else {
        (
            "ready_to_install",
            "Open Google Play, install the package, then refresh Wroid".to_owned(),
        )
    };

    GameCompatibility {
        name: family.name,
        package: family.canonical_package,
        installed,
        installed_package: detected.map(|variant| variant.package.to_owned()),
        state,
        detail,
    }
}

fn detects_arm_support(host_arch: &str, abi_list: &[String], native_bridge: Option<&str>) -> bool {
    host_arch.starts_with("aarch64")
        || host_arch.starts_with("arm")
        || abi_list.iter().any(|abi| {
            let abi = abi.to_ascii_lowercase();
            abi.starts_with("arm") || abi.starts_with("armeabi")
        })
        || native_bridge.is_some_and(native_bridge_enabled)
}

fn arm_support_status(
    host_arch: &str,
    abi_list: Option<&[String]>,
    native_bridge: Option<&str>,
) -> Option<bool> {
    if host_arch.starts_with("aarch64") || host_arch.starts_with("arm") {
        return Some(true);
    }
    if abi_list.is_none() && native_bridge.is_none() {
        return None;
    }
    Some(detects_arm_support(
        host_arch,
        abi_list.unwrap_or_default(),
        native_bridge,
    ))
}

fn native_bridge_enabled(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    !value.is_empty() && !matches!(value.as_str(), "0" | "false" | "none" | "disabled")
}

fn is_x86_host(arch: &str) -> bool {
    matches!(arch, "x86" | "x86_64")
}

fn waydroid_property(name: &str) -> Option<String> {
    let output = Command::new("waydroid")
        .args(["prop", "get", name])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn configured_waydroid_property(name: &str) -> Option<String> {
    let config = fs::read_to_string(WAYDROID_CONFIG).ok()?;
    ini_property(&config, "properties", name)
}

fn ini_property(config: &str, section: &str, key: &str) -> Option<String> {
    let mut active_section = None;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active_section = Some(line.trim_matches(['[', ']']));
            continue;
        }
        if active_section != Some(section) || line.starts_with(['#', ';']) {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() == key {
            return Some(value.trim().to_owned()).filter(|value| !value.is_empty());
        }
    }
    None
}

fn optional_status(status: Option<bool>) -> &'static str {
    match status {
        Some(true) => "available",
        Some(false) => "missing",
        None => "unknown",
    }
}

pub(crate) fn setup_route() -> SetupRoute {
    if command_path("waydroid-helper").is_some() {
        return SetupRoute {
            kind: "open_helper",
            label: "Open Waydroid Helper",
            detail: "Use Extensions to install libndk or libhoudini".to_owned(),
        };
    }
    if let Some(helper) = ["yay", "paru"]
        .into_iter()
        .find_map(|program| command_path(program).map(|path| (program, path)))
    {
        return SetupRoute {
            kind: if helper.0 == "yay" {
                "install_with_yay"
            } else {
                "install_with_paru"
            },
            label: "Install setup helper",
            detail: format!(
                "{} can install the AUR package waydroid-helper in a terminal",
                helper.0
            ),
        };
    }
    SetupRoute {
        kind: "documentation",
        label: "Open setup resources",
        detail: "No supported local setup helper was found".to_owned(),
    }
}

pub(crate) fn open_setup() -> Result<String> {
    let route = setup_route();
    match route.kind {
        "open_helper" => {
            Command::new("waydroid-helper")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("failed to open Waydroid Helper")?;
            Ok("Opened Waydroid Helper; choose an ARM translation extension".to_owned())
        }
        "install_with_yay" | "install_with_paru" => {
            let program = if route.kind == "install_with_yay" {
                "yay"
            } else {
                "paru"
            };
            let executable =
                env::current_exe().context("failed to locate the running Wroid executable")?;
            let command = [
                executable.into_os_string(),
                OsString::from("setup-waydroid-helper"),
                OsString::from("--installer"),
                OsString::from(program),
            ];
            let terminal = spawn_terminal(&command)?;
            Ok(format!(
                "Opened {terminal} to install and launch Waydroid Helper; review the AUR package before confirming"
            ))
        }
        _ => {
            Command::new("xdg-open")
                .arg(WAYDROID_COMMUNITY_URL)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .context("failed to open Waydroid setup resources")?;
            Ok("Opened official Waydroid community setup resources".to_owned())
        }
    }
}

pub(crate) fn install_and_open_helper(installer: &str) -> Result<()> {
    if !matches!(installer, "yay" | "paru") {
        bail!("unsupported Waydroid Helper installer: {installer}");
    }
    if command_path(installer).is_none() {
        bail!("{installer} is no longer available");
    }

    println!("Wroid ARM compatibility setup");
    println!("Installing the reviewed AUR package waydroid-helper with {installer}…");
    println!("Press Enter to approve the package transaction, then enter your sudo password.");
    io::stdout().flush().ok();
    let status = Command::new(installer)
        .args(helper_install_args(installer))
        .status()
        .with_context(|| format!("failed to start {installer}"))?;
    if !status.success() {
        pause_after_setup_failure();
        bail!("Waydroid Helper installation exited with {status}");
    }

    let helper = command_path("waydroid-helper")
        .context("waydroid-helper was not found after a successful package installation")?;
    Command::new(helper)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open the installed Waydroid Helper")?;
    println!("Waydroid Helper opened. In Extensions, install libndk or libhoudini.");
    Ok(())
}

fn helper_install_args(installer: &str) -> Vec<&'static str> {
    if installer == "yay" {
        vec![
            "-S",
            "--needed",
            "--answerclean",
            "None",
            "--answerdiff",
            "None",
            "waydroid-helper",
        ]
    } else {
        vec!["-S", "--needed", "waydroid-helper"]
    }
}

fn pause_after_setup_failure() {
    if !std::io::IsTerminal::is_terminal(&io::stdin()) {
        return;
    }
    eprint!("Setup did not finish. Press Enter to close this terminal… ");
    io::stderr().flush().ok();
    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
}

fn command_path(program: &str) -> Option<PathBuf> {
    let candidate = Path::new(program);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_owned());
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(program))
        .find(|path| path.is_file())
}

fn game_state_label(state: &str) -> &str {
    match state {
        "installed" => "INSTALLED",
        "ready_to_install" => "READY TO INSTALL",
        "arm_translation_needed" => "ARM SETUP NEEDED",
        "store_missing" => "GAPPS NEEDED",
        "compatibility_unknown" => "RUNTIME CHECK NEEDED",
        "unknown" => "PENDING",
        _ => state,
    }
}

pub(crate) fn run(json_output: bool, setup: bool) -> Result<()> {
    if setup {
        println!("{}", open_setup()?);
        return Ok(());
    }
    let report = CompatibilityReport::probe();
    if json_output {
        println!("{}", serde_json::to_string_pretty(&report.as_json())?);
    } else {
        print!("{}", report.text());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packages(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    fn report(
        arch: &str,
        running: bool,
        abi_list: &[&str],
        bridge: Option<&str>,
        packages: Option<&[String]>,
    ) -> CompatibilityReport {
        let abi_list = abi_list
            .iter()
            .map(|abi| (*abi).to_owned())
            .collect::<Vec<_>>();
        CompatibilityReport::from_probe(ProbeData {
            host_arch: arch.to_owned(),
            waydroid_running: running,
            android_version: Some("13".to_owned()),
            primary_abi: Some("x86_64".to_owned()),
            arm_translation: arm_support_status(arch, Some(&abi_list), bridge),
            abi_list,
            native_bridge: bridge.map(str::to_owned),
            play_store: packages.map(|packages| {
                packages
                    .iter()
                    .any(|package| package == "com.android.vending")
            }),
            installed_packages: packages,
            root_marker: RootMarkerProbe::Absent,
        })
    }

    #[test]
    fn x86_without_native_bridge_requires_action() {
        let packages = packages(&["com.android.vending"]);
        let report = report(
            "x86_64",
            true,
            &["x86_64", "x86"],
            Some("0"),
            Some(&packages),
        );
        assert_eq!(report.health(), "action_required");
        assert_eq!(report.arm_translation, Some(false));
        assert!(report
            .games
            .iter()
            .all(|game| game.state == "arm_translation_needed"));
    }

    #[test]
    fn native_bridge_makes_uninstalled_games_ready_for_store() {
        let packages = packages(&["com.android.vending"]);
        let report = report(
            "x86_64",
            true,
            &["x86_64", "x86"],
            Some("libndk_translation.so"),
            Some(&packages),
        );
        assert_eq!(report.health(), "ready");
        assert_eq!(report.arm_translation, Some(true));
        assert!(report
            .games
            .iter()
            .all(|game| game.state == "ready_to_install"));
    }

    #[test]
    fn installed_game_is_ready_even_before_native_bridge_setup() {
        let packages = packages(&["com.android.vending", "com.tencent.ig"]);
        let report = report("x86_64", true, &["x86_64"], Some("0"), Some(&packages));
        let pubg = report
            .games
            .iter()
            .find(|game| game.package == "com.tencent.ig")
            .unwrap();
        assert_eq!(pubg.state, "installed");
        assert!(report
            .ensure_package_installed_if_known("com.tencent.ig")
            .is_ok());
    }

    #[test]
    fn regional_pubg_installs_the_family_but_not_the_global_sibling() {
        let packages = packages(&["com.android.vending", "com.pubg.krmobile"]);
        let report = report(
            "x86_64",
            true,
            &["x86_64", "arm64-v8a"],
            Some("libhoudini.so"),
            Some(&packages),
        );
        let pubg = report.game("com.pubg.krmobile").unwrap();

        assert_eq!(pubg.state, "installed");
        assert_eq!(pubg.installed_package.as_deref(), Some("com.pubg.krmobile"));
        assert_eq!(
            report.as_json()["games"][0]["installedPackage"],
            "com.pubg.krmobile"
        );
        assert!(report
            .ensure_package_installed_if_known("com.pubg.krmobile")
            .is_ok());
        let error = report
            .ensure_package_installed_if_known("com.tencent.ig")
            .unwrap_err();
        assert!(error.to_string().contains("com.tencent.ig"));
        assert!(report.game("com.pubg.krmobile.clone").is_none());
    }

    #[test]
    fn free_fire_max_is_reported_as_the_installed_free_fire_edition() {
        let packages = packages(&["com.android.vending", "com.dts.freefiremax"]);
        let report = report(
            "x86_64",
            true,
            &["x86_64", "arm64-v8a"],
            Some("libhoudini.so"),
            Some(&packages),
        );
        let free_fire = report.game("com.dts.freefiremax").unwrap();

        assert_eq!(free_fire.installed, Some(true));
        assert_eq!(
            free_fire.installed_package.as_deref(),
            Some("com.dts.freefiremax")
        );
        assert!(report
            .ensure_package_installed_if_known("com.dts.freefiremax")
            .is_ok());
        assert!(report
            .ensure_package_installed_if_known("com.dts.freefireth")
            .is_err());
    }

    #[test]
    fn missing_popular_game_fails_before_session_teardown() {
        let packages = packages(&["com.android.vending"]);
        let report = report("x86_64", true, &["x86_64"], Some("0"), Some(&packages));
        let error = report
            .ensure_package_installed_if_known("com.tencent.ig")
            .unwrap_err();
        assert!(error.to_string().contains("ARM translation is missing"));
        assert!(report
            .ensure_package_installed_if_known("com.example.custom")
            .is_ok());
    }

    #[test]
    fn native_arm_host_does_not_need_translation() {
        assert!(detects_arm_support("aarch64", &[], Some("0")));
        assert_eq!(arm_support_status("aarch64", None, None), Some(true));
    }

    #[test]
    fn offline_x86_without_property_evidence_keeps_arm_status_unknown() {
        let report = CompatibilityReport::from_probe(ProbeData {
            host_arch: "x86_64".to_owned(),
            waydroid_running: false,
            android_version: None,
            primary_abi: None,
            abi_list: Vec::new(),
            native_bridge: None,
            arm_translation: None,
            play_store: None,
            installed_packages: None,
            root_marker: RootMarkerProbe::Absent,
        });

        assert_eq!(report.arm_translation, None);
        assert_eq!(report.health(), "warning");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "arm-translation-unknown"));
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.code == "arm-translation-missing"));
    }

    #[test]
    fn parses_saved_waydroid_arm_translation_properties() {
        let config = r#"
[waydroid]
arch = x86_64

[properties]
ro.product.cpu.abilist = x86_64,x86,arm64-v8a,armeabi-v7a
ro.dalvik.vm.native.bridge = libhoudini.so
"#;

        assert_eq!(
            ini_property(config, "properties", "ro.dalvik.vm.native.bridge").as_deref(),
            Some("libhoudini.so")
        );
        assert_eq!(
            ini_property(config, "properties", "ro.product.cpu.abilist").as_deref(),
            Some("x86_64,x86,arm64-v8a,armeabi-v7a")
        );
        assert_eq!(ini_property(config, "waydroid", "missing"), None);
    }

    #[test]
    fn enabled_bridge_values_are_detected() {
        for value in ["libhoudini.so", "libndk_translation.so", "arm64_houdini"] {
            assert!(native_bridge_enabled(value));
        }
        for value in ["", "0", "false", "none", "disabled"] {
            assert!(!native_bridge_enabled(value));
        }
    }

    #[test]
    fn setup_route_is_actionable() {
        let route = setup_route();
        assert!(matches!(
            route.kind,
            "open_helper" | "install_with_yay" | "install_with_paru" | "documentation"
        ));
        assert!(!route.label.is_empty());
        assert!(!route.detail.is_empty());
    }

    #[test]
    fn internal_helper_installer_rejects_arbitrary_commands() {
        let error = install_and_open_helper("sh").unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported Waydroid Helper installer"));
    }

    #[test]
    fn yay_setup_skips_only_build_housekeeping_prompts() {
        let args = helper_install_args("yay");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--answerclean", "None"]));
        assert!(args.windows(2).any(|pair| pair == ["--answerdiff", "None"]));
        assert!(!args.contains(&"--noconfirm"));
        assert_eq!(
            helper_install_args("paru"),
            ["-S", "--needed", "waydroid-helper"]
        );
    }

    #[test]
    fn package_probe_waits_for_android_launcher_apps() {
        assert!(!package_list_is_ready(&[], false));
        assert!(!package_list_is_ready(
            &packages(&["com.android.vending"]),
            true
        ));
        assert!(package_list_is_ready(
            &packages(&["com.android.settings"]),
            false
        ));
        assert!(!package_list_is_ready(
            &packages(&["com.android.settings"]),
            true
        ));
        assert!(package_list_is_ready(
            &packages(&["com.android.settings", "com.android.vending"]),
            true
        ));
    }

    #[test]
    fn package_probe_has_one_deadline_including_slow_attempts() {
        let started = std::time::Instant::now();
        let mut attempts = 0;

        let packages = probe_ready_package_list_with(
            false,
            Duration::from_millis(20),
            Duration::from_millis(1),
            |_| {
                attempts += 1;
                thread::sleep(Duration::from_millis(40));
                anyhow::bail!("simulated stuck package probe")
            },
        );

        assert_eq!(packages, None);
        assert_eq!(attempts, 1);
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn package_probe_rejects_a_success_returned_after_its_deadline() {
        let packages = probe_ready_package_list_with(
            false,
            Duration::from_millis(20),
            Duration::from_millis(1),
            |_| {
                thread::sleep(Duration::from_millis(40));
                Ok(packages(&["com.android.settings"]))
            },
        );

        assert_eq!(packages, None);
    }

    #[test]
    fn running_runtime_without_package_inventory_is_not_reported_ready_to_install() {
        let report = CompatibilityReport::from_probe(ProbeData {
            host_arch: "x86_64".to_owned(),
            waydroid_running: true,
            android_version: Some("13".to_owned()),
            primary_abi: Some("x86_64".to_owned()),
            abi_list: packages(&["x86_64", "arm64-v8a"]),
            native_bridge: Some("libhoudini.so".to_owned()),
            arm_translation: Some(true),
            play_store: None,
            installed_packages: None,
            root_marker: RootMarkerProbe::Absent,
        });

        assert_eq!(report.health(), "warning");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "android-package-list-unknown"));
        assert!(report
            .games
            .iter()
            .all(|game| game.state == "runtime_not_ready" && game.installed.is_none()));
    }

    #[test]
    fn active_magisk_overlay_requires_action() {
        let access = classify_root_access(
            RootMarkerProbe::Present,
            Some(&packages(&["com.android.settings"])),
        );

        assert_eq!(access.state, RootAccessState::Detected);
        assert_eq!(access.evidence, Some("magisk_overlay"));
    }

    #[test]
    fn active_magisk_package_requires_action() {
        for package in ["io.github.huskydg.magisk", "com.topjohnwu.magisk"] {
            let access = classify_root_access(
                RootMarkerProbe::Absent,
                Some(&packages(&["com.android.settings", package])),
            );

            assert_eq!(access.state, RootAccessState::Detected);
            assert_eq!(access.evidence, Some("magisk_package"));
        }
    }

    #[test]
    fn absent_active_signals_are_clean_even_when_stale_data_exists_elsewhere() {
        let access = classify_root_access(
            RootMarkerProbe::Absent,
            Some(&packages(&["com.android.settings"])),
        );

        assert_eq!(access.state, RootAccessState::NotDetected);
        assert_eq!(access.evidence, None);
    }

    #[test]
    fn incomplete_root_evidence_stays_unknown() {
        assert_eq!(
            classify_root_access(RootMarkerProbe::Unknown, Some(&packages(&[]))).state,
            RootAccessState::Unknown,
        );
        assert_eq!(
            classify_root_access(RootMarkerProbe::Absent, None).state,
            RootAccessState::Unknown,
        );
    }

    #[test]
    fn active_root_is_serialized_and_blocks_only_known_games() {
        let installed = packages(&[
            "com.android.settings",
            "com.android.vending",
            "com.axlebolt.standoff2",
        ]);
        let report = CompatibilityReport::from_probe(ProbeData {
            host_arch: "x86_64".to_owned(),
            waydroid_running: true,
            android_version: Some("13".to_owned()),
            primary_abi: Some("x86_64".to_owned()),
            abi_list: packages(&["x86_64", "arm64-v8a"]),
            native_bridge: Some("libhoudini.so".to_owned()),
            arm_translation: Some(true),
            play_store: Some(true),
            installed_packages: Some(&installed),
            root_marker: RootMarkerProbe::Present,
        });

        assert_eq!(report.health(), "action_required");
        assert_eq!(report.as_json()["rootAccess"]["state"], "detected");
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.code == "android-root-detected"));
        let error = report
            .ensure_known_game_launch_ready("com.axlebolt.standoff2")
            .unwrap_err();
        assert!(error.to_string().contains("Magisk"));
        assert!(report
            .ensure_known_game_launch_ready("com.example.custom")
            .is_ok());
    }
}
