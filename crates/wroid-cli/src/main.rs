use std::env;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::terminal;
use wroid_core::{
    Binding, BindingAction, BindingInput, ControlProfile, Point, ProfileError, Resolution,
    ValidationError,
};

const DEFAULT_LAUNCH_DELAY_MS: u64 = 1500;

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
    },
    Run {
        profile_path: PathBuf,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
        #[arg(long, default_value_t = DEFAULT_LAUNCH_DELAY_MS)]
        launch_delay_ms: u64,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
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
    Keyevent {
        code: u32,
        #[arg(long, value_enum, default_value_t = InputBackend::Auto)]
        backend: InputBackend,
    },
}

#[derive(Debug, Subcommand)]
enum AppCommand {
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

impl fmt::Display for SelectedInputBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adb => write!(f, "adb"),
            Self::WaydroidShell => write!(f, "waydroid-shell"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CurrentAndroidActivity {
    package_name: String,
    activity_name: String,
    component_name: String,
}

trait InputExecutor {
    fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>>;
    fn adb_tap(&self, x: u32, y: u32) -> Result<()>;
    fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()>;
    fn adb_keyevent(&self, code: u32) -> Result<()>;
    fn adb_list_packages(&self) -> Result<Vec<String>>;
    fn adb_launch_package(&self, package_name: &str) -> Result<()>;
    fn adb_install_apk(&self, path: &Path) -> Result<()>;
    fn adb_current_activity(&self) -> Result<Option<CurrentAndroidActivity>>;
    fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()>;
    fn waydroid_shell_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()>;
    fn waydroid_shell_keyevent(&self, code: u32) -> Result<()>;
    fn waydroid_app_list_packages(&self) -> Result<Vec<String>>;
    fn waydroid_app_launch_package(&self, package_name: &str) -> Result<()>;
    fn waydroid_app_launch_package_as_user(
        &self,
        package_name: &str,
        user: &str,
        session_env: &wroid_waydroid::WaydroidAppLaunchEnv,
    ) -> Result<()>;
    fn waydroid_app_install(&self, path: &Path) -> Result<()>;
    fn waydroid_shell_current_activity(&self) -> Result<Option<CurrentAndroidActivity>>;
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

    fn adb_keyevent(&self, code: u32) -> Result<()> {
        wroid_adb::keyevent(code)
    }

    fn adb_list_packages(&self) -> Result<Vec<String>> {
        wroid_adb::list_packages()
    }

    fn adb_launch_package(&self, package_name: &str) -> Result<()> {
        wroid_adb::launch_package(package_name)
    }

    fn adb_install_apk(&self, path: &Path) -> Result<()> {
        wroid_adb::install_apk(path)
    }

    fn adb_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
        Ok(
            wroid_adb::current_activity()?.map(|activity| CurrentAndroidActivity {
                package_name: activity.package_name,
                activity_name: activity.activity_name,
                component_name: activity.component_name,
            }),
        )
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

    fn waydroid_shell_keyevent(&self, code: u32) -> Result<()> {
        wroid_waydroid::shell_input_keyevent(code)
    }

    fn waydroid_app_list_packages(&self) -> Result<Vec<String>> {
        wroid_waydroid::app_list_packages()
    }

    fn waydroid_app_launch_package(&self, package_name: &str) -> Result<()> {
        wroid_waydroid::app_launch_package(package_name)
    }

    fn waydroid_app_launch_package_as_user(
        &self,
        package_name: &str,
        user: &str,
        session_env: &wroid_waydroid::WaydroidAppLaunchEnv,
    ) -> Result<()> {
        wroid_waydroid::app_launch_package_as_user(package_name, user, session_env)
    }

    fn waydroid_app_install(&self, path: &Path) -> Result<()> {
        wroid_waydroid::app_install(path)
    }

    fn waydroid_shell_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
        Ok(
            wroid_waydroid::shell_current_activity()?.map(|activity| CurrentAndroidActivity {
                package_name: activity.package_name,
                activity_name: activity.activity_name,
                component_name: activity.component_name,
            }),
        )
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
            ProfileCommand::New {
                path,
                name,
                package,
                width,
                height,
                force,
            } => create_profile(path, name, package, width, height, force),
            ProfileCommand::AddTap {
                path,
                name,
                key,
                x,
                y,
            } => add_tap_binding(path, name, key, x, y),
            ProfileCommand::AddSwipe {
                path,
                name,
                key,
                from,
                to,
                duration_ms,
            } => add_swipe_binding(path, name, key, from, to, duration_ms),
            ProfileCommand::RemoveBinding { path, binding_name } => {
                remove_binding(path, &binding_name)
            }
            ProfileCommand::ListBindings { profile_path } => list_bindings(profile_path),
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
            InputCommand::Keyevent { code, backend } => {
                input_keyevent(&input_executor, backend, code)
            }
        },
        Commands::App { command } => match command {
            AppCommand::List { backend } => app_list(&input_executor, backend),
            AppCommand::Launch {
                package_name,
                backend,
            } => app_launch(&input_executor, backend, &package_name),
            AppCommand::InstallApk { path, backend } => {
                app_install_apk(&input_executor, backend, path)
            }
            AppCommand::Current { backend } => app_current(&input_executor, backend),
        },
        Commands::Binding { command } => match command {
            BindingCommand::Run {
                profile_path,
                binding_name,
                backend,
            } => run_binding(&input_executor, profile_path, &binding_name, backend),
        },
        Commands::Play {
            profile_path,
            backend,
        } => play(&input_executor, profile_path, backend),
        Commands::Run {
            profile_path,
            backend,
            launch_delay_ms,
        } => run(&input_executor, profile_path, backend, launch_delay_ms),
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
    load_validated_profile(&path)?;
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

fn create_profile(
    path: PathBuf,
    name: String,
    package_name: String,
    width: u32,
    height: u32,
    force: bool,
) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "profile {} already exists; pass --force to overwrite",
            path.display()
        );
    }

    let profile = ControlProfile {
        name,
        package_name,
        resolution: Resolution { width, height },
        bindings: Vec::new(),
    };
    profile.validate().context("new profile is invalid")?;
    save_profile(&profile, &path)?;
    println!("created profile: {}", path.display());
    Ok(())
}

