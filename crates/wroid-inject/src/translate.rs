use evdev::{AbsoluteAxisCode, KeyCode};
use wroid_runtime::{TouchFrame, TouchPhase};

use crate::{EventSink, LinuxInputEvent, UinputFrameError, UinputTouchInjector};

impl<S: EventSink> UinputTouchInjector<S> {
    pub fn inject_frame(&mut self, frame: &TouchFrame) -> Result<(), UinputFrameError> {
        if frame.is_empty() {
            return Err(UinputFrameError::EmptyFrame);
        }

        let current = self.state;
        let next = self.next_state(frame)?;

        self.scratch.clear();
        if current.active_count == 0 && next.active_count > 0 {
            self.scratch
                .push(LinuxInputEvent::key(KeyCode::BTN_TOUCH, 1));
        } else if current.active_count > 0 && next.active_count == 0 {
            self.scratch
                .push(LinuxInputEvent::key(KeyCode::BTN_TOUCH, 0));
        }

        for event in frame.events() {
            let slot = match event.phase {
                TouchPhase::Down => next.active_slot(event.contact_id),
                TouchPhase::Move => current
                    .active_slot(event.contact_id)
                    .or_else(|| next.active_slot(event.contact_id)),
                TouchPhase::Up | TouchPhase::Cancel => current.active_slot(event.contact_id),
            }
            .ok_or(UinputFrameError::ContactNotActive {
                contact_id: event.contact_id.get(),
            })?;

            self.scratch.push(LinuxInputEvent::absolute(
                AbsoluteAxisCode::ABS_MT_SLOT,
                slot as i32,
            ));

            match event.phase {
                TouchPhase::Down => {
                    self.scratch.push(LinuxInputEvent::absolute(
                        AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                        i32::from(event.contact_id.get()),
                    ));
                    self.push_mt_position(event.position.x, event.position.y);
                }
                TouchPhase::Move => {
                    self.push_mt_position(event.position.x, event.position.y);
                }
                TouchPhase::Up | TouchPhase::Cancel => {
                    self.scratch.push(LinuxInputEvent::absolute(
                        AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                        -1,
                    ));
                }
            }
        }

        if let Some((x, y)) = next.primary_position() {
            self.scratch
                .push(LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_X, x as i32));
            self.scratch
                .push(LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_Y, y as i32));
        }

        self.sink
            .emit(&self.scratch)
            .map_err(UinputFrameError::Emit)?;
        self.state = next;
        Ok(())
    }

    fn push_mt_position(&mut self, x: u32, y: u32) {
        self.scratch.push(LinuxInputEvent::absolute(
            AbsoluteAxisCode::ABS_MT_POSITION_X,
            x as i32,
        ));
        self.scratch.push(LinuxInputEvent::absolute(
            AbsoluteAxisCode::ABS_MT_POSITION_Y,
            y as i32,
        ));
    }
}
