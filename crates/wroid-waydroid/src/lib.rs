use std::process::Command;

use anyhow::{bail, Context, Result};

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

fn ensure_success(command: &str, output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    bail!("{command} failed: {}", stderr.trim());
}
