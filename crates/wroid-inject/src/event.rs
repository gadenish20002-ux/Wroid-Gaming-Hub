use std::io;

use evdev::{AbsoluteAxisCode, EventType, KeyCode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinuxInputEvent {
    pub event_type: u16,
    pub code: u16,
    pub value: i32,
}

impl LinuxInputEvent {
    const fn new(event_type: u16, code: u16, value: i32) -> Self {
        Self {
            event_type,
            code,
            value,
        }
    }

    pub(crate) fn absolute(code: AbsoluteAxisCode, value: i32) -> Self {
        Self::new(EventType::ABSOLUTE.0, code.0, value)
    }

    pub(crate) fn key(code: KeyCode, value: i32) -> Self {
        Self::new(EventType::KEY.0, code.0, value)
    }
}

/// Receives one complete Linux input frame.
///
/// Production implementations must keep their transport open across calls.
pub trait EventSink {
    fn emit(&mut self, events: &[LinuxInputEvent]) -> io::Result<()>;
}
