use thiserror::Error;
use wroid_core::{Point, Resolution};

use crate::{
    ContactId, TouchEngine, TouchEngineError, TouchEvent, TouchFrame, TouchInjector, TouchPhase,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseAimDelta {
    pub dx: i32,
    pub dy: i32,
}

impl MouseAimDelta {
    pub const fn new(dx: i32, dy: i32) -> Self {
        Self { dx, dy }
    }

    fn is_zero(self) -> bool {
        self.dx == 0 && self.dy == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseAimSensitivity {
    numerator: u32,
    denominator: u32,
}

impl MouseAimSensitivity {
    pub const fn one_to_one() -> Self {
        Self {
            numerator: 1,
            denominator: 1,
        }
    }

    pub fn new(numerator: u32, denominator: u32) -> Result<Self, MouseAimConfigError> {
        if numerator == 0 || denominator == 0 {
            return Err(MouseAimConfigError::InvalidSensitivity {
                numerator,
                denominator,
            });
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    fn scale(self, delta: i32) -> i64 {
        (i64::from(delta) * i64::from(self.numerator)) / i64::from(self.denominator)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseAimRegion {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl MouseAimRegion {
    pub fn full_surface(resolution: Resolution) -> Result<Self, MouseAimConfigError> {
        if resolution.width == 0 || resolution.height == 0 {
            return Err(MouseAimConfigError::InvalidResolution {
                width: resolution.width,
                height: resolution.height,
            });
        }
        Ok(Self {
            left: 0,
            top: 0,
            right: resolution.width - 1,
            bottom: resolution.height - 1,
        })
    }

    pub fn validate(self, resolution: Resolution) -> Result<Self, MouseAimConfigError> {
        if resolution.width == 0 || resolution.height == 0 {
            return Err(MouseAimConfigError::InvalidResolution {
                width: resolution.width,
                height: resolution.height,
            });
        }
        if self.left > self.right
            || self.top > self.bottom
            || self.right >= resolution.width
            || self.bottom >= resolution.height
        {
            return Err(MouseAimConfigError::InvalidRegion {
                left: self.left,
                top: self.top,
                right: self.right,
                bottom: self.bottom,
                width: resolution.width,
                height: resolution.height,
            });
        }
        Ok(self)
    }

    fn contains(self, point: Point) -> bool {
        point.x >= self.left
            && point.x <= self.right
            && point.y >= self.top
            && point.y <= self.bottom
    }

    fn clamp(self, point: Point) -> Point {
        Point {
            x: point.x.clamp(self.left, self.right),
            y: point.y.clamp(self.top, self.bottom),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MouseAimConfigError {
    #[error("mouse aim resolution must be non-zero, got {width}x{height}")]
    InvalidResolution { width: u32, height: u32 },
    #[error(
        "mouse aim region {left},{top}-{right},{bottom} is invalid for resolution {width}x{height}"
    )]
    InvalidRegion {
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
        width: u32,
        height: u32,
    },
    #[error("mouse aim origin {x},{y} is outside region {left},{top}-{right},{bottom}")]
    OriginOutOfRegion {
        x: u32,
        y: u32,
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
    },
    #[error("mouse aim sensitivity must be non-zero, got {numerator}/{denominator}")]
    InvalidSensitivity { numerator: u32, denominator: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseAim {
    contact_id: ContactId,
    origin: Point,
    region: MouseAimRegion,
    sensitivity: MouseAimSensitivity,
}

impl MouseAim {
    pub fn new(
        contact_id: ContactId,
        origin: Point,
        region: MouseAimRegion,
        resolution: Resolution,
        sensitivity: MouseAimSensitivity,
    ) -> Result<Self, MouseAimConfigError> {
        let region = region.validate(resolution)?;
        if !region.contains(origin) {
            return Err(MouseAimConfigError::OriginOutOfRegion {
                x: origin.x,
                y: origin.y,
                left: region.left,
                top: region.top,
                right: region.right,
                bottom: region.bottom,
            });
        }
        Ok(Self {
            contact_id,
            origin,
            region,
            sensitivity,
        })
    }

    pub const fn contact_id(&self) -> ContactId {
        self.contact_id
    }

    pub const fn origin(&self) -> Point {
        self.origin
    }

    pub const fn region(&self) -> MouseAimRegion {
        self.region
    }

    pub const fn sensitivity(&self) -> MouseAimSensitivity {
        self.sensitivity
    }

    pub fn begin<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
    ) -> Result<bool, TouchEngineError> {
        if engine.state().is_active(self.contact_id) {
            return Ok(false);
        }
        engine.begin_contact(self.contact_id, self.origin)?;
        Ok(true)
    }

    pub fn move_by<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        delta: MouseAimDelta,
    ) -> Result<bool, TouchEngineError> {
        if delta.is_zero() {
            return Ok(false);
        }
        let Some(current) = engine.state().position(self.contact_id) else {
            return Ok(false);
        };

        let target = self.region.clamp(offset_point(
            current,
            self.sensitivity.scale(delta.dx),
            self.sensitivity.scale(delta.dy),
        ));
        if target == current {
            return Ok(false);
        }

        engine.move_contact(self.contact_id, target)?;
        Ok(true)
    }

    pub fn end<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
    ) -> Result<bool, TouchEngineError> {
        if !engine.state().is_active(self.contact_id) {
            return Ok(false);
        }
        engine.end_contact(self.contact_id)?;
        Ok(true)
    }

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

fn offset_point(point: Point, dx: i64, dy: i64) -> Point {
    Point {
        x: offset_axis(point.x, dx),
        y: offset_axis(point.y, dy),
    }
}

fn offset_axis(value: u32, delta: i64) -> u32 {
    let value = i64::from(value) + delta;
    if value <= 0 {
        0
    } else if value >= i64::from(u32::MAX) {
        u32::MAX
    } else {
        value as u32
    }
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

    fn resolution() -> Resolution {
        Resolution {
            width: 1920,
            height: 1080,
        }
    }

    fn aim() -> MouseAim {
        MouseAim::new(
            ContactId::new(9),
            Point { x: 960, y: 540 },
            MouseAimRegion {
                left: 640,
                top: 240,
                right: 1600,
                bottom: 840,
            },
            resolution(),
            MouseAimSensitivity::one_to_one(),
        )
        .unwrap()
    }

    #[test]
    fn begins_and_moves_one_persistent_aim_contact() {
        let aim = aim();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert!(aim.begin(&mut engine).unwrap());
        assert!(!aim.begin(&mut engine).unwrap());
        assert!(aim
            .move_by(&mut engine, MouseAimDelta::new(25, -10))
            .unwrap());

        assert_eq!(engine.state().position(aim.contact_id()), Some(Point { x: 985, y: 530 }));
        let frames = &engine.injector().frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].events()[0].phase, TouchPhase::Down);
        assert_eq!(frames[1].events()[0].phase, TouchPhase::Move);
    }

    #[test]
    fn ignores_motion_before_activation_and_zero_motion() {
        let aim = aim();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert!(!aim
            .move_by(&mut engine, MouseAimDelta::new(30, 30))
            .unwrap());
        assert!(aim.begin(&mut engine).unwrap());
        assert!(!aim
            .move_by(&mut engine, MouseAimDelta::new(0, 0))
            .unwrap());

        assert_eq!(engine.injector().frames.len(), 1);
    }

    #[test]
    fn applies_sensitivity_and_clamps_to_region() {
        let aim = MouseAim::new(
            ContactId::new(5),
            Point { x: 100, y: 100 },
            MouseAimRegion {
                left: 50,
                top: 50,
                right: 150,
                bottom: 150,
            },
            Resolution {
                width: 200,
                height: 200,
            },
            MouseAimSensitivity::new(2, 1).unwrap(),
        )
        .unwrap();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert!(aim.begin(&mut engine).unwrap());
        assert!(aim
            .move_by(&mut engine, MouseAimDelta::new(100, -100))
            .unwrap());

        assert_eq!(engine.state().position(aim.contact_id()), Some(Point { x: 150, y: 50 }));
    }

    #[test]
    fn end_releases_and_focus_loss_cancels() {
        let aim = aim();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert!(!aim.end(&mut engine).unwrap());
        assert!(aim.begin(&mut engine).unwrap());
        assert!(aim.end(&mut engine).unwrap());
        assert!(!engine.state().is_active(aim.contact_id()));

        assert!(aim.begin(&mut engine).unwrap());
        assert!(aim.cancel(&mut engine).unwrap());
        assert!(!aim.cancel(&mut engine).unwrap());
        assert_eq!(
            engine.injector().frames.last().unwrap().events()[0].phase,
            TouchPhase::Cancel
        );
    }

    #[test]
    fn failed_begin_does_not_activate_contact() {
        let aim = aim();
        let injector = RecordingInjector {
            fail_next: true,
            ..RecordingInjector::default()
        };
        let mut engine = TouchEngine::new(injector);

        let error = aim.begin(&mut engine).unwrap_err();

        assert!(matches!(error, TouchEngineError::Injection(_)));
        assert!(!engine.state().is_active(aim.contact_id()));
        assert!(engine.injector().frames.is_empty());
    }

    #[test]
    fn rejects_invalid_geometry_and_sensitivity() {
        assert!(matches!(
            MouseAimRegion::full_surface(Resolution {
                width: 0,
                height: 100
            }),
            Err(MouseAimConfigError::InvalidResolution { .. })
        ));
        assert!(matches!(
            MouseAimSensitivity::new(0, 1),
            Err(MouseAimConfigError::InvalidSensitivity { .. })
        ));
        assert!(matches!(
            MouseAim::new(
                ContactId::new(1),
                Point { x: 10, y: 10 },
                MouseAimRegion {
                    left: 20,
                    top: 20,
                    right: 30,
                    bottom: 30,
                },
                Resolution {
                    width: 100,
                    height: 100,
                },
                MouseAimSensitivity::one_to_one(),
            ),
            Err(MouseAimConfigError::OriginOutOfRegion { .. })
        ));
    }
}
