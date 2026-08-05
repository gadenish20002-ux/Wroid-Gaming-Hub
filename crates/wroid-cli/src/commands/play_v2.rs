use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use wroid_core::profile_v2::{InputV2, ProfileV2};
use wroid_core::Resolution;
use wroid_inject::{run_game_session, BridgeHelperCommand, GameSessionOptions, GameSessionReport};
use wroid_input::{discover_keyboard_path, discover_mouse_path};

use super::system_helper;

pub(crate) struct PlayV2Options {
    pub keyboard: Option<PathBuf>,
    pub mouse: Option<PathBuf>,
    pub resolution: Resolution,
    pub grab: bool,
    pub show_ui: bool,
    pub launch_package: bool,
    pub trace_input: bool,
    pub exit_after: Option<Duration>,
    pub focus_socket: Option<PathBuf>,
}

pub(crate) fn play_v2(profile_path: PathBuf, options: PlayV2Options) -> Result<GameSessionReport> {
    let profile = ProfileV2::load_from_path(&profile_path)
        .with_context(|| format!("failed to load profile v2 {}", profile_path.display()))?;
    profile
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid profile v2: {}", error.errors.join("; ")))?;
    let needs_mouse = profile.bindings.iter().any(|binding| {
        matches!(
            binding.input,
            InputV2::MouseMove | InputV2::MouseButton { .. }
        )
    });
    let keyboard_path = match options.keyboard {
        Some(path) => path,
        None => discover_keyboard_path().context(
            "failed to auto-detect a keyboard; pass --keyboard /dev/input/by-id/...-event-kbd",
        )?,
    };
    let mouse_path = match (options.mouse, needs_mouse) {
        (Some(path), _) => Some(path),
        (None, true) => Some(discover_mouse_path().context(
            "failed to auto-detect a mouse; pass --mouse /dev/input/by-id/...-event-mouse",
        )?),
        (None, false) => None,
    };

    println!("Keyboard input: {}", keyboard_path.display());
    match mouse_path.as_ref() {
        Some(path) => println!("Mouse input: {}", path.display()),
        None => println!("Mouse input: not required by profile"),
    }
    println!(
        "Android surface: {}x{}",
        options.resolution.width, options.resolution.height
    );

    let mut session = GameSessionOptions::new(
        profile_path,
        keyboard_path,
        mouse_path,
        options.resolution.width,
        options.resolution.height,
    )
    .map_err(anyhow::Error::msg)?;
    session.grab = options.grab;
    session.show_ui = options.show_ui;
    session.launch_package = options.launch_package;
    session.trace_input = options.trace_input;
    session.exit_after = options.exit_after;
    session.focus_socket = options.focus_socket;
    system_helper::ensure_ready()?;
    session.bridge_helper = Some(
        BridgeHelperCommand::production()
            .context("production Wroid bridge helper is not ready; run `wroid helper install`")?,
    );
    run_game_session(session).map_err(anyhow::Error::msg)
}
