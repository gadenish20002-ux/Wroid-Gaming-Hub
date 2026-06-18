use thiserror::Error;
use wroid_core::{Point, Resolution};

use crate::{
    ContactId, TouchEngine, TouchEngineError, TouchEvent, TouchFrame, TouchInjector, TouchPhase,
};

/// Current digital direction state for one virtual joystick.
///
/// Opposing directions on the same axis cancel each other. A neutral state
/// releases an active joystick contact.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DirectionalInput {
    pub up: bool,
    pub left: bool,
    pub down: bool,
    pub right: bool,
}

impl DirectionalInput {
    pub const fn new(up: bool, left: bool, down: bool, right: bool) -> Self {
        Self {
            up,
            left,
            down,
            right,
        }
    }

    fn horizontal(self) -> i32 {
        axis(self.left, self.right)
    }

    fn vertical(self) -> i32 {
        axis(self.up, self.down)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VirtualJoystickConfigError {
    #[error("virtual joystick radius must be greater than zero")]
    ZeroRadius,
    #[error("virtual joystick resolution must be non-zero, got {width}x{height}")]
    InvalidResolution { width: u32, height: u32 },
    #[error("virtual joystick center {x},{y} is outside resolution {width}x{height}")]
    CenterOutOfBounds {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

/// Converts digital direction state into one persistent Android touch contact.
///
/// The first non-neutral input emits `Down`, direction changes emit `Move`, and
/// returning to neutral emits `Up`. Repeating the same direction emits no frame.
/// State is read from [`TouchEngine`], so a failed injection never advances the
/// joystick independently from the committed Android touch state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualJoystick {
    contact_id: ContactId,
    center: Point,
    radius: u32,
    resolution: Resolution,
}

impl VirtualJoystick {
    pub fn new(
        contact_id: ContactId,
        center: Point,
        radius: u32,
        resolution: Resolution,
    ) -> Result<Self, VirtualJoystickConfigError> {
        if radius == 0 {
            return Err(VirtualJoystickConfigError::ZeroRadius);
        }
        if resolution.width == 0 || resolution.height == 0 {
            return Err(VirtualJoystickConfigError::InvalidResolution {
                width: resolution.width,
                height: resolution.height,
            });
        }
        if center.x >= resolution.width || center.y >= resolution.height {
            return Err(VirtualJoystickConfigError::CenterOutOfBounds {
                x: center.x,
                y: center.y,
                width: resolution.width,
                height: resolution.height,
            });
        }

        Ok(Self {
            contact_id,
            center,
            radius,
            resolution,
        })
    }

    pub const fn contact_id(&self) -> ContactId {
        self.contact_id
    }

    pub const fn center(&self) -> Point {
        self.center
    }

    pub const fn radius(&self) -> u32 {
        self.radius
    }

    pub const fn resolution(&self) -> Resolution {
        self.resolution
    }

    /// Computes the clamped Android surface point for the supplied direction.
    pub fn target(&self, input: DirectionalInput) -> Option<Point> {
        let horizontal = input.horizontal();
        let vertical = input.vertical();
        if horizontal == 0 && vertical == 0 {
            return None;
        }

        let component = if horizontal != 0 && vertical != 0 {
            // 181 / 256 approximates 1 / sqrt(2), keeping diagonals within radius.
            ((u64::from(self.radius) * 181 + 128) / 256) as i64
        } else {
            i64::from(self.radius)
        };

        Some(Point {
            x: offset_axis(
                self.center.x,
                i64::from(horizontal) * component,
                self.resolution.width,
            ),
            y: offset_axis(
                self.center.y,
                i64::from(vertical) * component,
                self.resolution.height,
            ),
        })
    }

    /// Applies one direction state to a persistent touch engine.
    ///
    /// Returns `true` when a frame was submitted and `false` when the input did
    /// not change the committed contact state.
    pub fn apply<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        input: DirectionalInput,
    ) -> Result<bool, TouchEngineError> {
        let current = engine.state().position(self.contact_id);
        let target = self.target(input);

        match (current, target) {
            (None, None) => Ok(false),
            (None, Some(position)) => {
                engine.begin_contact(self.contact_id, position)?;
                Ok(true)
            }
            (Some(_), None) => {
                engine.end_contact(self.contact_id)?;
                Ok(true)
            }
            (Some(position), Some(target)) if position == target => Ok(false),
            (Some(_), Some(target)) => {
                engine.move_contact(self.contact_id, target)?;
                Ok(true)
            }
        }
    }

    /// Cancels the joystick contact after focus loss or abnormal session stop.
    pub fn cancel<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
    ) -> Result<bool, TouchEngineError> {
        let Some(position) = engine.state().position(self.contact_id) else {
            return Ok(false);
        };

        engine.submit(TouchFrame::single(TouchEvent::new(
            self.contact_id,
            TouchPhase::Cancel,
            position,
        )))?;
        Ok(true)
    }
}

fn axis(negative: bool, positive: bool) -> i32 {
    match (negative, positive) {
        (true, false) => -1,
        (false, true) => 1,
        (false, false) | (true, true) => 0,
    }
}

fn offset_axis(origin: u32, delta: i64, limit: u32) -> u32 {
    let maximum = i64::from(limit - 1);
    (i64::from(origin) + delta).clamp(0, maximum) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TouchFrame, TouchInjectionError};