fn add_tap_binding(path: PathBuf, name: String, key: String, x: u32, y: u32) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    ensure_binding_name_available(&profile, &name)?;

    let point = Point { x, y };
    ensure_point_in_bounds(&profile, point, "tap point")?;

    profile.bindings.push(Binding {
        name: name.clone(),
        input: BindingInput::Key {
            key: normalize_key(&key),
        },
        action: BindingAction::Tap { point },
    });
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("added tap binding: {name}");
    Ok(())
}

fn add_swipe_binding(
    path: PathBuf,
    name: String,
    key: String,
    from: String,
    to: String,
    duration_ms: u64,
) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    ensure_binding_name_available(&profile, &name)?;

    let from = parse_point_arg(&from, "--from")?;
    let to = parse_point_arg(&to, "--to")?;
    if duration_ms == 0 {
        bail!("swipe duration must be greater than zero");
    }
    ensure_point_in_bounds(&profile, from, "--from point")?;
    ensure_point_in_bounds(&profile, to, "--to point")?;

    profile.bindings.push(Binding {
        name: name.clone(),
        input: BindingInput::Key {
            key: normalize_key(&key),
        },
        action: BindingAction::Swipe {
            from,
            to,
            duration_ms,
        },
    });
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("added swipe binding: {name}");
    Ok(())
}

fn remove_binding(path: PathBuf, binding_name: &str) -> Result<()> {
    let mut profile = load_validated_profile(&path)?;
    let index = profile
        .bindings
        .iter()
        .position(|binding| binding.name == binding_name)
        .with_context(|| format!("binding {binding_name} not found"))?;

    let removed = profile.bindings.remove(index);
    profile
        .validate()
        .with_context(|| format!("updated profile {} is invalid", path.display()))?;
    save_profile(&profile, &path)?;
    println!("removed binding: {}", removed.name);
    Ok(())
}

fn list_bindings(profile_path: PathBuf) -> Result<()> {
    let profile = load_validated_profile(&profile_path)?;
    write_stdout(&profile_bindings_listing(&profile))
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

fn input_keyevent(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    code: u32,
) -> Result<()> {
    match select_input_backend(input_executor, backend) {
        SelectedInputBackend::Adb => input_executor.adb_keyevent(code),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_keyevent(code),
    }
}

fn app_list(input_executor: &impl InputExecutor, backend: InputBackend) -> Result<()> {
    let packages = app_packages(input_executor, backend)?;

    write_stdout(&package_listing(&packages))
}

fn app_packages(input_executor: &impl InputExecutor, backend: InputBackend) -> Result<Vec<String>> {
    match select_input_backend(input_executor, backend) {
        SelectedInputBackend::Adb => input_executor.adb_list_packages(),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_app_list_packages(),
    }
}

fn app_launch(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    package_name: &str,
) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    launch_profile_package(input_executor, selected_backend, package_name)?;

    println!("Launched {package_name} via {selected_backend}.");
    Ok(())
}

