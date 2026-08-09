use std::fmt;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

pub(crate) const DEFAULT_LAUNCH_DELAY_MS: u64 = 1500;

#[derive(Debug, Parser)]
#[command(name = "wroid", version, about = "Wroid Gaming Hub CLI")]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Commands,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Commands {
    /// Install, inspect, or remove the user-level desktop application.
    Desktop {
        #[command(subcommand)]
        command: DesktopCommand,
    },
    /// Install or inspect the minimal root-owned input bridge helper.
    Helper {
        #[command(subcommand)]
        command: HelperCommand,
    },
    /// Inspect host and Waydroid graphics performance readiness.
    Performance {
        #[arg(long)]
        json: bool,
        /// Configure Waydroid to use the GPU driving the active host renderer.
        #[arg(long, conflicts_with = "json")]
        setup_gpu: bool,
    },
    /// Inspect Android ABI, ARM translation, Play Store, and popular-game readiness.
    Compatibility {
        #[arg(long)]
        json: bool,
        /// Open or install a supported graphical Waydroid extension manager.
        #[arg(long, conflicts_with = "json")]
        setup: bool,
    },
    /// Internal interactive installer for Waydroid Helper.
    #[command(hide = true)]
    SetupWaydroidHelper {
        #[arg(long)]
        installer: String,
    },
    /// Internal detached graphical installer for the production bridge helper.
    #[command(hide = true)]
    InstallHelperGraphical,
    /// Internal detached worker for a Hub-staged APK ticket.
    #[command(hide = true)]
    InstallApkWorker {
        #[arg(long)]
        ticket: String,
    },
    /// Internal interactive Waydroid GPU setup.
    #[command(hide = true)]
    SetupWaydroidGpu {
        #[arg(long)]
        device: PathBuf,
    },
    /// Internal privileged Waydroid GPU config writer.
    #[command(hide = true)]
    ConfigureWaydroidGpu {
        #[arg(long)]
        device: PathBuf,
    },
    /// Open the Wroid desktop gaming hub in the default browser.
    Hub {
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
        #[arg(long)]
        profiles_dir: Option<PathBuf>,
    },
    Doctor {
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
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
    Session {
        #[command(subcommand)]
        command: SessionCommand,
    },
    /// Start or inspect the private per-user Wroid runtime daemon.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Play {
        profile_path: PathBuf,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long)]
        scale_to_current: bool,
    },
    /// Run a low-latency profile v2 session through evdev and persistent uinput.
    PlayV2 {
        profile_path: PathBuf,
        #[arg(long)]
        keyboard: Option<PathBuf>,
        #[arg(long)]
        mouse: Option<PathBuf>,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
        #[arg(long)]
        no_grab: bool,
        #[arg(long)]
        no_ui: bool,
        #[arg(long)]
        no_launch: bool,
        #[arg(long)]
        trace_input: bool,
        #[arg(long, hide = true)]
        exit_after_ms: Option<u64>,
        /// Internal desktop focus relay used by launch-v2.
        #[arg(long, hide = true)]
        focus_socket: Option<PathBuf>,
    },
    /// Stop desktop Waydroid and launch play-v2 with only the bridge helper elevated.
    LaunchV2 {
        profile_path: PathBuf,
        #[arg(long)]
        keyboard: Option<PathBuf>,
        #[arg(long)]
        mouse: Option<PathBuf>,
        #[arg(long, default_value_t = 1600)]
        width: u32,
        #[arg(long, default_value_t = 900)]
        height: u32,
        #[arg(long)]
        no_grab: bool,
        #[arg(long)]
        no_ui: bool,
        #[arg(long)]
        no_launch: bool,
        #[arg(long)]
        trace_input: bool,
        /// Stop automatically after the live diagnostic interval.
        #[arg(
            long,
            value_parser = clap::value_parser!(u64).range(1..=3600),
            conflicts_with = "exit_after_ms"
        )]
        exit_after_seconds: Option<u64>,
        #[arg(
            long,
            hide = true,
            requires_all = ["bridge_fd", "daemon_parent_pid"]
        )]
        daemon_worker: bool,
        #[arg(
            long,
            hide = true,
            value_parser = clap::value_parser!(i32).range(3..=1024),
            requires = "daemon_worker"
        )]
        bridge_fd: Option<i32>,
        #[arg(
            long,
            hide = true,
            value_parser = clap::value_parser!(u32).range(1..),
            requires = "daemon_worker"
        )]
        daemon_parent_pid: Option<u32>,
        #[arg(
            long,
            hide = true,
            value_parser = clap::value_parser!(u64).range(1..=3_600_000),
            requires = "daemon_worker",
            conflicts_with = "exit_after_seconds"
        )]
        exit_after_ms: Option<u64>,
    },
    /// Internal crash-recovery watchdog for launch-v2.
    #[command(hide = true)]
    RestoreDesktopSession {
        #[arg(long)]
        parent_pid: u32,
        #[arg(long)]
        ticket: String,
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
pub(crate) enum DesktopCommand {
    /// Install the current Wroid binary and application-menu entry.
    Install,
    /// Show user-level desktop installation paths and state.
    Status,
    /// Remove the Wroid binary and application-menu entry, preserving profiles.
    Uninstall,
}

#[derive(Debug, Subcommand)]
pub(crate) enum HelperCommand {
    /// Install the staged helper once as a root-owned executable.
    Install,
    /// Verify helper ownership, permissions, and production readiness.
    Status,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProfileCommand {
    Validate {
        path: PathBuf,
    },
    /// Open the local visual editor for a profile v2 JSON file.
    EditV2 {
        path: PathBuf,
        #[arg(long, default_value_t = 0)]
        port: u16,
        #[arg(long)]
        no_open: bool,
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
    AddJoystick {
        path: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        up: String,
        #[arg(long)]
        left: String,
        #[arg(long)]
        down: String,
        #[arg(long)]
        right: String,
        #[arg(long)]
        center: String,
        #[arg(long)]
        radius: u32,
        #[arg(long, default_value_t = 80)]
        tick_ms: u64,
        #[arg(long, default_value_t = 70)]
        swipe_duration_ms: u64,
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
    Export {
        profile_id: String,
        output_path: PathBuf,
        #[arg(long)]
        force: bool,
    },
    Remove {
        profile_id: String,
    },
    Rename {
        old_id: String,
        new_id: String,
    },
    Duplicate {
        source_id: String,
        target_id: String,
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
        #[arg(long)]
        allow_any_extension: bool,
        /// Override a confirmed ABI incompatibility after inspection.
        #[arg(long)]
        force_incompatible: bool,
    },
    /// Inspect Android package format, native ABIs, and Waydroid compatibility.
    Inspect {
        path: PathBuf,
        #[arg(long)]
        json: bool,
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

#[derive(Debug, Subcommand)]
pub(crate) enum SessionCommand {
    /// Prepare a daemon-managed runtime control plan from a profile v2 document.
    ///
    /// This sends the profile through protocol v1, materializes a runtime
    /// control plan against the Android surface, and records the prepared
    /// session. No input capture or injection is started yet.
    PrepareV2 {
        profile_path: PathBuf,
        #[arg(long)]
        width: u32,
        #[arg(long)]
        height: u32,
        #[arg(long, default_value = "cli-session")]
        session_id: String,
        #[arg(long)]
        no_launch: bool,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub(crate) enum DaemonCommand {
    /// Start wroidd for the current desktop user if it is not running.
    Start,
    /// Verify the private daemon socket and protocol version.
    Status,
    /// List sessions currently owned by wroidd.
    Sessions,
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
    fn doctor_accepts_backend_option() {
        let cli = Cli::try_parse_from(["wroid", "doctor", "--backend", "waydroid-shell"]).unwrap();

        let Commands::Doctor { backend } = cli.command else {
            panic!("expected doctor command");
        };

        assert_eq!(backend, InputBackend::WaydroidShell);
    }

    #[test]
    fn daemon_lifecycle_commands_parse() {
        for (argument, expected) in [
            ("start", DaemonCommand::Start),
            ("status", DaemonCommand::Status),
            ("sessions", DaemonCommand::Sessions),
        ] {
            let cli = Cli::try_parse_from(["wroid", "daemon", argument]).unwrap();
            let Commands::Daemon { command } = cli.command else {
                panic!("expected daemon command");
            };
            assert_eq!(command, expected);
        }
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
    fn profile_add_joystick_uses_default_timing() {
        let cli = Cli::try_parse_from([
            "wroid",
            "profile",
            "add-joystick",
            "profiles/my-game.json",
            "--name",
            "movement",
            "--up",
            "W",
            "--left",
            "A",
            "--down",
            "S",
            "--right",
            "D",
            "--center",
            "320,780",
            "--radius",
            "120",
        ])
        .unwrap();

        let Commands::Profile { command } = cli.command else {
            panic!("expected profile command");
        };
        let ProfileCommand::AddJoystick {
            path,
            name,
            up,
            left,
            down,
            right,
            center,
            radius,
            tick_ms,
            swipe_duration_ms,
        } = command
        else {
            panic!("expected add-joystick command");
        };

        assert_eq!(path, PathBuf::from("profiles/my-game.json"));
        assert_eq!(name, "movement");
        assert_eq!(up, "W");
        assert_eq!(left, "A");
        assert_eq!(down, "S");
        assert_eq!(right, "D");
        assert_eq!(center, "320,780");
        assert_eq!(radius, 120);
        assert_eq!(tick_ms, 80);
        assert_eq!(swipe_duration_ms, 70);
    }

    #[test]
    fn app_install_apk_accepts_allow_any_extension() {
        let cli = Cli::try_parse_from([
            "wroid",
            "app",
            "install-apk",
            "downloads/game.bin",
            "--backend",
            "waydroid-shell",
            "--allow-any-extension",
        ])
        .unwrap();

        let Commands::App { command } = cli.command else {
            panic!("expected app command");
        };
        let AppCommand::InstallApk {
            path,
            backend,
            allow_any_extension,
            force_incompatible,
        } = command
        else {
            panic!("expected install-apk command");
        };

        assert_eq!(path, PathBuf::from("downloads/game.bin"));
        assert_eq!(backend, InputBackend::WaydroidShell);
        assert!(allow_any_extension);
        assert!(!force_incompatible);
    }

    #[test]
    fn app_inspect_accepts_json_output() {
        let cli = Cli::try_parse_from(["wroid", "app", "inspect", "downloads/game.xapk", "--json"])
            .unwrap();

        let Commands::App {
            command: AppCommand::Inspect { path, json },
        } = cli.command
        else {
            panic!("expected app inspect command");
        };
        assert_eq!(path, PathBuf::from("downloads/game.xapk"));
        assert!(json);
    }

    #[test]
    fn session_prepare_v2_parses_resolution_and_defaults() {
        let cli = Cli::try_parse_from([
            "wroid",
            "session",
            "prepare-v2",
            "profiles/examples/movement-v2.json",
            "--width",
            "1920",
            "--height",
            "1080",
        ])
        .unwrap();

        let Commands::Session { command } = cli.command else {
            panic!("expected session command");
        };
        let SessionCommand::PrepareV2 {
            profile_path,
            width,
            height,
            session_id,
            no_launch,
        } = command;

        assert_eq!(
            profile_path,
            PathBuf::from("profiles/examples/movement-v2.json")
        );
        assert_eq!(width, 1920);
        assert_eq!(height, 1080);
        assert_eq!(session_id, "cli-session");
        assert!(!no_launch);
    }

    #[test]
    fn session_prepare_v2_accepts_session_id_and_no_launch() {
        let cli = Cli::try_parse_from([
            "wroid",
            "session",
            "prepare-v2",
            "profiles/examples/movement-v2.json",
            "--width",
            "1280",
            "--height",
            "720",
            "--session-id",
            "shooter",
            "--no-launch",
        ])
        .unwrap();

        let Commands::Session { command } = cli.command else {
            panic!("expected session command");
        };
        let SessionCommand::PrepareV2 {
            session_id,
            no_launch,
            ..
        } = command;

        assert_eq!(session_id, "shooter");
        assert!(no_launch);
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

    #[test]
    fn play_v2_parses_devices_resolution_and_safety_flags() {
        let cli = Cli::try_parse_from([
            "wroid",
            "play-v2",
            "profiles/examples/brawlstars-v2.json",
            "--keyboard",
            "/dev/input/by-id/keyboard-event-kbd",
            "--mouse",
            "/dev/input/by-id/mouse-event-mouse",
            "--width",
            "1600",
            "--height",
            "900",
            "--no-grab",
            "--no-launch",
            "--trace-input",
        ])
        .unwrap();

        let Commands::PlayV2 {
            profile_path,
            keyboard,
            mouse,
            width,
            height,
            no_grab,
            no_launch,
            trace_input,
            exit_after_ms,
            ..
        } = cli.command
        else {
            panic!("expected play-v2 command");
        };

        assert_eq!(
            profile_path,
            PathBuf::from("profiles/examples/brawlstars-v2.json")
        );
        assert_eq!(
            keyboard,
            Some(PathBuf::from("/dev/input/by-id/keyboard-event-kbd"))
        );
        assert_eq!(
            mouse,
            Some(PathBuf::from("/dev/input/by-id/mouse-event-mouse"))
        );
        assert_eq!((width, height), (1600, 900));
        assert!(no_grab);
        assert!(no_launch);
        assert!(trace_input);
        assert!(exit_after_ms.is_none());
    }

    #[test]
    fn profile_edit_v2_parses_local_server_options() {
        let cli = Cli::try_parse_from([
            "wroid",
            "profile",
            "edit-v2",
            "profiles/examples/brawlstars-v2.json",
            "--port",
            "9876",
            "--no-open",
        ])
        .unwrap();

        let Commands::Profile { command } = cli.command else {
            panic!("expected profile command");
        };
        let ProfileCommand::EditV2 {
            path,
            port,
            no_open,
        } = command
        else {
            panic!("expected edit-v2 command");
        };

        assert_eq!(path, PathBuf::from("profiles/examples/brawlstars-v2.json"));
        assert_eq!(port, 9876);
        assert!(no_open);
    }

    #[test]
    fn hub_parses_local_server_and_library_options() {
        let cli = Cli::try_parse_from([
            "wroid",
            "hub",
            "--port",
            "9001",
            "--no-open",
            "--profiles-dir",
            "/tmp/wroid-games",
        ])
        .unwrap();

        let Commands::Hub {
            port,
            no_open,
            profiles_dir,
        } = cli.command
        else {
            panic!("expected hub command");
        };

        assert_eq!(port, 9001);
        assert!(no_open);
        assert_eq!(profiles_dir, Some(PathBuf::from("/tmp/wroid-games")));
    }

    #[test]
    fn launch_v2_uses_balanced_resolution_by_default() {
        let cli =
            Cli::try_parse_from(["wroid", "launch-v2", "profiles/examples/pubg-v2.json"]).unwrap();

        let Commands::LaunchV2 {
            profile_path,
            width,
            height,
            ..
        } = cli.command
        else {
            panic!("expected launch-v2 command");
        };

        assert_eq!(
            profile_path,
            PathBuf::from("profiles/examples/pubg-v2.json")
        );
        assert_eq!((width, height), (1600, 900));
    }

    #[test]
    fn launch_v2_accepts_bounded_diagnostic_timeout() {
        let cli = Cli::try_parse_from([
            "wroid",
            "launch-v2",
            "profiles/examples/pubg-v2.json",
            "--no-launch",
            "--trace-input",
            "--exit-after-seconds",
            "20",
        ])
        .unwrap();

        let Commands::LaunchV2 {
            no_launch,
            trace_input,
            exit_after_seconds,
            ..
        } = cli.command
        else {
            panic!("expected launch-v2 command");
        };
        assert!(no_launch);
        assert!(trace_input);
        assert_eq!(exit_after_seconds, Some(20));
        assert!(Cli::try_parse_from([
            "wroid",
            "launch-v2",
            "profiles/examples/pubg-v2.json",
            "--exit-after-seconds",
            "0",
        ])
        .is_err());
    }

    #[test]
    fn launch_v2_daemon_worker_requires_complete_private_invocation() {
        for incomplete in [
            vec!["--daemon-worker"],
            vec!["--daemon-worker", "--bridge-fd", "198"],
            vec!["--bridge-fd", "198", "--daemon-parent-pid", "42"],
        ] {
            let mut arguments = vec!["wroid", "launch-v2", "profiles/examples/pubg-v2.json"];
            arguments.extend(incomplete);
            assert!(Cli::try_parse_from(arguments).is_err());
        }

        let cli = Cli::try_parse_from([
            "wroid",
            "launch-v2",
            "profiles/examples/pubg-v2.json",
            "--daemon-worker",
            "--bridge-fd",
            "198",
            "--daemon-parent-pid",
            "42",
            "--exit-after-ms",
            "25",
        ])
        .unwrap();
        let Commands::LaunchV2 {
            daemon_worker,
            bridge_fd,
            daemon_parent_pid,
            exit_after_ms,
            ..
        } = cli.command
        else {
            panic!("expected launch-v2 command");
        };
        assert!(daemon_worker);
        assert_eq!(bridge_fd, Some(198));
        assert_eq!(daemon_parent_pid, Some(42));
        assert_eq!(exit_after_ms, Some(25));
    }

    #[test]
    fn launch_v2_public_timeout_cannot_mix_with_worker_timeout() {
        assert!(Cli::try_parse_from([
            "wroid",
            "launch-v2",
            "profiles/examples/pubg-v2.json",
            "--exit-after-seconds",
            "20",
            "--exit-after-ms",
            "25",
        ])
        .is_err());
    }

    #[test]
    fn hidden_desktop_restore_watchdog_command_parses() {
        let cli = Cli::try_parse_from([
            "wroid",
            "restore-desktop-session",
            "--parent-pid",
            "42",
            "--ticket",
            "0123456789abcdef0123456789abcdef",
        ])
        .unwrap();
        let Commands::RestoreDesktopSession { parent_pid, ticket } = cli.command else {
            panic!("expected restore-desktop-session command");
        };
        assert_eq!(parent_pid, 42);
        assert_eq!(ticket, "0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn hidden_apk_install_worker_command_parses_ticket_only() {
        let ticket = "0123456789abcdef0123456789abcdef0123456789abcdef";
        let cli = Cli::try_parse_from(["wroid", "install-apk-worker", "--ticket", ticket]).unwrap();
        let Commands::InstallApkWorker { ticket: parsed } = cli.command else {
            panic!("expected install-apk-worker command");
        };
        assert_eq!(parsed, ticket);
    }

    #[test]
    fn desktop_install_command_parses() {
        let cli = Cli::try_parse_from(["wroid", "desktop", "install"]).unwrap();
        let Commands::Desktop { command } = cli.command else {
            panic!("expected desktop command");
        };
        assert!(matches!(command, DesktopCommand::Install));
    }

    #[test]
    fn helper_install_and_status_commands_parse() {
        for (subcommand, expected) in [
            ("install", HelperCommand::Install),
            ("status", HelperCommand::Status),
        ] {
            let cli = Cli::try_parse_from(["wroid", "helper", subcommand]).unwrap();
            let Commands::Helper { command } = cli.command else {
                panic!("expected helper command");
            };
            assert_eq!(
                std::mem::discriminant(&command),
                std::mem::discriminant(&expected)
            );
        }
    }

    #[test]
    fn performance_command_accepts_json_output() {
        let cli = Cli::try_parse_from(["wroid", "performance", "--json"]).unwrap();
        let Commands::Performance { json, setup_gpu } = cli.command else {
            panic!("expected performance command");
        };
        assert!(json);
        assert!(!setup_gpu);
    }

    #[test]
    fn performance_command_accepts_gpu_setup() {
        let cli = Cli::try_parse_from(["wroid", "performance", "--setup-gpu"]).unwrap();
        let Commands::Performance { json, setup_gpu } = cli.command else {
            panic!("expected performance command");
        };
        assert!(!json);
        assert!(setup_gpu);
    }

    #[test]
    fn compatibility_command_accepts_json_output() {
        let cli = Cli::try_parse_from(["wroid", "compatibility", "--json"]).unwrap();
        let Commands::Compatibility { json, setup } = cli.command else {
            panic!("expected compatibility command");
        };
        assert!(json);
        assert!(!setup);
    }

    #[test]
    fn compatibility_command_accepts_setup_action() {
        let cli = Cli::try_parse_from(["wroid", "compatibility", "--setup"]).unwrap();
        let Commands::Compatibility { json, setup } = cli.command else {
            panic!("expected compatibility command");
        };
        assert!(!json);
        assert!(setup);
    }

    #[test]
    fn hidden_waydroid_helper_installer_command_parses() {
        let cli =
            Cli::try_parse_from(["wroid", "setup-waydroid-helper", "--installer", "yay"]).unwrap();
        let Commands::SetupWaydroidHelper { installer } = cli.command else {
            panic!("expected setup-waydroid-helper command");
        };
        assert_eq!(installer, "yay");
    }

    #[test]
    fn hidden_graphical_bridge_helper_installer_command_parses() {
        let cli = Cli::try_parse_from(["wroid", "install-helper-graphical"]).unwrap();
        assert!(matches!(cli.command, Commands::InstallHelperGraphical));
    }
}
