use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use wroid_core::{BindingAction, ControlProfile};

#[derive(Debug, Parser)]
#[command(name = "wroid", about = "Wroid Gaming Hub CLI")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Doctor,
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    Input {
        #[command(subcommand)]
        command: InputCommand,
    },
    Binding {
        #[command(subcommand)]
        command: BindingCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    Validate { path: PathBuf },
    Example { path: PathBuf },
}

#[derive(Debug, Subcommand)]
enum InputCommand {
    Tap {
        x: u32,
        y: u32,
    },
    Swipe {
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    },
}

#[derive(Debug, Subcommand)]
enum BindingCommand {
    Run {
        profile_path: PathBuf,
        binding_name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Doctor => doctor(),
        Commands::Profile { command } => match command {
            ProfileCommand::Validate { path } => validate_profile(path),
            ProfileCommand::Example { path } => write_example_profile(path),
        },
        Commands::Input { command } => match command {
            InputCommand::Tap { x, y } => wroid_adb::tap(x, y),
            InputCommand::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration_ms,
            } => wroid_adb::swipe(x1, y1, x2, y2, duration_ms),
        },
        Commands::Binding { command } => match command {
            BindingCommand::Run {
                profile_path,
                binding_name,
            } => run_binding(profile_path, &binding_name),
        },
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

fn validate_profile(path: PathBuf) -> Result<()> {
    let profile = ControlProfile::load_from_path(&path)
        .with_context(|| format!("failed to load profile {}", path.display()))?;
    profile
        .validate()
        .with_context(|| format!("profile {} is invalid", path.display()))?;
    println!("valid profile: {}", path.display());
    Ok(())
}

fn write_example_profile(path: PathBuf) -> Result<()> {
    let profile = ControlProfile::example();
    profile
        .validate()
        .context("built-in example profile is invalid")?;
    profile
        .save_to_path(&path)
        .with_context(|| format!("failed to write example profile {}", path.display()))?;
    println!("wrote example profile: {}", path.display());
    Ok(())
}

fn run_binding(profile_path: PathBuf, binding_name: &str) -> Result<()> {
    let profile = ControlProfile::load_from_path(&profile_path)
        .with_context(|| format!("failed to load profile {}", profile_path.display()))?;
    profile
        .validate()
        .with_context(|| format!("profile {} is invalid", profile_path.display()))?;

    let binding = profile
        .binding(binding_name)
        .with_context(|| format!("binding {binding_name} not found"))?;

    match &binding.action {
        BindingAction::Tap { point } => wroid_adb::tap(point.x, point.y),
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => wroid_adb::swipe(from.x, from.y, to.x, to.y, *duration_ms),
        BindingAction::VirtualJoystick { .. } => bail!("virtual_joystick is not implemented"),
        BindingAction::MouseAim { .. } => bail!("mouse_aim is not implemented"),
        BindingAction::Macro { .. } => bail!("macro is not implemented"),
    }
}

fn availability(is_available: bool) -> &'static str {
    if is_available {
        "available"
    } else {
        "missing"
    }
}
