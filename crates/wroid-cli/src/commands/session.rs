//! Daemon-backed session commands.
//!
//! These commands exercise the in-memory daemon session manager so the CLI can
//! prepare profile v2 control plans through the same typed runtime contracts the
//! future `wroidd` process and desktop UI will use. No input capture, injection,
//! or Waydroid lifecycle work happens here yet: preparation only materializes a
//! runtime control plan and reports it.

use std::fmt::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};
use wroid_core::profile_v2::{InputV2, ProfileV2};
use wroid_core::Resolution;
use wroid_daemon::DaemonSessionManager;
use wroid_runtime::{
    DisplayInfo, PreparedSession, RuntimeControlAction, RuntimeControlPlan, SessionId,
};

use crate::output;

/// Load a profile v2 document and prepare a daemon-managed runtime control plan.
pub(crate) fn prepare_v2(
    profile_path: PathBuf,
    resolution: Resolution,
    session_id: String,
    no_launch: bool,
) -> Result<()> {
    let profile = ProfileV2::load_from_path(&profile_path)
        .with_context(|| format!("failed to load profile v2 from {}", profile_path.display()))?;

    let summary = prepare_v2_summary(profile, resolution, &session_id, !no_launch)
        .with_context(|| format!("failed to prepare session '{session_id}'"))?;

    output::write_stdout(&summary)
}

/// Prepare the session in an in-memory daemon and render a human-readable report.
///
/// Kept separate from [`prepare_v2`] so tests can assert the rendered control
/// plan and the unsupported-action failure path without touching the filesystem.
fn prepare_v2_summary(
    profile: ProfileV2,
    resolution: Resolution,
    session_id: &str,
    launch_package: bool,
) -> Result<String> {
    // Surface detailed validation messages before the daemon collapses them into
    // a generic "N validation error(s)" summary.
    if let Err(error) = profile.validate() {
        anyhow::bail!("invalid profile v2:\n  - {}", error.errors.join("\n  - "));
    }

    let session_id = SessionId::new(session_id).context("invalid session id")?;

    let mut manager = DaemonSessionManager::new();
    let prepared = manager.prepare_profile_v2(
        session_id.clone(),
        profile,
        DisplayInfo::new(resolution),
        launch_package,
    )?;

    let plan = manager
        .session(&prepared.session_id)
        .and_then(|session| session.control_plan())
        .context("prepared session is missing its control plan")?;

    Ok(render_prepared(&prepared, plan, launch_package))
}

fn render_prepared(
    prepared: &PreparedSession,
    plan: &RuntimeControlPlan,
    launch_package: bool,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "Prepared session: {}", prepared.session_id.as_str());
    let _ = writeln!(output, "State: {:?}", prepared.state);
    let _ = writeln!(output, "Profile: {}", plan.profile_name);
    let _ = writeln!(output, "Package: {}", plan.package_name);
    let _ = writeln!(output, "Launch package: {launch_package}");
    let _ = writeln!(
        output,
        "Resolution: {}x{}",
        plan.resolution.width, plan.resolution.height
    );
    let _ = writeln!(output, "Controls ({}):", plan.controls.len());
    for control in &plan.controls {
        let _ = writeln!(
            output,
            "  {} [{}] -> {}",
            control.name,
            describe_input(&control.input),
            describe_action(&control.action),
        );
    }
    output
}

fn describe_input(input: &InputV2) -> String {
    match input {
        InputV2::Key { key } => format!("key:{key}"),
        InputV2::KeyCluster {
            up,
            left,
            down,
            right,
        } => format!("key_cluster:{up}{left}{down}{right}"),
        InputV2::MouseButton { button } => format!("mouse_button:{button}"),
        InputV2::MouseMove => "mouse_move".to_owned(),
    }
}

