use std::fmt;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

pub(crate) const DEFAULT_LAUNCH_DELAY_MS: u64 = 1500;

#[derive(Debug, Parser)]
#[command(name = "wroid", about = "Wroid Gaming Hub CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    Doctor,
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    Device {
        #[command(subcommand)]
        command: DeviceCommand,
    },
    Input {
        #[command(subcommand)]
        command: InputCommand,
    },
    App {
        #[command(subcommand)]
        command: AppCommand,
    },
    Binding {
        #[command(subcommand)]
        command: BindingCommand,
    },
    Play {
        profile_path: PathBuf,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long)]
        scale_to_current: bool,
    },
    Run {
        profile_path: PathBuf,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long, default_value_t = DEFAULT_LAUNCH_DELAY_MS)]
        launch_delay_ms: u64,
        #[arg(long)]
        no_launch: bool,
        #[arg(long)]
        scale_to_current: bool,
    },
    RunProfile {
        profile_id: String,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long, default_value_t = DEFAULT_LAUNCH_DELAY_MS)]
        launch_delay_ms: u64,
        #[arg(long)]
        no_launch: bool,
        #[arg(long)]
        scale_to_current: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
    Validate {
        path: PathBuf,
    },
    Example {
        path: PathBuf,
    },
    New {
        path: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        package: String,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long)]
        force: bool,
    },
    NewCurrent {
        path: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        package: String,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long)]
        force: bool,
    },
    Scale {
        input_path: PathBuf,
        output_path: PathBuf,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long)]
        force: bool,
    },
    ScaleCurrent {
        input_path: PathBuf,
        output_path: PathBuf,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long)]
        force: bool,
    },
    AddTap {
        path: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        x: u32,
        #[arg(long)]
        y: u32,
    },
    AddSwipe {
        path: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        key: String,
        #[arg(long = "from")]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        duration_ms: u64,
    },
    RemoveBinding {
        path: PathBuf,
        binding_name: String,
    },
    ListBindings {
        profile_path: PathBuf,
    },
    Import {
        path: PathBuf,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        force: bool,
    },
    RegistryNewCurrent {
        #[arg(long)]
        name: String,
        #[arg(long)]
        package: String,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        force: bool,
    },
    List,
    Path {
        profile_id: String,
    },
    Show {
        profile_id: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum DeviceCommand {
    Screen {
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    Density {
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    Info {
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum InputCommand {
    Tap {
        x: u32,
        y: u32,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    Swipe {
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    Keyevent {
        code: u32,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AppCommand {
    List {
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    Launch {
        package_name: String,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    InstallApk {
        path: PathBuf,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
    Current {
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum BindingCommand {
    Run {
        profile_path: PathBuf,
        binding_name: String,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long)]
        scale_to_current: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum InputBackend {
    Adb,
    #[value(name = "waydroid-shell")]
    WaydroidShell,
    Auto,
}

impl fmt::Display for InputBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adb => write!(f, "adb"),
            Self::WaydroidShell => write!(f, "waydroid-shell"),
            Self::Auto => write!(f, "auto"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use clap::Parser;

    use super::*;

    #[test]
    fn run_command_default_launch_delay_is_1500() {
        let cli = Cli::try_parse_from(["wroid", "run", "profiles/my-game.json"]).unwrap();

        let Commands::Run {
            profile_path,
            backend,
            launch_delay_ms,
            no_launch,
            scale_to_current,
        } = cli.command
        else {
            panic!("expected run command");
        };

        assert_eq!(profile_path, PathBuf::from("profiles/my-game.json"));
        assert_eq!(backend, InputBackend::Auto);
        assert_eq!(launch_delay_ms, DEFAULT_LAUNCH_DELAY_MS);
        assert!(!no_launch);
        assert!(!scale_to_current);
    }

    #[test]
    fn run_command_accepts_explicit_launch_delay() {
        let cli = Cli::try_parse_from([
            "wroid",
            "run",
            "profiles/my-game.json",
            "--backend",
            "waydroid-shell",
            "--launch-delay-ms",
            "2500",
        ])
        .unwrap();

        let Commands::Run {
            backend,
            launch_delay_ms,
            no_launch,
            ..
        } = cli.command
        else {
            panic!("expected run command");
        };

        assert_eq!(backend, InputBackend::WaydroidShell);
        assert_eq!(launch_delay_ms, 2500);
        assert!(!no_launch);
    }

    #[test]
    fn run_command_accepts_no_launch() {
        let cli = Cli::try_parse_from([
            "wroid",
            "run",
            "profiles/my-game.json",
            "--backend",
            "waydroid-shell",
            "--no-launch",
        ])
        .unwrap();

        let Commands::Run {
            backend, no_launch, ..
        } = cli.command
        else {
            panic!("expected run command");
        };

        assert_eq!(backend, InputBackend::WaydroidShell);
        assert!(no_launch);
    }

    #[test]
    fn run_profile_command_accepts_no_launch() {
        let cli = Cli::try_parse_from([
            "wroid",
            "run-profile",
            "com.android.settings",
            "--backend",
            "waydroid-shell",
            "--no-launch",
        ])
        .unwrap();

        let Commands::RunProfile {
            profile_id,
            backend,
            no_launch,
            ..
        } = cli.command
        else {
            panic!("expected run-profile command");
        };

        assert_eq!(profile_id, "com.android.settings");
        assert_eq!(backend, InputBackend::WaydroidShell);
        assert!(no_launch);
    }

    #[test]
    fn binding_run_accepts_scale_to_current() {
        let cli = Cli::try_parse_from([
            "wroid",
            "binding",
            "run",
            "profiles/my-game.json",
            "fire",
            "--backend",
            "waydroid-shell",
            "--scale-to-current",
        ])
        .unwrap();

        let Commands::Binding { command } = cli.command else {
            panic!("expected binding command");
        };
        let BindingCommand::Run {
            profile_path,
            binding_name,
            backend,
            scale_to_current,
        } = command;

        assert_eq!(profile_path, PathBuf::from("profiles/my-game.json"));
        assert_eq!(binding_name, "fire");
        assert_eq!(backend, InputBackend::WaydroidShell);
        assert!(scale_to_current);
    }

    #[test]
    fn run_profile_accepts_scale_to_current() {
        let cli = Cli::try_parse_from([
            "wroid",
            "run-profile",
            "com.android.settings",
            "--backend",
            "waydroid-shell",
            "--scale-to-current",
        ])
        .unwrap();

        let Commands::RunProfile {
            profile_id,
            backend,
            scale_to_current,
            ..
        } = cli.command
        else {
            panic!("expected run-profile command");
        };

        assert_eq!(profile_id, "com.android.settings");
        assert_eq!(backend, InputBackend::WaydroidShell);
        assert!(scale_to_current);
    }
}
