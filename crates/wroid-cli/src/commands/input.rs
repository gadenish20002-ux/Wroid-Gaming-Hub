use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use wroid_core::BindingAction;

use crate::backend::{
    execute_keyevent, execute_swipe, execute_tap, select_input_backend, InputExecutor,
};
use crate::cli::InputBackend;
use crate::registry::load_validated_profile;
use crate::scaling::profile_for_execution;

pub(crate) fn input_tap(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    x: u32,
    y: u32,
) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    execute_tap(input_executor, selected_backend, x, y)
}

pub(crate) fn input_swipe(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration_ms: u64,
) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    execute_swipe(
        input_executor,
        selected_backend,
        x1,
        y1,
        x2,
        y2,
        duration_ms,
    )
}

pub(crate) fn input_keyevent(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    code: u32,
) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    execute_keyevent(input_executor, selected_backend, code)
}

pub(crate) fn run_binding(
    input_executor: &impl InputExecutor,
    profile_path: PathBuf,
    binding_name: &str,
    backend: InputBackend,
    scale_to_current: bool,
) -> Result<()> {
    let profile = load_validated_profile(&profile_path)?;
    let selected_backend = select_input_backend(input_executor, backend);
    let profile =
        profile_for_execution(input_executor, profile, selected_backend, scale_to_current)?;

    let binding = profile
        .binding(binding_name)
        .with_context(|| format!("binding {binding_name} not found"))?;

    match &binding.action {
        BindingAction::Tap { point } => {
            execute_tap(input_executor, selected_backend, point.x, point.y)
        }
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => execute_swipe(
            input_executor,
            selected_backend,
            from.x,
            from.y,
            to.x,
            to.y,
            *duration_ms,
        ),
        BindingAction::VirtualJoystick { .. } => bail!("virtual_joystick is not implemented"),
        BindingAction::MouseAim { .. } => bail!("mouse_aim is not implemented"),
        BindingAction::Macro { .. } => bail!("macro is not implemented"),
    }
}

#[cfg(test)]
mod tests {
    use wroid_core::ControlProfile;

    use crate::backend::SelectedInputBackend;
    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn input_tap_dispatches_to_auto_selected_waydroid_shell_backend() {
        let executor = FakeInputExecutor::with_device_state("offline-device", "offline");

        input_tap(&executor, InputBackend::Auto, 500, 400).unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidShellTap(500, 400)]
        );
    }

    #[test]
    fn input_swipe_dispatches_to_explicit_adb_backend() {
        let executor = FakeInputExecutor::default();

        input_swipe(&executor, InputBackend::Adb, 400, 500, 800, 500, 180).unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::AdbSwipe(400, 500, 800, 500, 180)]
        );
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn input_keyevent_dispatches_to_explicit_adb_backend() {
        let executor = FakeInputExecutor::default();

        input_keyevent(&executor, InputBackend::Adb, 3).unwrap();

        assert_eq!(executor.calls(), vec![InputCall::AdbKeyevent(3)]);
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn input_keyevent_dispatches_to_auto_selected_waydroid_shell_backend() {
        let executor = FakeInputExecutor::with_device_state("offline-device", "offline");

        input_keyevent(&executor, InputBackend::Auto, 4).unwrap();

        assert_eq!(executor.calls(), vec![InputCall::WaydroidShellKeyevent(4)]);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn scale_to_current_execution_uses_scaled_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("profile.json");
        ControlProfile::example()
            .save_to_path(&profile_path)
            .unwrap();
        let executor = FakeInputExecutor::with_waydroid_screen(1920, 1050);

        run_binding(
            &executor,
            profile_path,
            "fire",
            InputBackend::WaydroidShell,
            true,
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![
                InputCall::WaydroidShellWmSize,
                InputCall::WaydroidShellTap(1640, 525)
            ]
        );
    }

    #[test]
    fn execution_does_not_scale_when_flag_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("profile.json");
        ControlProfile::example()
            .save_to_path(&profile_path)
            .unwrap();
        let executor = FakeInputExecutor::with_waydroid_screen(1920, 1050);

        run_binding(
            &executor,
            profile_path,
            "fire",
            InputBackend::WaydroidShell,
            false,
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidShellTap(1640, 540)]
        );
    }

    #[test]
    fn scale_to_current_detection_failure_does_not_execute_action() {
        let dir = tempfile::tempdir().unwrap();
        let profile_path = dir.path().join("profile.json");
        ControlProfile::example()
            .save_to_path(&profile_path)
            .unwrap();
        let executor = FakeInputExecutor::default();

        let err = run_binding(
            &executor,
            profile_path,
            "fire",
            InputBackend::WaydroidShell,
            true,
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("failed to detect current screen size for coordinate scaling"));
        assert_eq!(executor.calls(), vec![InputCall::WaydroidShellWmSize]);
    }

    #[test]
    fn explicit_execute_helpers_do_not_query_auto_selection() {
        let executor = FakeInputExecutor::default();

        execute_tap(&executor, SelectedInputBackend::Adb, 1, 2).unwrap();

        assert_eq!(executor.calls(), vec![InputCall::AdbTap(1, 2)]);
        assert_eq!(executor.device_queries.get(), 0);
    }
}
