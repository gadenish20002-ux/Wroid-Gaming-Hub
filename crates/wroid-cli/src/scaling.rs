use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use wroid_core::{scale_profile, ControlProfile, Resolution};

use crate::backend::{InputExecutor, SelectedInputBackend};
use crate::cli::InputBackend;
use crate::device::{detect_device_screen, detect_device_screen_with_selected_backend};
use crate::registry::{load_validated_profile, save_profile};

pub(crate) fn scale_profile_file(
    input_path: PathBuf,
    output_path: PathBuf,
    resolution: Resolution,
    force: bool,
) -> Result<()> {
    write_scaled_profile_file(input_path, output_path, resolution, force)
}

pub(crate) fn scale_profile_file_to_current_screen(
    input_executor: &impl InputExecutor,
    input_path: PathBuf,
    output_path: PathBuf,
    backend: InputBackend,
    force: bool,
) -> Result<()> {
    let resolution = detect_device_screen(input_executor, backend)?;
    write_scaled_profile_file(input_path, output_path, resolution, force)
}

fn write_scaled_profile_file(
    input_path: PathBuf,
    output_path: PathBuf,
    resolution: Resolution,
    force: bool,
) -> Result<()> {
    if resolution.width == 0 || resolution.height == 0 {
        bail!("target resolution must be non-zero, got {resolution}");
    }

    if output_path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            output_path.display()
        );
    }

    let profile = load_validated_profile(&input_path)?;
    let scaled = scale_profile(&profile, resolution);
    scaled
        .validate()
        .with_context(|| format!("scaled profile {} is invalid", output_path.display()))?;
    save_profile(&scaled, &output_path)?;
    println!(
        "Scaled profile coordinates: {} -> {}",
        profile.resolution, resolution
    );
    println!("wrote scaled profile: {}", output_path.display());
    Ok(())
}

pub(crate) fn profile_for_execution(
    input_executor: &impl InputExecutor,
    profile: ControlProfile,
    selected_backend: SelectedInputBackend,
    scale_to_current: bool,
) -> Result<ControlProfile> {
    if !scale_to_current {
        return Ok(profile);
    }

    let current_resolution =
        detect_device_screen_with_selected_backend(input_executor, selected_backend)
            .with_context(|| "failed to detect current screen size for coordinate scaling")?;

    if current_resolution == profile.resolution {
        println!("Profile resolution already matches current screen.");
        return Ok(profile);
    }

    println!(
        "Scaling profile coordinates: {} -> {}",
        profile.resolution, current_resolution
    );
    Ok(scale_profile(&profile, current_resolution))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use wroid_core::{BindingAction, ControlProfile, Point, Resolution};

    use crate::backend::SelectedInputBackend;
    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn profile_scale_updates_resolution_and_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let input_path = dir.path().join("source.json");
        let output_path = dir.path().join("scaled.json");
        ControlProfile::example().save_to_path(&input_path).unwrap();

        scale_profile_file(
            input_path,
            output_path.clone(),
            Resolution {
                width: 1920,
                height: 1050,
            },
            false,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(output_path).unwrap();
        assert_eq!(profile.name, "Shooter Basic");
        assert_eq!(profile.package_name, "com.example.shooter");
        assert_eq!(
            profile.resolution,
            Resolution {
                width: 1920,
                height: 1050
            }
        );
        assert_eq!(
            profile.bindings[0].action,
            BindingAction::Tap {
                point: Point { x: 1640, y: 525 }
            }
        );
        assert_eq!(
            profile.bindings[2].action,
            BindingAction::Swipe {
                from: Point { x: 960, y: 525 },
                to: Point { x: 1260, y: 525 },
                duration_ms: 180,
            }
        );
        profile.validate().unwrap();
    }

    #[test]
    fn profile_scale_rejects_existing_output_without_force() {
        let dir = tempfile::tempdir().unwrap();
        let input_path = dir.path().join("source.json");
        let output_path = dir.path().join("scaled.json");
        ControlProfile::example().save_to_path(&input_path).unwrap();
        fs::write(&output_path, b"existing").unwrap();

        let err = scale_profile_file(
            input_path,
            output_path,
            Resolution {
                width: 1920,
                height: 1050,
            },
            false,
        )
        .unwrap_err();

        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn scale_current_uses_detected_screen_size() {
        let dir = tempfile::tempdir().unwrap();
        let input_path = dir.path().join("source.json");
        let output_path = dir.path().join("scaled.json");
        ControlProfile::example().save_to_path(&input_path).unwrap();
        let executor = FakeInputExecutor::with_waydroid_screen(1920, 1050);

        scale_profile_file_to_current_screen(
            &executor,
            input_path,
            output_path.clone(),
            InputBackend::WaydroidShell,
            false,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(output_path).unwrap();
        assert_eq!(
            profile.resolution,
            Resolution {
                width: 1920,
                height: 1050
            }
        );
        assert_eq!(
            profile.bindings[0].action,
            BindingAction::Tap {
                point: Point { x: 1640, y: 525 }
            }
        );
        assert_eq!(executor.calls(), vec![InputCall::WaydroidShellWmSize]);
    }

    #[test]
    fn profile_for_execution_scales_when_current_resolution_differs() {
        let executor = FakeInputExecutor::with_waydroid_screen(1920, 1050);

        let profile = profile_for_execution(
            &executor,
            ControlProfile::example(),
            SelectedInputBackend::WaydroidShell,
            true,
        )
        .unwrap();

        assert_eq!(
            profile.resolution,
            Resolution {
                width: 1920,
                height: 1050
            }
        );
        assert_eq!(executor.calls(), vec![InputCall::WaydroidShellWmSize]);
    }
}