fn describe_action(action: &RuntimeControlAction) -> String {
    match action {
        RuntimeControlAction::Tap { point } => format!("tap ({},{})", point.x, point.y),
        RuntimeControlAction::VirtualJoystick { joystick } => format!(
            "virtual_joystick center=({},{}) radius={} dead_zone={} contact={}",
            joystick.center().x,
            joystick.center().y,
            joystick.radius(),
            joystick.dead_zone(),
            joystick.contact_id().get(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_core::profile_v2::{ActionV2, BindingV2, NormalizedPoint, NormalizedRect};

    fn supported_profile() -> ProfileV2 {
        ProfileV2 {
            schema_version: 2,
            name: "Shooter v2".to_owned(),
            package_name: "com.example.shooter".to_owned(),
            orientation: Default::default(),
            bindings: vec![
                BindingV2 {
                    name: "movement".to_owned(),
                    input: InputV2::KeyCluster {
                        up: "w".to_owned(),
                        left: "a".to_owned(),
                        down: "s".to_owned(),
                        right: "d".to_owned(),
                    },
                    action: ActionV2::VirtualJoystick {
                        center: NormalizedPoint { x: 0.18, y: 0.78 },
                        radius: 0.09,
                        dead_zone: 0.02,
                        mode: Default::default(),
                        reaffirm_ms: Some(50),
                    },
                },
                BindingV2 {
                    name: "fire".to_owned(),
                    input: InputV2::MouseButton {
                        button: "left".to_owned(),
                    },
                    action: ActionV2::Tap {
                        point: NormalizedPoint { x: 0.86, y: 0.50 },
                    },
                },
            ],
        }
    }

    fn resolution() -> Resolution {
        Resolution {
            width: 1920,
            height: 1080,
        }
    }

    #[test]
    fn prepares_supported_profile_into_control_plan_summary() {
        let summary =
            prepare_v2_summary(supported_profile(), resolution(), "shooter", true).unwrap();

        assert!(summary.contains("Prepared session: shooter"));
        assert!(summary.contains("State: Preparing"));
        assert!(summary.contains("Package: com.example.shooter"));
        assert!(summary.contains("Launch package: true"));
        assert!(summary.contains("Resolution: 1920x1080"));
        assert!(summary.contains("Controls (2):"));
        // The joystick materializes its geometry against the surface resolution.
        assert!(summary.contains(
            "movement [key_cluster:wasd] -> virtual_joystick center=(345,842) radius=97 dead_zone=22 contact=1"
        ));
        assert!(summary.contains("fire [mouse_button:left] -> tap (1650,540)"));
    }

    #[test]
    fn no_launch_is_reflected_in_summary() {
        let summary =
            prepare_v2_summary(supported_profile(), resolution(), "shooter", false).unwrap();

        assert!(summary.contains("Launch package: false"));
    }

    #[test]
    fn rejects_unsupported_mouse_aim_action_with_clear_error() {
        let mut profile = supported_profile();
        profile.bindings[0].action = ActionV2::MouseAim {
            region: NormalizedRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            sensitivity: 1.0,
        };

        let error = prepare_v2_summary(profile, resolution(), "shooter", true).unwrap_err();
        let message = format!("{error:#}");

        assert!(
            message.contains("unsupported runtime action kind: mouse_aim"),
            "unexpected error message: {message}"
        );
        assert!(
            message.contains("movement"),
            "error should name the offending binding: {message}"
        );
    }

    #[test]
    fn rejects_invalid_profile_with_detailed_messages() {
        let mut profile = supported_profile();
        profile.package_name.clear();

        let error = prepare_v2_summary(profile, resolution(), "shooter", true).unwrap_err();
        let message = format!("{error:#}");

        assert!(
            message.contains("invalid profile v2"),
            "unexpected error message: {message}"
        );
    }

    #[test]
    fn rejects_empty_session_id() {
        let error = prepare_v2_summary(supported_profile(), resolution(), "   ", true).unwrap_err();
        let message = format!("{error:#}");

        assert!(
            message.contains("invalid session id"),
            "unexpected error message: {message}"
        );
    }
}
