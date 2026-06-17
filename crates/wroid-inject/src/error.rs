use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UinputFrameError {
    #[error("uinput touch frame must contain at least one event")]
    EmptyFrame,
    #[error("contact {contact_id} is already assigned to a uinput slot")]
    ContactAlreadyActive { contact_id: u16 },
    #[error("contact {contact_id} has no active uinput slot")]
    ContactNotActive { contact_id: u16 },
    #[error("no free uinput slot is available for contact {contact_id}")]
    NoFreeSlot { contact_id: u16 },
    #[error("contact {contact_id} coordinate ({x}, {y}) exceeds touch surface {width}x{height}")]
    CoordinateOutOfRange {
        contact_id: u16,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    #[error("failed to emit uinput event batch")]
    Emit(#[source] io::Error),
}
