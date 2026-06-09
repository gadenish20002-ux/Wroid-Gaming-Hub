use std::fmt;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
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
}

#[derive(Debug, Subcommand)]
enum BindingCommand {
    Run {
        profile_path: PathBuf,
        binding_name: String,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum InputBackend {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectedInputBackend {
    Adb,
    WaydroidShell,
}

trait InputExecutor {
    fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>>;
    fn adb_tap(&self, x: u32, y: u32) -> Result<()>;
    fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()>;
    fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()>;
    fn waydroid_shell_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
struct CommandInputExecutor;

impl InputExecutor for CommandInputExecutor {
    fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>> {
        wroid_adb::devices()
    }

    fn adb_tap(&self, x: u32, y: u32) -> Result<()> {
        wroid_adb::tap(x, y)
    }

    fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
        wroid_adb::swipe(x1, y1, x2, y2, duration_ms)
    }

    fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()> {
        wroid_waydroid::shell_input_tap(x, y)
    }

    fn waydroid_shell_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()> {
        wroid_waydroid::shell_input_swipe(x1, y1, x2, y2, duration_ms)
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let input_executor = CommandInputExecutor;

    match cli.command {
        Commands::Doctor => doctor(),
        Commands::Profile { command } => match command {
            ProfileCommand::Validate { path } => validate_profile(path),
            ProfileCommand::Example { path } => write_example_profile(path),
        },
        Commands::Input { command } => match command {
            InputCommand::Tap { x, y, backend } => input_tap(&input_executor, backend, x, y),
            InputCommand::Swipe {
                x1,
                y1,
                x2,
                y2,
                duration_ms,
                backend,
            } => input_swipe(&input_executor, backend, x1, y1, x2, y2, duration_ms),
        },
        Commands::Binding { command } => match command {
            BindingCommand::Run {
                profile_path,
                binding_name,
                backend,
            } => run_binding(&input_executor, profile_path, &binding_name, backend),
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

fn input_tap(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    x: u32,
    y: u32,
) -> Result<()> {
    match select_input_backend(input_executor, backend) {
        SelectedInputBackend::Adb => input_executor.adb_tap(x, y),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_tap(x, y),
    }
}

fn input_swipe(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration_ms: u64,
) -> Result<()> {
    match select_input_backend(input_executor, backend) {
        SelectedInputBackend::Adb => input_executor.adb_swipe(x1, y1, x2, y2, duration_ms),
        SelectedInputBackend::WaydroidShell => {
            input_executor.waydroid_shell_swipe(x1, y1, x2, y2, duration_ms)
        }
    }
}

fn select_input_backend(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> SelectedInputBackend {
    match backend {
        InputBackend::Adb => SelectedInputBackend::Adb,
        InputBackend::WaydroidShell => SelectedInputBackend::WaydroidShell,
        InputBackend::Auto => {
            let has_adb_device = input_executor
                .adb_devices()
                .unwrap_or_default()
                .iter()
                .any(|device| device.state == "device");

            if has_adb_device {
                SelectedInputBackend::Adb
            } else {
                SelectedInputBackend::WaydroidShell
            }
        }
    }
}

fn run_binding(
    input_executor: &impl InputExecutor,
    profile_path: PathBuf,
    binding_name: &str,
    backend: InputBackend,
) -> Result<()> {
    let profile = ControlProfile::load_from_path(&profile_path)
        .with_context(|| format!("failed to load profile {}", profile_path.display()))?;
    profile
        .validate()
        .with_context(|| format!("profile {} is invalid", profile_path.display()))?;

    let binding = profile
        .binding(binding_name)
        .with_context(|| format!("binding {binding_name} not found"))?;

    match &binding.action {
        BindingAction::Tap { point } => input_tap(input_executor, backend, point.x, point.y),
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => input_swipe(
            input_executor,
            backend,
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

fn availability(is_available: bool) -> &'static str {
    if is_available {
        "available"
    } else {
        "missing"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use anyhow::{anyhow, Result};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum InputCall {
        AdbTap(u32, u32),
        AdbSwipe(u32, u32, u32, u32, u64),
        WaydroidShellTap(u32, u32),
        WaydroidShellSwipe(u32, u32, u32, u32, u64),
    }

    #[derive(Debug, Default)]
    struct FakeInputExecutor {
        devices: Vec<wroid_adb::AdbDevice>,
        fail_devices: bool,
        device_queries: Cell<usize>,
        calls: RefCell<Vec<InputCall>>,
    }

    impl FakeInputExecutor {
        fn with_devices(devices: Vec<wroid_adb::AdbDevice>) -> Self {
            Self {
                devices,
                ..Self::default()
            }
        }

        fn with_device_state(serial: &str, state: &str) -> Self {
            Self::with_devices(vec![wroid_adb::AdbDevice {
                serial: serial.to_owned(),
                state: state.to_owned(),
            }])
        }

        fn with_device_error() -> Self {
            Self {
                fail_devices: true,
                ..Self::default()
            }
        }

        fn calls(&self) -> Vec<InputCall> {
            self.calls.borrow().clone()
        }
    }

    impl InputExecutor for FakeInputExecutor {
        fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>> {
            self.device_queries.set(self.device_queries.get() + 1);
            if self.fail_devices {
                Err(anyhow!("adb devices failed"))
            } else {
                Ok(self.devices.clone())
            }
        }

        fn adb_tap(&self, x: u32, y: u32) -> Result<()> {
            self.calls.borrow_mut().push(InputCall::AdbTap(x, y));
            Ok(())
        }

        fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::AdbSwipe(x1, y1, x2, y2, duration_ms));
            Ok(())
        }

        fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::WaydroidShellTap(x, y));
            Ok(())
        }

        fn waydroid_shell_swipe(
            &self,
            x1: u32,
            y1: u32,
            x2: u32,
            y2: u32,
            duration_ms: u64,
        ) -> Result<()> {
            self.calls.borrow_mut().push(InputCall::WaydroidShellSwipe(
                x1,
                y1,
                x2,
                y2,
                duration_ms,
            ));
            Ok(())
        }
    }

    #[test]
    fn auto_selects_adb_when_connected_device_exists() {
        let executor = FakeInputExecutor::with_devices(vec![
            wroid_adb::AdbDevice {
                serial: "offline-device".to_owned(),
                state: "offline".to_owned(),
            },
            wroid_adb::AdbDevice {
                serial: "connected-device".to_owned(),
                state: "device".to_owned(),
            },
        ]);

        let selected = select_input_backend(&executor, InputBackend::Auto);

        assert_eq!(selected, SelectedInputBackend::Adb);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn auto_selects_waydroid_shell_when_adb_has_no_connected_device() {
        let executor = FakeInputExecutor::with_device_state("offline-device", "offline");

        let selected = select_input_backend(&executor, InputBackend::Auto);

        assert_eq!(selected, SelectedInputBackend::WaydroidShell);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn auto_selects_waydroid_shell_when_adb_device_listing_fails() {
        let executor = FakeInputExecutor::with_device_error();

        let selected = select_input_backend(&executor, InputBackend::Auto);

        assert_eq!(selected, SelectedInputBackend::WaydroidShell);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn explicit_backend_selection_does_not_query_adb_devices() {
        let executor = FakeInputExecutor::with_device_error();

        let selected = select_input_backend(&executor, InputBackend::Adb);

        assert_eq!(selected, SelectedInputBackend::Adb);
        assert_eq!(executor.device_queries.get(), 0);
    }

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
}
