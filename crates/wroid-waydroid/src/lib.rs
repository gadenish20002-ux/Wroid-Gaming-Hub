use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

const WAYDROID_SHELL_ROOT_ERROR: &str = "Action \"shell\" needs root access";
const WAYDROID_SHELL_ROOT_MESSAGE: &str =
    "Waydroid shell backend requires root privileges on this system. Try: sudo target/debug/wroid ...";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidActivity {
    pub package_name: String,
    pub activity_name: String,
    pub component_name: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Waydroid;

impl Waydroid {
    pub fn is_available(&self) -> bool {
        Command::new("waydroid")
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    pub fn status(&self) -> Result<String> {
        let output = Command::new("waydroid")
            .arg("status")
            .output()
            .context("failed to run waydroid status")?;
        ensure_success("waydroid status", &output)?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    pub fn session_start(&self) -> Result<()> {
        let output = Command::new("waydroid")
            .args(["session", "start"])
            .output()
            .context("failed to run waydroid session start")?;
        ensure_success("waydroid session start", &output)
    }

    pub fn show_full_ui(&self) -> Result<()> {
        let output = Command::new("waydroid")
            .arg("show-full-ui")
            .output()
            .context("failed to run waydroid show-full-ui")?;
        ensure_success("waydroid show-full-ui", &output)
    }

    pub fn shell_input_tap(&self, x: u32, y: u32) -> Result<()> {
        let output = Command::new("waydroid")
            .args(["shell", "input", "tap"])
            .arg(x.to_string())
            .arg(y.to_string())
            .output()
            .context("failed to run waydroid shell input tap")?;
        ensure_success("waydroid shell input tap", &output)
    }

    pub fn shell_input_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()> {
        let output = Command::new("waydroid")
            .args(["shell", "input", "swipe"])
            .arg(x1.to_string())
            .arg(y1.to_string())
            .arg(x2.to_string())
            .arg(y2.to_string())
            .arg(duration_ms.to_string())
            .output()
            .context("failed to run waydroid shell input swipe")?;
        ensure_success("waydroid shell input swipe", &output)
    }

    pub fn shell_input_keyevent(&self, code: u32) -> Result<()> {
        let output = Command::new("waydroid")
            .args(["shell", "input", "keyevent"])
            .arg(code.to_string())
            .output()
            .context("failed to run waydroid shell input keyevent")?;
        ensure_success("waydroid shell input keyevent", &output)
    }

    pub fn app_list_packages(&self) -> Result<Vec<String>> {
        let output = Command::new("waydroid")
            .args(app_list_args())
            .output()
            .context("failed to run waydroid app list")?;
        ensure_success("waydroid app list", &output)?;

        Ok(parse_package_list(&String::from_utf8_lossy(&output.stdout)))
    }

    pub fn app_launch_package(&self, package_name: &str) -> Result<()> {
        let output = Command::new("waydroid")
            .args(app_launch_args(package_name))
            .output()
            .context("failed to run waydroid app launch")?;
        ensure_success("waydroid app launch", &output)
    }

    pub fn app_install(&self, path: impl AsRef<Path>) -> Result<()> {
        let output = Command::new("waydroid")
            .args(["app", "install"])
            .arg(path.as_ref())
            .output()
            .context("failed to run waydroid app install")?;
        ensure_success("waydroid app install", &output)
    }

    pub fn shell_current_activity(&self) -> Result<Option<AndroidActivity>> {
        let output = Command::new("waydroid")
            .args(["shell", "dumpsys", "activity", "activities"])
            .output()
            .context("failed to run waydroid shell dumpsys activity activities")?;
        ensure_success("waydroid shell dumpsys activity activities", &output)?;

        Ok(parse_current_activity(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }
}

pub fn is_available() -> bool {
    Waydroid.is_available()
}

pub fn status() -> Result<String> {
    Waydroid.status()
}

pub fn session_start() -> Result<()> {
    Waydroid.session_start()
}

pub fn show_full_ui() -> Result<()> {
    Waydroid.show_full_ui()
}

pub fn shell_input_tap(x: u32, y: u32) -> Result<()> {
    Waydroid.shell_input_tap(x, y)
}

pub fn shell_input_swipe(x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
    Waydroid.shell_input_swipe(x1, y1, x2, y2, duration_ms)
}

pub fn shell_input_keyevent(code: u32) -> Result<()> {
    Waydroid.shell_input_keyevent(code)
}

pub fn app_list_packages() -> Result<Vec<String>> {
    Waydroid.app_list_packages()
}

pub fn app_launch_package(package_name: &str) -> Result<()> {
    Waydroid.app_launch_package(package_name)
}

pub fn app_install(path: impl AsRef<Path>) -> Result<()> {
    Waydroid.app_install(path)
}

pub fn shell_current_activity() -> Result<Option<AndroidActivity>> {
    Waydroid.shell_current_activity()
}

fn parse_package_list(output: &str) -> Vec<String> {
    output.lines().filter_map(parse_package_list_line).collect()
}

fn parse_package_list_line(line: &str) -> Option<String> {
    let trimmed = line.trim().trim_matches(',');
    if trimmed.is_empty() {
        return None;
    }

    if is_package_name(trimmed) {
        return Some(trimmed.to_owned());
    }

    for separator in [':', '='] {
        let Some((key, value)) = trimmed.split_once(separator) else {
            continue;
        };

        if key.to_ascii_lowercase().contains("package") {
            return first_package_name_token(value);
        }
    }

    None
}

fn first_package_name_token(value: &str) -> Option<String> {
    value
        .split(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '"' | '\'' | ',' | ';' | '[' | ']' | '{' | '}' | '(' | ')'
                )
        })
        .find(|token| is_package_name(token))
        .map(std::borrow::ToOwned::to_owned)
}

fn app_list_args() -> [&'static str; 2] {
    ["app", "list"]
}

fn app_launch_args(package_name: &str) -> [&str; 3] {
    ["app", "launch", package_name]
}

fn parse_current_activity(output: &str) -> Option<AndroidActivity> {
    output
        .lines()
        .filter(|line| is_focused_or_resumed_activity_line(line))
        .find_map(parse_activity_line_component)
}

fn is_focused_or_resumed_activity_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    line.contains("mFocusedActivity")
        || line.contains("mResumedActivity")
        || line.contains("topResumedActivity")
        || trimmed.starts_with("ResumedActivity")
}

fn parse_activity_line_component(line: &str) -> Option<AndroidActivity> {
    line.split_whitespace()
        .filter_map(component_from_token)
        .next()
}

fn component_from_token(token: &str) -> Option<AndroidActivity> {
    let token = token
        .trim_matches(|character| {
            matches!(
                character,
                '{' | '}' | '[' | ']' | '(' | ')' | ',' | ';' | '\''
            )
        })
        .strip_prefix("cmp=")
        .unwrap_or_else(|| {
            token.trim_matches(|character| {
                matches!(
                    character,
                    '{' | '}' | '[' | ']' | '(' | ')' | ',' | ';' | '\''
                )
            })
        });

    let (package_name, activity_part) = token.split_once('/')?;
    if !is_package_name(package_name) || activity_part.trim().is_empty() {
        return None;
    }

    let activity_name = if activity_part.starts_with('.') {
        format!("{package_name}{activity_part}")
    } else {
        activity_part.to_owned()
    };

    Some(AndroidActivity {
        package_name: package_name.to_owned(),
        activity_name,
        component_name: format!("{package_name}/{activity_part}"),
    })
}

fn is_package_name(value: &str) -> bool {
    value.contains('.')
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '.'))
}

