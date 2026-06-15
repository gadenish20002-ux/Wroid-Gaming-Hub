use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::backend::{select_input_backend, InputExecutor, SelectedInputBackend};
use crate::cli::InputBackend;
use crate::commands::app::launch_profile_package;
use crate::interactive::start_interactive_keymapper;
use crate::registry::{
    effective_uid, env_value, load_play_profile, profile_registry_dir, registry_profile_file_path,
};
use crate::scaling::profile_for_execution;

pub(crate) fn play(
    input_executor: &impl InputExecutor,
    profile_path: PathBuf,
    backend: InputBackend,
    scale_to_current: bool,
) -> Result<()> {
    let profile = load_play_profile(&profile_path)?;
    let selected_backend = select_input_backend(input_executor, backend);
    let profile =
        profile_for_execution(input_executor, profile, selected_backend, scale_to_current)?;

    println!("Profile: {}", profile.name);
    println!("Package: {}", profile.package_name);
    start_interactive_keymapper(input_executor, &profile, selected_backend)
}

pub(crate) fn run(
    input_executor: &impl InputExecutor,
    profile_path: PathBuf,
    backend: InputBackend,
    options: RunOptions,
) -> Result<()> {
    let profile = load_play_profile(&profile_path)?;
    let selected_backend = select_input_backend(input_executor, backend);
    let profile = profile_for_execution(
        input_executor,
        profile,
        selected_backend,
        options.scale_to_current,
    )?;

    println!("Profile: {}", profile.name);
    println!("Package: {}", profile.package_name);
    if options.no_launch {
        println!("Skipping app launch (--no-launch).");
    } else {
        println!("Launching package {} ...", profile.package_name);
    }
    io::stdout().flush().context("failed to flush stdout")?;

    let launch_context = RunLaunchContext::current();
    run_game_workflow_steps(
        || {
            launch_run_package(
                input_executor,
                selected_backend,
                &profile.package_name,
                &profile_path,
                &launch_context,
            )
        },
        std::thread::sleep,
        || {
            println!("Starting keymapper ...");
            start_interactive_keymapper(input_executor, &profile, selected_backend)
        },
        options,
    )
}