fn launch_profile_package(
    input_executor: &impl InputExecutor,
    selected_backend: SelectedInputBackend,
    package_name: &str,
) -> Result<()> {
    match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_launch_package(package_name),
        SelectedInputBackend::WaydroidShell => {
            input_executor.waydroid_app_launch_package(package_name)
        }
    }
    .with_context(|| {
        format!("failed to launch Android package {package_name} via {selected_backend}")
    })
}

fn app_install_apk(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    path: PathBuf,
) -> Result<()> {
    ensure_apk_path_exists(&path)?;

    let selected_backend = select_input_backend(input_executor, backend);
    match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_install_apk(&path),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_app_install(&path),
    }
    .with_context(|| {
        format!(
            "failed to install APK {} via {selected_backend}",
            path.display()
        )
    })?;

    println!("Installed APK {} via {selected_backend}.", path.display());
    Ok(())
}

fn app_current(input_executor: &impl InputExecutor, backend: InputBackend) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    let current = match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_current_activity(),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_current_activity(),
    }
    .with_context(|| format!("failed to query current Android activity via {selected_backend}"))?;

    if let Some(activity) = current {
        println!("Current Android activity: {}", activity.component_name);
        println!("Package: {}", activity.package_name);
        println!("Activity: {}", activity.activity_name);
    } else {
        println!(
            "Current Android activity: unavailable (dumpsys did not report a focused or resumed activity)."
        );
    }

    Ok(())
}

fn ensure_apk_path_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("APK path does not exist: {}", path.display());
    }

    if !path.is_file() {
        bail!("APK path is not a file: {}", path.display());
    }

    Ok(())
}

fn package_listing(packages: &[String]) -> String {
    let mut output = String::new();
    for package in packages {
        output.push_str(package);
        output.push('\n');
    }
    output
}

fn write_stdout(output: &str) -> Result<()> {
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    write_output(&mut handle, output)
}

fn write_output(writer: &mut impl Write, output: &str) -> Result<()> {
    match writer.write_all(output.as_bytes()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(error).context("failed to write command output"),
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
    let profile = load_validated_profile(&profile_path)?;

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

fn play(
    input_executor: &impl InputExecutor,
    profile_path: PathBuf,
    backend: InputBackend,
) -> Result<()> {
    let profile = load_play_profile(&profile_path)?;
    let selected_backend = select_input_backend(input_executor, backend);

    println!("Profile: {}", profile.name);
    println!("Package: {}", profile.package_name);
    start_interactive_keymapper(input_executor, &profile, selected_backend)
}

fn run(
    input_executor: &impl InputExecutor,
    profile_path: PathBuf,
    backend: InputBackend,
    launch_delay_ms: u64,
) -> Result<()> {
    let profile = load_play_profile(&profile_path)?;
    let selected_backend = select_input_backend(input_executor, backend);

    println!("Profile: {}", profile.name);
    println!("Package: {}", profile.package_name);
    println!("Launching package {} ...", profile.package_name);
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
        |duration| std::thread::sleep(duration),
        || {
            println!("Starting keymapper ...");
            start_interactive_keymapper(input_executor, &profile, selected_backend)
        },
        launch_delay_ms,
    )
}

fn run_game_workflow_steps(
    launch_package: impl FnOnce() -> Result<()>,
    wait_for_launch: impl FnOnce(Duration),
    start_keymapper: impl FnOnce() -> Result<()>,
    launch_delay_ms: u64,
) -> Result<()> {
    launch_package()?;
    wait_for_launch(Duration::from_millis(launch_delay_ms));
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

fn env_value(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
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
then start the keymapper with: sudo target/debug/wroid play {} --backend waydroid-shell",
        profile_path.display()
    )
}

#[cfg(unix)]
fn effective_uid() -> u32 {
    unsafe extern "C" {
        fn geteuid() -> u32;
    }

    unsafe { geteuid() }
}

#[cfg(not(unix))]
fn effective_uid() -> u32 {
    if std::process::id() == 0 {
        0
    } else {
        u32::MAX
    }
}

fn start_interactive_keymapper(
    input_executor: &impl InputExecutor,
    profile: &ControlProfile,
    selected_backend: SelectedInputBackend,
) -> Result<()> {
    println!("Backend: {}", selected_backend);
    println!();
    print_keyboard_bindings(&profile);
    println!();
    println!("Press a mapped key to run its binding. Press Esc or Ctrl+C to exit.");
    io::stdout().flush().context("failed to flush stdout")?;

    let raw_mode = RawModeGuard::enable()?;

    loop {
        let event = event::read().context("failed to read terminal input")?;
        let Event::Key(key_event) = event else {
            continue;
        };

        if should_exit_key(key_event) {
            break;
        }

        let Some(key_name) = key_event_name(key_event) else {
            continue;
        };

        let Some(binding) = resolve_key_binding(&profile.bindings, &key_name) else {
            continue;
        };

        execute_interactive_binding(input_executor, selected_backend, binding)?;
        io::stdout().flush().context("failed to flush stdout")?;
    }

    drop(raw_mode);
    println!();
    Ok(())
}

fn load_validated_profile(profile_path: &Path) -> Result<ControlProfile> {
    let profile = load_profile_with_context(profile_path)?;
    profile
        .validate()
        .with_context(|| format!("profile {} is invalid", profile_path.display()))?;
    Ok(profile)
}

fn load_play_profile(profile_path: &Path) -> Result<ControlProfile> {
    let profile = load_profile_with_context(profile_path)?;

    match profile.validate() {
        Ok(()) => Ok(profile),
        Err(error)
            if error
                .errors
                .iter()
                .all(|error| matches!(error, ValidationError::UnsupportedAction { .. })) =>
        {
            Ok(profile)
        }
        Err(error) => {
            Err(error).with_context(|| format!("profile {} is invalid", profile_path.display()))
        }
    }
}

fn load_profile_with_context(profile_path: &Path) -> Result<ControlProfile> {
    match ControlProfile::load_from_path(profile_path) {
        Ok(profile) => Ok(profile),
        Err(error) => {
            let mut context = format!("failed to load profile {}", profile_path.display());
            if let Some(hint) = profile_error_hint(&error) {
                context.push_str("\nHint: ");
                context.push_str(hint);
            }
            Err(error).context(context)
        }
    }
}

fn save_profile(profile: &ControlProfile, path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create profile directory {}", parent.display()))?;
    }

    profile
        .save_to_path(path)
        .with_context(|| format!("failed to write profile {}", path.display()))?;
    Ok(())
}

