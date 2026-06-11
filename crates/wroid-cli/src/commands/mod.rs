pub(crate) mod app;
pub(crate) mod device;
pub(crate) mod input;
pub(crate) mod profile;
pub(crate) mod run;

use anyhow::{Context, Result};
use wroid_core::Resolution;

use crate::backend::InputExecutor;
use crate::cli::{
    AppCommand, BindingCommand, Cli, Commands, DeviceCommand, InputCommand, ProfileCommand,
};

pub(crate) fn run(cli: Cli, input_executor: &impl InputExecutor) -> Result<()> {
    match cli.command {
        Commands::Doctor => doctor(),
        Commands::Profile { command } => match command {
            ProfileCommand::Validate { path } => profile::validate_profile(path),
            ProfileCommand::Example { path } => profile::write_example_profile(path),
            ProfileCommand::New {
                path,
                name,
                package,
                width,
                height,
                force,
            } => profile::create_profile(path, name, package, width, height, force),
            ProfileCommand::NewCurrent {
                path,
                name,
                package,
                backend,
                force,
            } => profile::create_profile_from_current_screen(
                input_executor,
                path,
                name,
                package,
                backend,
                force,
            ),
            ProfileCommand::Scale {
                input_path,
                output_path,
                width,
                height,
                force,
            } => crate::scaling::scale_profile_file(
                input_path,
                output_path,
                Resolution { width, height },
                force,
            ),
            ProfileCommand::ScaleCurrent {
                input_path,
                output_path,
                backend,
                force,
            } => crate::scaling::scale_profile_file_to_current_screen(
                input_executor,
                input_path,
                output_path,
                backend,
                force,
            ),
            ProfileCommand::AddTap {
                path,
                name,
                key,
                x,
                y,
            } => profile::add_tap_binding(path, name, key, x, y),
            ProfileCommand::AddSwipe {
                path,
                name,
                key,
                from,
                to,
                duration_ms,
            } => profile::add_swipe_binding(path, name, key, from, to, duration_ms),
            ProfileCommand::RemoveBinding { path, binding_name } => {
                profile::remove_binding(path, &binding_name)
            }
            ProfileCommand::ListBindings { profile_path } => profile::list_bindings(profile_path),
            ProfileCommand::Import { path, id, force } => profile::import_profile(path, id, force),
            ProfileCommand::RegistryNewCurrent {
                name,
                package,
                backend,
                id,
                force,
            } => profile::registry_create_profile_from_current_screen(
                input_executor,
                name,
                package,
                backend,
                id,
                force,
            ),
            ProfileCommand::List => profile::list_profiles(),
            ProfileCommand::Path { profile_id } => profile::print_profile_path(&profile_id),
            ProfileCommand::Show { profile_id } => profile::show_profile(&profile_id),
        },
        Commands::Device { command } => match command {
            DeviceCommand::Screen { backend } => device::device_screen(input_executor, backend),
            DeviceCommand::Density { backend } => device::device_density(input_executor, backend),
            DeviceCommand::Info { backend } => device::device_info(input_executor, backend),
        },
        Commands::Input { command } => match command {
            InputCommand::Tap { x, y, backend } => input::input_tap(input_executor, backend, x, y),
            InputCommand::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration_ms,
                backend,
            } => input::input_swipe(input_executor, backend, x1, y1, x2, y2, duration_ms),
            InputCommand::Keyevent { code, backend } => {
                input::input_keyevent(input_executor, backend, code)
            }
        },
        Commands::App { command } => match command {
            AppCommand::List { backend } => app::app_list(input_executor, backend),
            AppCommand::Launch {
                package_name,
                backend,
            } => app::app_launch(input_executor, backend, &package_name),
            AppCommand::InstallApk { path, backend } => {
                app::app_install_apk(input_executor, backend, path)
            }
            AppCommand::Current { backend } => app::app_current(input_executor, backend),
        },
        Commands::Binding { command } => match command {
            BindingCommand::Run {
                profile_path,
                binding_name,
                backend,
                scale_to_current,
            } => input::run_binding(
                input_executor,
                profile_path,
                &binding_name,
                backend,
                scale_to_current,
            ),
        },
        Commands::Play {
            profile_path,
            backend,
            scale_to_current,
        } => run::play(input_executor, profile_path, backend, scale_to_current),
        Commands::Run {
            profile_path,
            backend,
            launch_delay_ms,
            no_launch,
            scale_to_current,
        } => run::run(
            input_executor,
            profile_path,
            backend,
            run::RunOptions {
                launch_delay_ms,
                no_launch,
                scale_to_current,
            },
        ),
        Commands::RunProfile {
            profile_id,
            backend,
            launch_delay_ms,
            no_launch,
            scale_to_current,
        } => run::run_profile(
            input_executor,
            &profile_id,
            backend,
            run::RunOptions {
                launch_delay_ms,
                no_launch,
                scale_to_current,
            },
        ),
    }
}

fn doctor() -> Result<()> {
    println!("adb: {}", availability(wroid_adb::is_available()));
    println!("waydroid: {}", availability(wroid_waydroid::is_available()));

    if wroid_adb::is_available() {
        let devices = wroid_adb::devices().context("failed to list adb devices")?;
        println!("adb devices: {}", devices.len());
        for device in devices {
            println!("  {} {}", device.serial, device.state);
        }
    }

    if wroid_waydroid::is_available() {
        let status = wroid_waydroid::status().context("failed to read waydroid status")?;
        println!("waydroid status:");
        println!("{status}");
    }

    Ok(())
}

fn availability(is_available: bool) -> &'static str {
    if is_available {
        "available"
    } else {
        "missing"
    }
}