    #[derive(Debug, Default)]
    struct RecordingInjector {
        frames: Vec<TouchFrame>,
        fail_next: bool,
    }

    impl TouchInjector for RecordingInjector {
        fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
            if self.fail_next {
                self.fail_next = false;
                return Err(TouchInjectionError::new("injected failure"));
            }
            self.frames.push(frame.clone());
            Ok(())
        }
    }

    fn joystick() -> VirtualJoystick {
        VirtualJoystick::new(
            ContactId::new(7),
            Point { x: 500, y: 500 },
            100,
            Resolution {
                width: 1000,
                height: 1000,
            },
        )
        .unwrap()
    }

    #[test]
    fn holds_one_contact_across_direction_changes() {
        let joystick = joystick();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert!(joystick
            .apply(
                &mut engine,
                DirectionalInput::new(false, false, false, true),
            )
            .unwrap());
        assert!(!joystick
            .apply(
                &mut engine,
                DirectionalInput::new(false, false, false, true),
            )
            .unwrap());
        assert!(joystick
            .apply(&mut engine, DirectionalInput::new(true, false, false, true),)
            .unwrap());
        assert!(joystick
            .apply(&mut engine, DirectionalInput::default())
            .unwrap());

        let frames = &engine.injector().frames;
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].events()[0].phase, TouchPhase::Down);
        assert_eq!(frames[0].events()[0].position, Point { x: 600, y: 500 });
        assert_eq!(frames[1].events()[0].phase, TouchPhase::Move);
        assert_eq!(frames[1].events()[0].position, Point { x: 571, y: 429 });
        assert_eq!(frames[2].events()[0].phase, TouchPhase::Up);
        assert!(!engine.state().is_active(joystick.contact_id()));
    }

    #[test]
    fn clamps_targets_to_the_android_surface() {
        let joystick = VirtualJoystick::new(
            ContactId::new(1),
            Point { x: 5, y: 5 },
            100,
            Resolution {
                width: 200,
                height: 200,
            },
        )
        .unwrap();

        assert_eq!(
            joystick.target(DirectionalInput::new(true, true, false, false)),
            Some(Point { x: 0, y: 0 })
        );
    }

    #[test]
    fn opposing_directions_are_neutral() {
        let joystick = joystick();
        let input = DirectionalInput::new(true, true, true, true);

        assert_eq!(joystick.target(input), None);
    }

    #[test]
    fn failed_down_does_not_activate_the_contact() {
        let joystick = joystick();
        let injector = RecordingInjector {
            fail_next: true,
            ..RecordingInjector::default()
        };
        let mut engine = TouchEngine::new(injector);

        let error = joystick
            .apply(
                &mut engine,
                DirectionalInput::new(false, false, false, true),
            )
            .unwrap_err();

        assert!(matches!(error, TouchEngineError::Injection(_)));
        assert!(!engine.state().is_active(joystick.contact_id()));
        assert!(engine.injector().frames.is_empty());
    }

    #[test]
    fn focus_loss_uses_cancel_instead_of_up() {
        let joystick = joystick();
        let mut engine = TouchEngine::new(RecordingInjector::default());
        joystick
            .apply(
                &mut engine,
                DirectionalInput::new(false, false, true, false),
            )
            .unwrap();

        assert!(joystick.cancel(&mut engine).unwrap());
        assert!(!joystick.cancel(&mut engine).unwrap());
        assert_eq!(
            engine.injector().frames.last().unwrap().events()[0].phase,
            TouchPhase::Cancel
        );
        assert!(!engine.state().is_active(joystick.contact_id()));
    }

    #[test]
    fn rejects_invalid_geometry() {
        assert!(matches!(
            VirtualJoystick::new(
                ContactId::new(1),
                Point { x: 0, y: 0 },
                0,
                Resolution {
                    width: 100,
                    height: 100,
                },
            ),
            Err(VirtualJoystickConfigError::ZeroRadius)
        ));
        assert!(matches!(
            VirtualJoystick::new(
                ContactId::new(1),
                Point { x: 100, y: 0 },
                10,
                Resolution {
                    width: 100,
                    height: 100,
                },
            ),
            Err(VirtualJoystickConfigError::CenterOutOfBounds { .. })
        ));
    }
}