fn ensure_binding_name_available(profile: &ControlProfile, name: &str) -> Result<()> {
    if profile.binding(name).is_some() {
        bail!("binding {name} already exists");
    }

    Ok(())
}

fn ensure_point_in_bounds(profile: &ControlProfile, point: Point, label: &str) -> Result<()> {
    if point.x >= profile.resolution.width || point.y >= profile.resolution.height {
        bail!(
            "{label} {point} is outside profile resolution {}",
            profile.resolution
        );
    }

    Ok(())
}

fn parse_point_arg(value: &str, label: &str) -> Result<Point> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.trim().is_empty()) {
        bail!("{label} must use x,y coordinate format");
    }

    let x = parts[0]
        .trim()
        .parse()
        .with_context(|| format!("{label} has an invalid x coordinate"))?;
    let y = parts[1]
        .trim()
        .parse()
        .with_context(|| format!("{label} has an invalid y coordinate"))?;

    Ok(Point { x, y })
}

fn profile_error_hint(error: &ProfileError) -> Option<&'static str> {
    let ProfileError::Json(error) = error else {
        return None;
    };

    let message = error.to_string();
    if message.contains("missing field `kind`") {
        Some("profile input and action objects are tagged; include a `kind` field such as `key`, `tap`, or `swipe`.")
    } else if message.contains("unknown variant") {
        Some("check `kind` values. Supported MVP input kind: `key`; supported MVP action kinds: `tap` and `swipe`.")
    } else if message.contains("invalid type: map, expected a sequence") {
        Some("check array fields. `bindings` must be a JSON array, and macro `steps` must be an array.")
    } else {
        None
    }
}

fn profile_bindings_listing(profile: &ControlProfile) -> String {
    let mut output = String::new();
    output.push_str(&format!("Profile: {}\n", profile.name));
    output.push_str(&format!("Package: {}\n", profile.package_name));
    output.push_str(&format!("Resolution: {}\n", profile.resolution));
    output.push_str("Bindings:\n");

    if profile.bindings.is_empty() {
        output.push_str("  (none)\n");
    } else {
        for binding in &profile.bindings {
            output.push_str("  ");
            output.push_str(&binding_description(binding));
            output.push('\n');
        }
    }

    output
}

fn binding_description(binding: &Binding) -> String {
    format!(
        "{} -> {} -> {}",
        input_description(&binding.input),
        binding.name,
        action_description(&binding.action)
    )
}

fn input_description(input: &BindingInput) -> String {
    match input {
        BindingInput::Key { key } => key.trim().to_owned(),
        BindingInput::MouseButton { button } => format!("mouse_button {}", button.trim()),
    }
}

