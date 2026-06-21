use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;

#[path = "../mouse.rs"]
mod mouse;

use mouse::{EvdevMouse, MouseButtonTransition, MouseEvent};

fn main() -> Result<(), Box<dyn Error>> {
    let Some(options) = Options::parse(env::args().skip(1))? else {
        print_usage();
        return Ok(());
    };

    let mut mouse = EvdevMouse::open(&options.mouse_path)?;
    if options.grab {
        mouse.grab()?;
    }

    println!("Mouse: {} ({})", mouse.name(), mouse.path().display());
    println!(
        "Relative capture active. Exclusive grab: {}.",
        if mouse.is_grabbed() {
            "enabled"
        } else {
            "disabled"
        },
    );

    let mut seen = 0_usize;
    loop {
        for event in mouse.next_events()? {
            print_event(event);
            seen += 1;
            if options.max_events.is_some_and(|limit| seen >= limit) {
                return Ok(());
            }
        }
    }
}

#[derive(Debug)]
struct Options {
    mouse_path: PathBuf,
    grab: bool,
    max_events: Option<usize>,
}

impl Options {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Option<Self>, Box<dyn Error>> {
        let mut path = None;
        let mut grab = false;
        let mut max_events = None;
        let mut args = args.into_iter();

        while let Some(argument) = args.next() {
            match argument.as_str() {
                "--help" | "-h" => return Ok(None),
                "--grab" => grab = true,
                "--no-grab" => grab = false,
                "--max-events" => {
                    let value = args.next().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "missing value after --max-events",
                        )
                    })?;
                    let count = value.parse::<usize>().map_err(|source| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            format!("invalid --max-events value {value}: {source}"),
                        )
                    })?;
                    if count == 0 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "--max-events must be greater than zero",
                        )
                        .into());
                    }
                    max_events = Some(count);
                }
                value if value.starts_with("--") => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown option: {value}"),
                    )
                    .into());
                }
                value => {
                    if path.replace(PathBuf::from(value)).is_some() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "only one mouse event-device path is supported",
                        )
                        .into());
                    }
                }
            }
        }

        let mouse_path = path.ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing mouse event-device path")
        })?;

        Ok(Some(Self {
            mouse_path,
            grab,
            max_events,
        }))
    }
}

fn print_event(event: MouseEvent) {
    match event {
        MouseEvent::Motion(motion) => {
            println!("motion dx={} dy={}", motion.dx, motion.dy);
        }
        MouseEvent::Wheel(wheel) => {
            println!(
                "wheel vertical={} horizontal={}",
                wheel.vertical, wheel.horizontal
            );
        }
        MouseEvent::Button(button) => {
            println!(
                "button {} {}",
                button.button.profile_name(),
                transition_label(button.transition)
            );
        }
    }
}

fn transition_label(transition: MouseButtonTransition) -> &'static str {
    match transition {
        MouseButtonTransition::Pressed => "pressed",
        MouseButtonTransition::Released => "released",
        MouseButtonTransition::Repeated => "repeated",
    }
}

fn print_usage() {
    println!("Usage: wroid-mouse-capture [--grab|--no-grab] [--max-events N] <mouse-event-device>");
}
