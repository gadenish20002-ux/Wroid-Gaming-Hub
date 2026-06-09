use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidActivity {
    pub package_name: String,
    pub activity_name: String,
    pub component_name: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Adb;

impl Adb {
    pub fn is_available(&self) -> bool {
        Command::new("adb")
            .arg("version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    pub fn devices(&self) -> Result<Vec<AdbDevice>> {
        let output = Command::new("adb")
            .arg("devices")
            .output()
            .context("failed to run adb devices")?;
        ensure_success("adb devices", &output)?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let devices = stdout
            .lines()
            .skip(1)
            .filter_map(parse_device_line)
            .collect();

        Ok(devices)
    }

    pub fn tap(&self, x: u32, y: u32) -> Result<()> {
        let output = Command::new("adb")
            .args(["shell", "input", "tap"])
            .arg(x.to_string())
            .arg(y.to_string())
            .output()
            .context("failed to run adb shell input tap")?;
        ensure_success("adb shell input tap", &output)
    }

    pub fn swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
        let output = Command::new("adb")
            .args(["shell", "input", "swipe"])
            .arg(x1.to_string())
            .arg(y1.to_string())
            .arg(x2.to_string())
            .arg(y2.to_string())
            .arg(duration_ms.to_string())
            .output()
            .context("failed to run adb shell input swipe")?;
        ensure_success("adb shell input swipe", &output)
    }

    pub fn keyevent(&self, code: u32) -> Result<()> {
        let output = Command::new("adb")
            .args(["shell", "input", "keyevent"])
            .arg(code.to_string())
            .output()
            .context("failed to run adb shell input keyevent")?;
        ensure_success("adb shell input keyevent", &output)
    }

    pub fn list_packages(&self) -> Result<Vec<String>> {
        let output = Command::new("adb")
            .args(["shell", "pm", "list", "packages"])
            .output()
            .context("failed to run adb shell pm list packages")?;
        ensure_success("adb shell pm list packages", &output)?;

        Ok(parse_package_list(&String::from_utf8_lossy(&output.stdout)))
    }

    pub fn launch_package(&self, package_name: &str) -> Result<()> {
        let output = Command::new("adb")
            .args(monkey_launch_args(package_name))
            .output()
            .context("failed to run adb shell monkey")?;
        ensure_success("adb shell monkey", &output)
    }

    pub fn install_apk(&self, path: impl AsRef<Path>) -> Result<()> {
        let output = Command::new("adb")
            .arg("install")
            .arg(path.as_ref())
            .output()
            .context("failed to run adb install")?;
        ensure_success("adb install", &output)
    }

    pub fn current_activity(&self) -> Result<Option<AndroidActivity>> {
        let output = Command::new("adb")
            .args(["shell", "dumpsys", "activity", "activities"])
            .output()
            .context("failed to run adb shell dumpsys activity activities")?;
        ensure_success("adb shell dumpsys activity activities", &output)?;

        Ok(parse_current_activity(&String::from_utf8_lossy(
            &output.stdout,
        )))
    }

    pub fn wm_size(&self) -> Result<String> {
        let output = Command::new("adb")
            .args(["shell", "wm", "size"])
            .output()
            .context("failed to run adb shell wm size")?;
        ensure_success("adb shell wm size", &output)?;

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub fn wm_density(&self) -> Result<String> {
        let output = Command::new("adb")
            .args(["shell", "wm", "density"])
            .output()
            .context("failed to run adb shell wm density")?;
        ensure_success("adb shell wm density", &output)?;

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

pub fn is_available() -> bool {
    Adb.is_available()
}

pub fn devices() -> Result<Vec<AdbDevice>> {
    Adb.devices()
}

pub fn tap(x: u32, y: u32) -> Result<()> {
    Adb.tap(x, y)
}

pub fn swipe(x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
    Adb.swipe(x1, y1, x2, y2, duration_ms)
}

pub fn keyevent(code: u32) -> Result<()> {
    Adb.keyevent(code)
}

pub fn list_packages() -> Result<Vec<String>> {
    Adb.list_packages()
}

pub fn launch_package(package_name: &str) -> Result<()> {
    Adb.launch_package(package_name)
}

pub fn install_apk(path: impl AsRef<Path>) -> Result<()> {
    Adb.install_apk(path)
}

pub fn current_activity() -> Result<Option<AndroidActivity>> {
    Adb.current_activity()
}

pub fn wm_size() -> Result<String> {
    Adb.wm_size()
}

pub fn wm_density() -> Result<String> {
    Adb.wm_density()
}

fn parse_device_line(line: &str) -> Option<AdbDevice> {
    let mut parts = line.split_whitespace();
    let serial = parts.next()?;
    let state = parts.next()?;

    Some(AdbDevice {
        serial: serial.to_owned(),
        state: state.to_owned(),
    })
}

fn parse_package_list(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let package = trimmed
                .strip_prefix("package:")
                .unwrap_or(trimmed)
                .rsplit_once('=')
                .map(|(_, package)| package)
                .unwrap_or_else(|| trimmed.strip_prefix("package:").unwrap_or(trimmed))
                .trim();

            if package.is_empty() {
                None
            } else {
                Some(package.to_owned())
            }
        })
        .collect()
}

fn monkey_launch_args(package_name: &str) -> [&str; 7] {
    [
        "shell",
        "monkey",
        "-p",
        package_name,
        "-c",
        "android.intent.category.LAUNCHER",
        "1",
    ]
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

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{command} failed: {}", stderr.trim());
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_pm_package_list_when_prefix_is_missing() {
        let packages = parse_package_list("com.example.game\npackage:org.example.second\n");

        assert_eq!(packages, vec!["com.example.game", "org.example.second"]);
    }

    #[test]
    fn launch_package_uses_monkey_launcher_args() {
        assert_eq!(
            monkey_launch_args("com.example.game"),
            [
                "shell",
                "monkey",
                "-p",
                "com.example.game",
                "-c",
                "android.intent.category.LAUNCHER",
                "1"
            ]
        );
    }

    #[test]
    fn parses_focused_activity_component() {
        let current = parse_current_activity(
            "mFocusedActivity: ActivityRecord{41d u0 com.example.game/.MainActivity t12}\n",
        )
        .unwrap();

        assert_eq!(current.package_name, "com.example.game");
        assert_eq!(current.activity_name, "com.example.game.MainActivity");
        assert_eq!(current.component_name, "com.example.game/.MainActivity");
    }

    #[test]
    fn parses_resumed_activity_component_with_full_activity_name() {
        let current = parse_current_activity(
            "mResumedActivity: ActivityRecord{41d u0 com.example.game/com.example.game.MainActivity t12}\n",
        )
        .unwrap();

        assert_eq!(current.package_name, "com.example.game");
        assert_eq!(current.activity_name, "com.example.game.MainActivity");
        assert_eq!(
            current.component_name,
            "com.example.game/com.example.game.MainActivity"
        );
    }

    #[test]
    fn does_not_guess_current_activity_from_unrelated_lines() {
        let current =
            parse_current_activity("ActivityRecord{41d u0 com.example.game/.MainActivity t12}\n");

        assert_eq!(current, None);
    }
}
