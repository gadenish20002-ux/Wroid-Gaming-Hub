use std::io;

use wroid_core::Point;
use wroid_runtime::{ContactId, TouchEvent, TouchPhase};

use crate::{DeviceConfig, EventSink, LinuxInputEvent};

mod protocol;
mod validation;

#[derive(Debug, Default)]
pub(super) struct RecordingSink {
    pub(super) frames: Vec<Vec<LinuxInputEvent>>,
    pub(super) fail_next: bool,
}

impl EventSink for RecordingSink {
    fn emit(&mut self, events: &[LinuxInputEvent]) -> io::Result<()> {
        if self.fail_next {
            self.fail_next = false;
            return Err(io::Error::other("injected sink failure"));
        }
        self.frames.push(events.to_vec());
        Ok(())
    }
}

pub(super) fn event(contact_id: u16, phase: TouchPhase, x: u32, y: u32) -> TouchEvent {
    TouchEvent::new(ContactId::new(contact_id), phase, Point { x, y })
}

pub(super) fn config(slots: u16) -> DeviceConfig {
    DeviceConfig::with_slots(1920, 1080, slots).unwrap()
}
