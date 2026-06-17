use wroid_runtime::{ContactId, TouchFrame, TouchPhase};

use crate::state::{Slot, SlotState};
use crate::{EventSink, UinputFrameError, UinputTouchInjector};

impl<S: EventSink> UinputTouchInjector<S> {
    pub(crate) fn next_state(&self, frame: &TouchFrame) -> Result<SlotState, UinputFrameError> {
        let mut next = self.state;

        for event in frame.events() {
            match event.phase {
                TouchPhase::Down => {
                    self.validate_position(event.contact_id, event.position.x, event.position.y)?;
                    if next.active_slot(event.contact_id).is_some() {
                        return Err(UinputFrameError::ContactAlreadyActive {
                            contact_id: event.contact_id.get(),
                        });
                    }
                    let slot = next.free_slot().ok_or(UinputFrameError::NoFreeSlot {
                        contact_id: event.contact_id.get(),
                    })?;
                    next.slots[slot] = Slot {
                        contact_id: Some(event.contact_id),
                        x: event.position.x,
                        y: event.position.y,
                    };
                    next.active_count += 1;
                }
                TouchPhase::Move => {
                    self.validate_position(event.contact_id, event.position.x, event.position.y)?;
                    let slot = next.active_slot(event.contact_id).ok_or(
                        UinputFrameError::ContactNotActive {
                            contact_id: event.contact_id.get(),
                        },
                    )?;
                    next.slots[slot].x = event.position.x;
                    next.slots[slot].y = event.position.y;
                }
                TouchPhase::Up | TouchPhase::Cancel => {
                    let slot = next.active_slot(event.contact_id).ok_or(
                        UinputFrameError::ContactNotActive {
                            contact_id: event.contact_id.get(),
                        },
                    )?;
                    next.slots[slot] = Slot::default();
                    next.active_count -= 1;
                }
            }
        }

        Ok(next)
    }

    fn validate_position(
        &self,
        contact_id: ContactId,
        x: u32,
        y: u32,
    ) -> Result<(), UinputFrameError> {
        if x >= self.config.width || y >= self.config.height {
            return Err(UinputFrameError::CoordinateOutOfRange {
                contact_id: contact_id.get(),
                x,
                y,
                width: self.config.width,
                height: self.config.height,
            });
        }
        Ok(())
    }
}