fn action_description(action: &BindingAction) -> String {
    match action {
        BindingAction::Tap { point } => format!("tap {point}"),
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => format!("swipe {from} to {to} ({duration_ms} ms)"),
        unsupported => format!("unsupported {}", action_kind(unsupported)),
    }
}

fn print_keyboard_bindings(profile: &ControlProfile) {
    println!("Keyboard bindings:");
    let mut printed = false;

    for binding in keyboard_bindings(&profile.bindings) {
        if let BindingInput::Key { key } = &binding.input {
            println!("  {} -> {}", display_key(key), binding.name);
            printed = true;
        }
    }

    if !printed {
        println!("  (none)");
    }
}

fn keyboard_bindings(bindings: &[Binding]) -> impl Iterator<Item = &Binding> {
    bindings
        .iter()
        .filter(|binding| matches!(binding.input, BindingInput::Key { .. }))
}

fn resolve_key_binding<'a>(bindings: &'a [Binding], key: &str) -> Option<&'a Binding> {
    let normalized_key = normalize_key(key);
    keyboard_bindings(bindings).find(|binding| {
        let BindingInput::Key { key } = &binding.input else {
            return false;
        };
        normalize_key(key) == normalized_key
    })
}

fn normalize_key(key: &str) -> String {
    key.trim().to_lowercase()
}

fn display_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.chars().count() == 1 {
        trimmed.to_uppercase()
    } else {
        trimmed.to_owned()
    }
}

fn key_event_name(key_event: KeyEvent) -> Option<String> {
    match key_event.code {
        KeyCode::Char(character) => Some(character.to_string()),
        KeyCode::Enter => Some("enter".to_owned()),
        KeyCode::Tab => Some("tab".to_owned()),
        KeyCode::BackTab => Some("backtab".to_owned()),
        KeyCode::Backspace => Some("backspace".to_owned()),
        KeyCode::Delete => Some("delete".to_owned()),
        KeyCode::Insert => Some("insert".to_owned()),
        KeyCode::Home => Some("home".to_owned()),
        KeyCode::End => Some("end".to_owned()),
        KeyCode::PageUp => Some("pageup".to_owned()),
        KeyCode::PageDown => Some("pagedown".to_owned()),
        KeyCode::Up => Some("up".to_owned()),
        KeyCode::Down => Some("down".to_owned()),
        KeyCode::Left => Some("left".to_owned()),
        KeyCode::Right => Some("right".to_owned()),
        KeyCode::F(number) => Some(format!("f{number}")),
        KeyCode::Esc
        | KeyCode::Null
        | KeyCode::CapsLock
        | KeyCode::ScrollLock
        | KeyCode::NumLock
        | KeyCode::PrintScreen
        | KeyCode::Pause
        | KeyCode::Menu
        | KeyCode::KeypadBegin
        | KeyCode::Media(_)
        | KeyCode::Modifier(_) => None,
    }
}