pub(crate) fn run_profile(
    input_executor: &impl InputExecutor,
    profile_id: &str,
    backend: InputBackend,
    options: RunOptions,
) -> Result<()> {
    let registry_dir = profile_registry_dir()?;
    resolve_registered_profile_and_run(
        profile_id,
        &registry_dir,
        backend,
        options,
        |path, options| run(input_executor, path, backend, options),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RunOptions {
    pub(crate) launch_delay_ms: u64,
    pub(crate) no_launch: bool,
    pub(crate) scale_to_current: bool,
}

pub(crate) fn resolve_registered_profile_and_run(
    profile_id: &str,
    registry_dir: &Path,
    backend: InputBackend,
    options: RunOptions,
    run_profile_path: impl FnOnce(PathBuf, RunOptions) -> Result<()>,
) -> Result<()> {
    let profile_path = registry_profile_file_path(registry_dir, profile_id)?;
    run_profile_path(profile_path, options).with_context(|| {
        format!(
            "failed to run registered profile {profile_id} with backend {backend}, launch delay {} ms, no-launch {}",
            options.launch_delay_ms, options.no_launch
        )
    })
}

pub(crate) fn run_game_workflow_steps(
    launch_package: impl FnOnce() -> Result<()>,
    wait_for_launch: impl FnOnce(Duration),
    start_keymapper: impl FnOnce() -> Result<()>,
    options: RunOptions,
) -> Result<()> {
    if !options.no_launch {
        launch_package()?;
        wait_for_launch(Duration::from_millis(options.launch_delay_ms));
    }
    start_keymapper()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RunLaunchContext {
    effective_uid: u32,
    sudo_user: Option<String>,
    sudo_uid: Option<String>,
    wayland_display: Option<String>,
    xdg_session_type: Option<String>,
    display: Option<String>,
}

impl RunLaunchContext {
    fn current() -> Self {
        Self {
            effective_uid: effective_uid(),
            sudo_user: env_value("SUDO_USER"),
            sudo_uid: env_value("SUDO_UID"),
            wayland_display: env_value("WAYLAND_DISPLAY"),
            xdg_session_type: env_value("XDG_SESSION_TYPE"),
            display: env_value("DISPLAY"),
        }
    }
}

fn launch_run_package(
    input_executor: &impl InputExecutor,
    selected_backend: SelectedInputBackend,
    package_name: &str,
    profile_path: &Path,
    launch_context: &RunLaunchContext,
) -> Result<()> {
    match selected_backend {
        SelectedInputBackend::Adb => {
            launch_profile_package(input_executor, selected_backend, package_name)
        }
        SelectedInputBackend::WaydroidShell => {
            if let Some(user) = original_sudo_user_for_launch(launch_context) {
                let Some(session_env) = sudo_user_session_env(launch_context) else {
                    return launch_profile_package(input_executor, selected_backend, package_name);
                };

                input_executor
                    .waydroid_app_launch_package_as_user(package_name, user, &session_env)
                    .with_context(|| {
                        waydroid_sudo_user_launch_error(package_name, profile_path, user)
                    })
            } else {
                launch_profile_package(input_executor, selected_backend, package_name)
            }
        }
    }
}

fn original_sudo_user_for_launch(launch_context: &RunLaunchContext) -> Option<&str> {
    if launch_context.effective_uid != 0 {
        return None;
    }

    launch_context
        .sudo_user
        .as_deref()
        .map(str::trim)
        .filter(|user| !user.is_empty() && *user != "root")
}

fn sudo_user_session_env(
    launch_context: &RunLaunchContext,
) -> Option<wroid_waydroid::WaydroidAppLaunchEnv> {
    let sudo_uid = launch_context.sudo_uid.as_deref()?.trim();
    if sudo_uid.is_empty() {
        return None;
    }

    Some(wroid_waydroid::WaydroidAppLaunchEnv {
        xdg_runtime_dir: format!("/run/user/{sudo_uid}"),
        dbus_session_bus_address: format!("unix:path=/run/user/{sudo_uid}/bus"),
        wayland_display: launch_context
            .wayland_display
            .clone()
            .unwrap_or_else(|| "wayland-0".to_owned()),
        xdg_session_type: launch_context
            .xdg_session_type
            .clone()
            .unwrap_or_else(|| "wayland".to_owned()),
        display: launch_context.display.clone(),
    })
}

fn waydroid_sudo_user_launch_error(package_name: &str, profile_path: &Path, user: &str) -> String {
    format!(
        "failed to launch Android package {package_name} as {user} via waydroid-shell. \
Waydroid app launch needs the original desktop user's DBus session. Try launching the app first with: \
target/debug/wroid app launch {package_name} --backend waydroid-shell; \
then start the keymapper with: sudo target/debug/wroid run {} --backend waydroid-shell --no-launch",
        profile_path.display()
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::commands::app::launch_profile_package;
    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn run_profile_resolves_registered_path_for_existing_run_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        let called_path = RefCell::new(None);

        resolve_registered_profile_and_run(
            "com.android.settings",
            &registry_dir,
            InputBackend::WaydroidShell,
            RunOptions {
                launch_delay_ms: 2500,
                no_launch: false,
                scale_to_current: false,
            },
            |path, _options| {
                *called_path.borrow_mut() = Some(path);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            called_path.into_inner(),
            Some(registry_dir.join("com.android.settings.json"))
        );
    }

    #[test]
    fn run_workflow_launches_package_before_keymapper_setup() {
        let executor = FakeInputExecutor::default();

        run_game_workflow_steps(
            || {
                launch_profile_package(
                    &executor,
                    SelectedInputBackend::WaydroidShell,
                    "com.example.game",
                )
            },
            |duration| {
                executor
                    .calls
                    .borrow_mut()
                    .push(InputCall::LaunchDelay(duration.as_millis()));
            },
            || {
                executor.calls.borrow_mut().push(InputCall::StartKeymapper);
                Ok(())
            },
            RunOptions {
                launch_delay_ms: 2500,
                no_launch: false,
                scale_to_current: false,
            },
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![
                InputCall::WaydroidAppLaunchPackage("com.example.game".to_owned()),
                InputCall::LaunchDelay(2500),
                InputCall::StartKeymapper,
            ]
        );
    }

    #[test]
    fn run_workflow_does_not_start_keymapper_when_launch_fails() {
        let executor = FakeInputExecutor {
            fail_launch: true,
            ..FakeInputExecutor::default()
        };

        let err = run_game_workflow_steps(
            || {
                launch_profile_package(
                    &executor,
                    SelectedInputBackend::WaydroidShell,
                    "com.example.game",
                )
            },
            |duration| {
                executor
                    .calls
                    .borrow_mut()
                    .push(InputCall::LaunchDelay(duration.as_millis()));
            },
            || {
                executor.calls.borrow_mut().push(InputCall::StartKeymapper);
                Ok(())
            },
            RunOptions {
                launch_delay_ms: 1500,
                no_launch: false,
                scale_to_current: false,
            },
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("failed to launch Android package com.example.game via waydroid-shell"));
        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackage(
                "com.example.game".to_owned()
            )]
        );
    }

    #[test]
    fn run_workflow_with_no_launch_does_not_call_app_launch() {
        let executor = FakeInputExecutor::default();

        run_game_workflow_steps(
            || {
                launch_profile_package(
                    &executor,
                    SelectedInputBackend::WaydroidShell,
                    "com.example.game",
                )
            },
            |duration| {
                executor
                    .calls
                    .borrow_mut()
                    .push(InputCall::LaunchDelay(duration.as_millis()));
            },
            || {
                executor.calls.borrow_mut().push(InputCall::StartKeymapper);
                Ok(())
            },
            RunOptions {
                launch_delay_ms: 2500,
                no_launch: true,
                scale_to_current: false,
            },
        )
        .unwrap();

        assert!(!executor
            .calls()
            .iter()
            .any(|call| matches!(call, InputCall::WaydroidAppLaunchPackage(_))));
    }

    #[test]
    fn run_workflow_with_no_launch_starts_keymapper() {
        let executor = FakeInputExecutor::default();

        run_game_workflow_steps(
            || {
                launch_profile_package(
                    &executor,
                    SelectedInputBackend::WaydroidShell,
                    "com.example.game",
                )
            },
            |duration| {
                executor
                    .calls
                    .borrow_mut()
                    .push(InputCall::LaunchDelay(duration.as_millis()));
            },
            || {
                executor.calls.borrow_mut().push(InputCall::StartKeymapper);
                Ok(())
            },
            RunOptions {
                launch_delay_ms: 2500,
                no_launch: true,
                scale_to_current: false,
            },
        )
        .unwrap();

        assert_eq!(executor.calls(), vec![InputCall::StartKeymapper]);
    }

    #[test]
    fn run_profile_passes_no_launch_into_existing_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let registry_dir = dir.path().join("registry");
        let called = RefCell::new(None);
        let options = RunOptions {
            launch_delay_ms: 2500,
            no_launch: true,
            scale_to_current: false,
        };

        resolve_registered_profile_and_run(
            "com.android.settings",
            &registry_dir,
            InputBackend::WaydroidShell,
            options,
            |path, received_options| {
                *called.borrow_mut() = Some((path, received_options));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            called.into_inner(),
            Some((registry_dir.join("com.android.settings.json"), options))
        );
    }

    #[test]
    fn run_workflow_ignores_launch_delay_when_no_launch_is_set() {
        let executor = FakeInputExecutor::default();

        run_game_workflow_steps(
            || {
                launch_profile_package(
                    &executor,
                    SelectedInputBackend::WaydroidShell,
                    "com.example.game",
                )
            },
            |duration| {
                executor
                    .calls
                    .borrow_mut()
                    .push(InputCall::LaunchDelay(duration.as_millis()));
            },
            || {
                executor.calls.borrow_mut().push(InputCall::StartKeymapper);
                Ok(())
            },
            RunOptions {
                launch_delay_ms: 9999,
                no_launch: true,
                scale_to_current: false,
            },
        )
        .unwrap();

        assert!(!executor
            .calls()
            .iter()
            .any(|call| matches!(call, InputCall::LaunchDelay(_))));
    }

    #[test]
    fn run_launch_uses_sudo_user_for_waydroid_shell_when_root() {
        let executor = FakeInputExecutor::default();
        let launch_context = RunLaunchContext {
            effective_uid: 0,
            sudo_user: Some("alice".to_owned()),
            sudo_uid: Some("1000".to_owned()),
            wayland_display: None,
            xdg_session_type: None,
            display: None,
        };

        launch_run_package(
            &executor,
            SelectedInputBackend::WaydroidShell,
            "com.example.game",
            Path::new("/tmp/game.json"),
            &launch_context,
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackageAsUser {
                package: "com.example.game".to_owned(),
                user: "alice".to_owned(),
                session_env: wroid_waydroid::WaydroidAppLaunchEnv {
                    xdg_runtime_dir: "/run/user/1000".to_owned(),
                    dbus_session_bus_address: "unix:path=/run/user/1000/bus".to_owned(),
                    wayland_display: "wayland-0".to_owned(),
                    xdg_session_type: "wayland".to_owned(),
                    display: None,
                },
            }]
        );
    }

    #[test]
    fn run_launch_copies_desktop_display_env_for_sudo_user() {
        let executor = FakeInputExecutor::default();
        let launch_context = RunLaunchContext {
            effective_uid: 0,
            sudo_user: Some("supergut".to_owned()),
            sudo_uid: Some("1000".to_owned()),
            wayland_display: Some("wayland-1".to_owned()),
            xdg_session_type: Some("wayland".to_owned()),
            display: Some(":0".to_owned()),
        };

        launch_run_package(
            &executor,
            SelectedInputBackend::WaydroidShell,
            "com.android.settings",
            Path::new("/tmp/settings.json"),
            &launch_context,
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackageAsUser {
                package: "com.android.settings".to_owned(),
                user: "supergut".to_owned(),
                session_env: wroid_waydroid::WaydroidAppLaunchEnv {
                    xdg_runtime_dir: "/run/user/1000".to_owned(),
                    dbus_session_bus_address: "unix:path=/run/user/1000/bus".to_owned(),
                    wayland_display: "wayland-1".to_owned(),
                    xdg_session_type: "wayland".to_owned(),
                    display: Some(":0".to_owned()),
                },
            }]
        );
    }

    #[test]
    fn run_launch_uses_normal_waydroid_app_launch_when_not_root() {
        let executor = FakeInputExecutor::default();
        let launch_context = RunLaunchContext {
            effective_uid: 1000,
            sudo_user: Some("alice".to_owned()),
            sudo_uid: Some("1000".to_owned()),
            wayland_display: None,
            xdg_session_type: None,
            display: None,
        };

        launch_run_package(
            &executor,
            SelectedInputBackend::WaydroidShell,
            "com.example.game",
            Path::new("/tmp/game.json"),
            &launch_context,
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackage(
                "com.example.game".to_owned()
            )]
        );
    }

    #[test]
    fn run_launch_keeps_adb_backend_unchanged_under_sudo_like_env() {
        let executor = FakeInputExecutor::default();
        let launch_context = RunLaunchContext {
            effective_uid: 0,
            sudo_user: Some("alice".to_owned()),
            sudo_uid: Some("1000".to_owned()),
            wayland_display: None,
            xdg_session_type: None,
            display: None,
        };

        launch_run_package(
            &executor,
            SelectedInputBackend::Adb,
            "com.example.game",
            Path::new("/tmp/game.json"),
            &launch_context,
        )
        .unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::AdbLaunchPackage("com.example.game".to_owned())]
        );
    }

    #[test]
    fn run_launch_as_sudo_user_error_includes_dbus_recovery_commands() {
        let executor = FakeInputExecutor {
            fail_launch: true,
            ..FakeInputExecutor::default()
        };
        let launch_context = RunLaunchContext {
            effective_uid: 0,
            sudo_user: Some("alice".to_owned()),
            sudo_uid: Some("1000".to_owned()),
            wayland_display: None,
            xdg_session_type: None,
            display: None,
        };

        let err = launch_run_package(
            &executor,
            SelectedInputBackend::WaydroidShell,
            "com.example.game",
            Path::new("/tmp/game.json"),
            &launch_context,
        )
        .unwrap_err();
        let message = err.to_string();

        assert!(message.contains("DBus session"));
        assert!(message
            .contains("target/debug/wroid app launch com.example.game --backend waydroid-shell"));
        assert!(message.contains(
            "sudo target/debug/wroid run /tmp/game.json --backend waydroid-shell --no-launch"
        ));
        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackageAsUser {
                package: "com.example.game".to_owned(),
                user: "alice".to_owned(),
                session_env: wroid_waydroid::WaydroidAppLaunchEnv {
                    xdg_runtime_dir: "/run/user/1000".to_owned(),
                    dbus_session_bus_address: "unix:path=/run/user/1000/bus".to_owned(),
                    wayland_display: "wayland-0".to_owned(),
                    xdg_session_type: "wayland".to_owned(),
                    display: None,
                },
            }]
        );
    }
}
