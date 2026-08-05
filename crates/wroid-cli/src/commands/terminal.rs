use std::env;
use std::ffi::OsString;
use std::io;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

pub(crate) fn spawn_terminal(command: &[OsString]) -> Result<String> {
    let mut candidates = Vec::<(OsString, Vec<OsString>)>::new();
    if let Some(terminal) = env::var_os("TERMINAL").filter(|value| !value.is_empty()) {
        candidates.push((terminal, vec![OsString::from("-e")]));
    }
    candidates.extend([
        (OsString::from("xdg-terminal-exec"), vec![]),
        (
            OsString::from("x-terminal-emulator"),
            vec![OsString::from("-e")],
        ),
        (OsString::from("gnome-terminal"), vec![OsString::from("--")]),
        (OsString::from("kgx"), vec![OsString::from("--")]),
        (OsString::from("konsole"), vec![OsString::from("-e")]),
        (OsString::from("foot"), vec![OsString::from("-e")]),
        (OsString::from("kitty"), Vec::new()),
        (OsString::from("alacritty"), vec![OsString::from("-e")]),
        (OsString::from("xterm"), vec![OsString::from("-e")]),
    ]);

    let mut last_error = None;
    for (program, prefix) in candidates {
        match Command::new(&program)
            .args(&prefix)
            .args(command)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(program.to_string_lossy().into_owned()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => last_error = Some(error),
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to launch terminal {}", program.to_string_lossy())
                });
            }
        }
    }

    Err(last_error.unwrap_or_else(|| io::Error::other("no terminal candidates")))
        .context("no supported terminal emulator found")
}
