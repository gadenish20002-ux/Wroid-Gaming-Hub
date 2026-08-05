use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;

use wroid_inject::{run_privileged_bridge_helper, run_privileged_bridge_helper_check};

fn main() -> io::Result<()> {
    match parse_command(std::env::args_os().skip(1))? {
        HelperCommand::Bridge(event_node) => run_privileged_bridge_helper(event_node),
        HelperCommand::Check => run_privileged_bridge_helper_check(),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum HelperCommand {
    Bridge(PathBuf),
    Check,
}

fn parse_command(arguments: impl IntoIterator<Item = OsString>) -> io::Result<HelperCommand> {
    let mut arguments = arguments.into_iter();
    let operation = arguments.next();
    if operation.as_deref() == Some(OsStr::new("--check")) && arguments.next().is_none() {
        return Ok(HelperCommand::Check);
    }
    if operation.as_deref() != Some(OsStr::new("--event-node")) {
        return Err(invalid_usage());
    }
    let event_node = arguments.next().ok_or_else(invalid_usage)?;
    if arguments.next().is_some() {
        return Err(invalid_usage());
    }
    Ok(HelperCommand::Bridge(PathBuf::from(event_node)))
}

fn invalid_usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: wroid-helper --check | --event-node /dev/input/eventN",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_typed_event_node_operation() {
        assert_eq!(
            parse_command(["--event-node", "/dev/input/event42"].map(OsString::from)).unwrap(),
            HelperCommand::Bridge(PathBuf::from("/dev/input/event42"))
        );
        assert_eq!(
            parse_command(["--check"].map(OsString::from)).unwrap(),
            HelperCommand::Check
        );
        assert!(parse_command(["--event-node"].map(OsString::from)).is_err());
        assert!(parse_command(
            ["--event-node", "/dev/input/event42", "--extra"].map(OsString::from)
        )
        .is_err());
        assert!(parse_command(["--check", "extra"].map(OsString::from)).is_err());
        assert!(parse_command(["sh", "-c", "id"].map(OsString::from)).is_err());
    }
}
