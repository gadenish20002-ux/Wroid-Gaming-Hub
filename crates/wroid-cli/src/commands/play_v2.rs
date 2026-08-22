use std::path::PathBuf;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use wroid_core::profile_v2::{InputV2, ProfileV2};
use wroid_core::Resolution;
use wroid_inject::{run_game_session, BridgeBrokerClient, GameSessionOptions, GameSessionReport};
use wroid_input::{discover_keyboard_path, discover_mouse_path};

pub(crate) struct PlayV2Options {
    pub keyboard: Option<PathBuf>,
    pub mouse: Option<PathBuf>,
    pub resolution: Resolution,
    pub grab: bool,
    pub show_ui: bool,
    pub launch_package: bool,
    pub trace_input: bool,
    pub trace_android_input: bool,
    pub exit_after: Option<Duration>,
    pub focus_socket: Option<PathBuf>,
}

pub(crate) fn play_v2(profile_path: PathBuf, options: PlayV2Options) -> Result<GameSessionReport> {
    play_v2_with_broker(profile_path, options, None)
}

pub(crate) fn play_v2_with_broker(
    profile_path: PathBuf,
    options: PlayV2Options,
    bridge_broker: Option<BridgeBrokerClient>,
) -> Result<GameSessionReport> {
    // SAFETY: geteuid takes no arguments and has no preconditions.
    let is_root = unsafe { libc::geteuid() } == 0;
    ensure_play_bridge_access(is_root, bridge_broker.is_some())?;
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
    session.trace_android_input = options.trace_android_input;
    session.exit_after = options.exit_after;
    session.focus_socket = options.focus_socket;
    session.bridge_broker = bridge_broker;
    run_game_session(session).map_err(anyhow::Error::msg)
}

fn ensure_play_bridge_access(is_root: bool, has_broker: bool) -> Result<()> {
    if !is_root && !has_broker {
        bail!("unprivileged play-v2 requires the daemon bridge; use `wroid launch-v2` for production sessions");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_play_requires_root_or_an_inherited_broker() {
        assert!(ensure_play_bridge_access(false, false).is_err());
        ensure_play_bridge_access(true, false).unwrap();
        ensure_play_bridge_access(false, true).unwrap();
    }
}
