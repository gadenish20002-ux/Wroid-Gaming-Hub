use std::process::Command;

use anyhow::{bail, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub state: String,
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

fn parse_device_line(line: &str) -> Option<AdbDevice> {
    let mut parts = line.split_whitespace();
    let serial = parts.next()?;
    let state = parts.next()?;

    Some(AdbDevice {
        serial: serial.to_owned(),
        state: state.to_owned(),
    })
}

fn ensure_success(command: &str, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{command} failed: {}", stderr.trim());
}