fn ensure_success(command: &str, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if let Some(message) = map_waydroid_shell_error(command, &stdout, &stderr) {
        bail!("{message}");
    }

    bail!("{command} failed: {}", stderr.trim());
}

fn map_waydroid_shell_error(command: &str, stdout: &str, stderr: &str) -> Option<&'static str> {
    if command.starts_with("waydroid shell")
        && (stdout.contains(WAYDROID_SHELL_ROOT_ERROR)
            || stderr.contains(WAYDROID_SHELL_ROOT_ERROR))
    {
        Some(WAYDROID_SHELL_ROOT_MESSAGE)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_waydroid_shell_root_error_to_actionable_message() {
        let message = map_waydroid_shell_error(
            "waydroid shell input tap",
            "",
            "ERROR: Action \"shell\" needs root access\n",
        );

        assert_eq!(message, Some(WAYDROID_SHELL_ROOT_MESSAGE));
    }

    #[test]
    fn maps_waydroid_shell_swipe_root_error_to_actionable_message() {
        let message = map_waydroid_shell_error(
            "waydroid shell input swipe",
            "",
            "ERROR: Action \"shell\" needs root access\n",
        );

        assert_eq!(message, Some(WAYDROID_SHELL_ROOT_MESSAGE));
    }

    #[test]
    fn maps_waydroid_shell_keyevent_root_error_to_actionable_message() {
        let message = map_waydroid_shell_error(
            "waydroid shell input keyevent",
            "",
            "ERROR: Action \"shell\" needs root access\n",
        );

        assert_eq!(message, Some(WAYDROID_SHELL_ROOT_MESSAGE));
    }

    #[test]
    fn does_not_map_non_shell_input_waydroid_errors() {
        let message = map_waydroid_shell_error(
            "waydroid status",
            "",
            "ERROR: Action \"shell\" needs root access\n",
        );

        assert_eq!(message, None);
    }

    #[test]
    fn maps_waydroid_shell_pm_root_error_to_actionable_message() {
        let message = map_waydroid_shell_error(
            "waydroid shell pm list packages",
            "",
            "ERROR: Action \"shell\" needs root access\n",
        );

        assert_eq!(message, Some(WAYDROID_SHELL_ROOT_MESSAGE));
    }

    #[test]
    fn does_not_map_unrelated_shell_input_errors() {
        let message = map_waydroid_shell_error(
            "waydroid shell input tap",
            "",
            "ERROR: WayDroid container is STOPPED\n",
        );

        assert_eq!(message, None);
    }

    #[test]
    fn parses_pm_package_list_without_prefix() {
        let packages = parse_package_list(
            "\
package:com.example.game
package:org.example.second

",
        );

        assert_eq!(packages, vec!["com.example.game", "org.example.second"]);
    }

    #[test]
    fn parses_waydroid_app_list_package_fields() {
        let packages = parse_package_list(
            "\
Name: Settings
packageName: com.android.settings
package_name = org.example.second
com.example.raw
",
        );

        assert_eq!(
            packages,
            vec![
                "com.android.settings",
                "org.example.second",
                "com.example.raw"
            ]
        );
    }

    #[test]
    fn app_list_uses_waydroid_app_list_args() {
        assert_eq!(app_list_args(), ["app", "list"]);
    }

    #[test]
    fn app_launch_package_uses_waydroid_app_launch_args() {
        assert_eq!(
            app_launch_args("com.example.game"),
            ["app", "launch", "com.example.game"]
        );
    }

    #[test]
    fn parses_resumed_activity_component() {
        let current = parse_current_activity(
            "topResumedActivity=ActivityRecord{41d u0 com.example.game/.MainActivity t12}\n",
        )
        .unwrap();

        assert_eq!(current.package_name, "com.example.game");
        assert_eq!(current.activity_name, "com.example.game.MainActivity");
        assert_eq!(current.component_name, "com.example.game/.MainActivity");
    }

    #[test]
    fn does_not_guess_current_activity_from_unrelated_lines() {
        let current =
            parse_current_activity("ActivityRecord{41d u0 com.example.game/.MainActivity t12}\n");

        assert_eq!(current, None);
    }
}