fn should_exit_key(key_event: KeyEvent) -> bool {
    key_event.code == KeyCode::Esc
        || (key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn execute_interactive_binding(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    binding: &Binding,
) -> Result<()> {
    match &binding.action {
        BindingAction::Tap { point } => {
            println!("\r{} -> tap {},{}", binding.name, point.x, point.y);
            execute_tap(input_executor, backend, point.x, point.y)
        }
        BindingAction::Swipe {
            from,
            to,
            duration_ms,
        } => {
            println!(
                "\r{} -> swipe {},{} to {},{} ({} ms)",
                binding.name, from.x, from.y, to.x, to.y, duration_ms
            );
            execute_swipe(
                input_executor,
                backend,
                from.x,
                from.y,
                to.x,
                to.y,
                *duration_ms,
            )
        }
        unsupported => {
            println!(
                "\r{} uses unsupported action kind: {}",
                binding.name,
                action_kind(unsupported)
            );
            Ok(())
        }
    }
}

fn execute_tap(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    x: u32,
    y: u32,
) -> Result<()> {
    match backend {
        SelectedInputBackend::Adb => input_executor.adb_tap(x, y),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_tap(x, y),
    }
}

fn execute_swipe(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration_ms: u64,
) -> Result<()> {
    match backend {
        SelectedInputBackend::Adb => input_executor.adb_swipe(x1, y1, x2, y2, duration_ms),
        SelectedInputBackend::WaydroidShell => {
            input_executor.waydroid_shell_swipe(x1, y1, x2, y2, duration_ms)
        }
    }
}

fn action_kind(action: &BindingAction) -> &'static str {
    match action {
        BindingAction::Tap { .. } => "tap",
        BindingAction::Swipe { .. } => "swipe",
        BindingAction::VirtualJoystick { .. } => "virtual_joystick",
        BindingAction::MouseAim { .. } => "mouse_aim",
        BindingAction::Macro { .. } => "macro",
    }
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        terminal::enable_raw_mode().context("failed to enable terminal raw mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
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
    use std::io::{self, Write};

    use anyhow::{anyhow, Result};
    use wroid_core::{Point, Resolution};

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum InputCall {
        AdbTap(u32, u32),
        AdbSwipe(u32, u32, u32, u32, u64),
        AdbKeyevent(u32),
        AdbLaunchPackage(String),
        AdbInstallApk(PathBuf),
        LaunchDelay(u128),
        StartKeymapper,
        WaydroidShellTap(u32, u32),
        WaydroidShellSwipe(u32, u32, u32, u32, u64),
        WaydroidShellKeyevent(u32),
        WaydroidAppLaunchPackage(String),
        WaydroidAppLaunchPackageAsUser {
            package: String,
            user: String,
            session_env: wroid_waydroid::WaydroidAppLaunchEnv,
        },
        WaydroidAppInstall(PathBuf),
    }

    #[derive(Debug, Default)]
    struct FakeInputExecutor {
        devices: Vec<wroid_adb::AdbDevice>,
        fail_devices: bool,
        device_queries: Cell<usize>,
        adb_packages: Vec<String>,
        waydroid_packages: Vec<String>,
        adb_current_activity: Option<CurrentAndroidActivity>,
        waydroid_current_activity: Option<CurrentAndroidActivity>,
        fail_launch: bool,
        calls: RefCell<Vec<InputCall>>,
    }

    struct BrokenPipeWriter;

    impl Write for BrokenPipeWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
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

        fn with_waydroid_packages(packages: Vec<&str>) -> Self {
            Self {
                waydroid_packages: packages
                    .into_iter()
                    .map(std::borrow::ToOwned::to_owned)
                    .collect(),
                ..Self::default()
            }
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

        fn adb_keyevent(&self, code: u32) -> Result<()> {
            self.calls.borrow_mut().push(InputCall::AdbKeyevent(code));
            Ok(())
        }

        fn adb_list_packages(&self) -> Result<Vec<String>> {
            Ok(self.adb_packages.clone())
        }

        fn adb_launch_package(&self, package_name: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::AdbLaunchPackage(package_name.to_owned()));
            if self.fail_launch {
                Err(anyhow!("adb launch failed"))
            } else {
                Ok(())
            }
        }

        fn adb_install_apk(&self, path: &Path) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::AdbInstallApk(path.to_owned()));
            Ok(())
        }

        fn adb_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
            Ok(self.adb_current_activity.clone())
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

        fn waydroid_shell_keyevent(&self, code: u32) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::WaydroidShellKeyevent(code));
            Ok(())
        }

        fn waydroid_app_list_packages(&self) -> Result<Vec<String>> {
            Ok(self.waydroid_packages.clone())
        }

        fn waydroid_app_launch_package(&self, package_name: &str) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::WaydroidAppLaunchPackage(package_name.to_owned()));
            if self.fail_launch {
                Err(anyhow!("waydroid launch failed"))
            } else {
                Ok(())
            }
        }

        fn waydroid_app_launch_package_as_user(
            &self,
            package_name: &str,
            user: &str,
            session_env: &wroid_waydroid::WaydroidAppLaunchEnv,
        ) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::WaydroidAppLaunchPackageAsUser {
                    package: package_name.to_owned(),
                    user: user.to_owned(),
                    session_env: session_env.clone(),
                });
            if self.fail_launch {
                Err(anyhow!("waydroid launch as user failed"))
            } else {
                Ok(())
            }
        }

        fn waydroid_app_install(&self, path: &Path) -> Result<()> {
            self.calls
                .borrow_mut()
                .push(InputCall::WaydroidAppInstall(path.to_owned()));
            Ok(())
        }

        fn waydroid_shell_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
            Ok(self.waydroid_current_activity.clone())
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
    fn app_launch_dispatches_to_explicit_waydroid_shell_backend() {
        let executor = FakeInputExecutor::default();

        app_launch(&executor, InputBackend::WaydroidShell, "com.example.game").unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackage(
                "com.example.game".to_owned()
            )]
        );
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn app_launch_dispatches_to_auto_selected_adb_backend() {
        let executor = FakeInputExecutor::with_device_state("connected-device", "device");

        app_launch(&executor, InputBackend::Auto, "com.example.game").unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::AdbLaunchPackage("com.example.game".to_owned())]
        );
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn app_list_uses_explicit_backend_without_adb_auto_selection() {
        let executor = FakeInputExecutor::with_waydroid_packages(vec!["com.example.game"]);

        let packages = app_packages(&executor, InputBackend::WaydroidShell).unwrap();

        assert_eq!(packages, vec!["com.example.game"]);
        assert_eq!(executor.device_queries.get(), 0);
        assert!(executor.calls().is_empty());
    }

    #[test]
    fn package_listing_prints_one_package_per_line() {
        assert_eq!(
            package_listing(&[
                "com.example.game".to_owned(),
                "org.example.second".to_owned()
            ]),
            "com.example.game\norg.example.second\n"
        );
    }

    #[test]
    fn write_output_treats_broken_pipe_as_success() {
        let mut writer = BrokenPipeWriter;

        write_output(&mut writer, "com.example.game\n").unwrap();
    }

    #[test]
    fn apk_path_validation_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.apk");

        let err = ensure_apk_path_exists(&path).unwrap_err();

        assert!(err.to_string().contains("APK path does not exist"));
    }

    #[test]
    fn app_install_apk_validates_path_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.apk");
        let executor = FakeInputExecutor::default();

        let err = app_install_apk(&executor, InputBackend::Adb, path).unwrap_err();

        assert!(err.to_string().contains("APK path does not exist"));
        assert!(executor.calls().is_empty());
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn app_install_apk_dispatches_to_adb_after_path_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.apk");
        fs::write(&path, b"fake apk").unwrap();
        let executor = FakeInputExecutor::default();

        app_install_apk(&executor, InputBackend::Adb, path.clone()).unwrap();

        assert_eq!(executor.calls(), vec![InputCall::AdbInstallApk(path)]);
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn run_command_default_launch_delay_is_1500() {
        let cli = Cli::try_parse_from(["wroid", "run", "profiles/my-game.json"]).unwrap();

        let Commands::Run {
            profile_path,
            backend,
            launch_delay_ms,
        } = cli.command
        else {
            panic!("expected run command");
        };

        assert_eq!(profile_path, PathBuf::from("profiles/my-game.json"));
        assert_eq!(backend, InputBackend::Auto);
        assert_eq!(launch_delay_ms, DEFAULT_LAUNCH_DELAY_MS);
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
            ..
        } = cli.command
        else {
            panic!("expected run command");
        };

        assert_eq!(backend, InputBackend::WaydroidShell);
        assert_eq!(launch_delay_ms, 2500);
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
            2500,
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
            1500,
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
        assert!(message
            .contains("sudo target/debug/wroid play /tmp/game.json --backend waydroid-shell"));
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
    fn binding_description_formats_tap_binding() {
        let binding = &ControlProfile::example().bindings[0];

        assert_eq!(binding_description(binding), "f -> fire -> tap 1640,540");
    }

    #[test]
    fn binding_description_formats_swipe_binding() {
        let binding = &ControlProfile::example().bindings[2];

        assert_eq!(
            binding_description(binding),
            "d -> look_right -> swipe 960,540 to 1260,540 (180 ms)"
        );
    }

    #[test]
    fn profile_bindings_listing_includes_metadata_and_bindings() {
        let listing = profile_bindings_listing(&ControlProfile::example());

        assert_eq!(
            listing,
            "\
Profile: Shooter Basic
Package: com.example.shooter
Resolution: 1920x1080
Bindings:
  f -> fire -> tap 1640,540
  r -> reload -> tap 1760,900
  d -> look_right -> swipe 960,540 to 1260,540 (180 ms)
"
        );
    }

    #[test]
    fn creates_profile_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("profile.json");

        create_profile(
            path.clone(),
            "Test Profile".to_owned(),
            "com.example.test".to_owned(),
            1280,
            720,
            false,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(profile.name, "Test Profile");
        assert_eq!(profile.package_name, "com.example.test");
        assert_eq!(
            profile.resolution,
            Resolution {
                width: 1280,
                height: 720
            }
        );
        assert!(profile.bindings.is_empty());
        profile.validate().unwrap();
    }

    #[test]
    fn adds_tap_binding() {
        let (_dir, path) = new_empty_profile();

        add_tap_binding(path.clone(), "fire".to_owned(), "F".to_owned(), 100, 200).unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(
            profile.bindings,
            vec![Binding {
                name: "fire".to_owned(),
                input: BindingInput::Key {
                    key: "f".to_owned()
                },
                action: BindingAction::Tap {
                    point: Point { x: 100, y: 200 }
                },
            }]
        );
        profile.validate().unwrap();
    }

    #[test]
    fn adds_swipe_binding() {
        let (_dir, path) = new_empty_profile();

        add_swipe_binding(
            path.clone(),
            "look_right".to_owned(),
            "D".to_owned(),
            "300,400".to_owned(),
            "600,400".to_owned(),
            180,
        )
        .unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert_eq!(
            profile.bindings,
            vec![Binding {
                name: "look_right".to_owned(),
                input: BindingInput::Key {
                    key: "d".to_owned()
                },
                action: BindingAction::Swipe {
                    from: Point { x: 300, y: 400 },
                    to: Point { x: 600, y: 400 },
                    duration_ms: 180,
                },
            }]
        );
        profile.validate().unwrap();
    }

    #[test]
    fn duplicate_binding_fails() {
        let (_dir, path) = new_empty_profile();
        add_tap_binding(path.clone(), "fire".to_owned(), "f".to_owned(), 100, 200).unwrap();

        let err = add_tap_binding(path, "fire".to_owned(), "g".to_owned(), 300, 400).unwrap_err();

        assert!(err.to_string().contains("binding fire already exists"));
    }

    #[test]
    fn invalid_coordinate_format_fails() {
        let (_dir, path) = new_empty_profile();

        let err = add_swipe_binding(
            path,
            "look_right".to_owned(),
            "d".to_owned(),
            "300".to_owned(),
            "600,400".to_owned(),
            180,
        )
        .unwrap_err();

        assert!(err
            .to_string()
            .contains("--from must use x,y coordinate format"));
    }

    #[test]
    fn removes_existing_binding() {
        let (_dir, path) = new_empty_profile();
        add_tap_binding(path.clone(), "fire".to_owned(), "f".to_owned(), 100, 200).unwrap();

        remove_binding(path.clone(), "fire").unwrap();

        let profile = ControlProfile::load_from_path(&path).unwrap();
        assert!(profile.bindings.is_empty());
        profile.validate().unwrap();
    }

    #[test]
    fn removing_missing_binding_fails() {
        let (_dir, path) = new_empty_profile();

        let err = remove_binding(path, "fire").unwrap_err();

        assert!(err.to_string().contains("binding fire not found"));
    }

    #[test]
    fn resolves_key_f_to_fire_binding() {
        let profile = keyboard_test_profile();

        let binding = resolve_key_binding(&profile.bindings, "F").unwrap();

        assert_eq!(binding.name, "fire");
    }

    #[test]
    fn resolves_key_r_to_reload_binding() {
        let profile = keyboard_test_profile();

        let binding = resolve_key_binding(&profile.bindings, "R").unwrap();

        assert_eq!(binding.name, "reload");
    }

    #[test]
    fn unknown_key_returns_no_binding() {
        let profile = keyboard_test_profile();

        assert!(resolve_key_binding(&profile.bindings, "X").is_none());
    }

    #[test]
    fn non_keyboard_bindings_are_ignored_by_interactive_runner() {
        let profile = ControlProfile {
            name: "Mouse Profile".to_owned(),
            package_name: "com.example.mouse".to_owned(),
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            bindings: vec![Binding {
                name: "fire".to_owned(),
                input: BindingInput::MouseButton {
                    button: "f".to_owned(),
                },
                action: BindingAction::Tap {
                    point: Point { x: 100, y: 100 },
                },
            }],
        };

        assert!(resolve_key_binding(&profile.bindings, "F").is_none());
    }

    fn new_empty_profile() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.json");
        create_profile(
            path.clone(),
            "Test Profile".to_owned(),
            "com.example.test".to_owned(),
            1280,
            720,
            false,
        )
        .unwrap();
        (dir, path)
    }

    fn keyboard_test_profile() -> ControlProfile {
        ControlProfile {
            name: "Keyboard Profile".to_owned(),
            package_name: "com.example.keyboard".to_owned(),
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            bindings: vec![
                Binding {
                    name: "fire".to_owned(),
                    input: BindingInput::Key {
                        key: "f".to_owned(),
                    },
                    action: BindingAction::Tap {
                        point: Point { x: 1640, y: 540 },
                    },
                },
                Binding {
                    name: "reload".to_owned(),
                    input: BindingInput::Key {
                        key: "r".to_owned(),
                    },
                    action: BindingAction::Tap {
                        point: Point { x: 1760, y: 900 },
                    },
                },
                Binding {
                    name: "look".to_owned(),
                    input: BindingInput::MouseButton {
                        button: "left".to_owned(),
                    },
                    action: BindingAction::Swipe {
                        from: Point { x: 960, y: 540 },
                        to: Point { x: 1260, y: 540 },
                        duration_ms: 180,
                    },
                },
            ],
        }
    }
}
