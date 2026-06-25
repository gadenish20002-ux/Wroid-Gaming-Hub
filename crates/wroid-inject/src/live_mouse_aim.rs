use wroid_input::mouse::{
    MouseButton, MouseButtonEvent, MouseButtonTransition, MouseEvent, RelativeMouseMotion,
};
use wroid_runtime::{MouseAim, MouseAimDelta, TouchEngine, TouchEngineError, TouchInjector};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAimAction {
    Activated,
    Moved,
    Released,
    Cancelled,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseAimBinding {
    pub activation_button: MouseButton,
}

impl Default for MouseAimBinding {
    fn default() -> Self {
        Self {
            activation_button: MouseButton::Right,
        }
    }
}

pub struct MouseAimController {
    aim: MouseAim,
    binding: MouseAimBinding,
}

impl MouseAimController {
    pub const fn new(aim: MouseAim, binding: MouseAimBinding) -> Self {
        Self { aim, binding }
    }

    pub const fn aim(&self) -> &MouseAim {
        &self.aim
    }

    pub const fn binding(&self) -> MouseAimBinding {
        self.binding
    }

    pub fn handle_event<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        event: MouseEvent,
    ) -> Result<MouseAimAction, TouchEngineError> {
        match event {
            MouseEvent::Button(event) => self.handle_button(engine, event),
            MouseEvent::Motion(motion) => self.handle_motion(engine, motion),
            MouseEvent::Wheel(_) => Ok(MouseAimAction::Ignored),
        }
    }

    pub fn cancel<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
    ) -> Result<MouseAimAction, TouchEngineError> {
        if self.aim.cancel(engine)? {
            Ok(MouseAimAction::Cancelled)
        } else {
            Ok(MouseAimAction::Ignored)
        }
    }

    fn handle_button<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        event: MouseButtonEvent,
    ) -> Result<MouseAimAction, TouchEngineError> {
        if event.button != self.binding.activation_button {
            return Ok(MouseAimAction::Ignored);
        }

        match event.transition {
            MouseButtonTransition::Pressed => {
                if self.aim.begin(engine)? {
                    Ok(MouseAimAction::Activated)
                } else {
                    Ok(MouseAimAction::Ignored)
                }
            }
            MouseButtonTransition::Released => {
                if self.aim.end(engine)? {
                    Ok(MouseAimAction::Released)
                } else {
                    Ok(MouseAimAction::Ignored)
                }
            }
            MouseButtonTransition::Repeated => Ok(MouseAimAction::Ignored),
        }
    }

    fn handle_motion<I: TouchInjector>(
        &self,
        engine: &mut TouchEngine<I>,
        motion: RelativeMouseMotion,
    ) -> Result<MouseAimAction, TouchEngineError> {
        if self
            .aim
            .move_by(engine, MouseAimDelta::new(motion.dx, motion.dy))?
        {
            Ok(MouseAimAction::Moved)
        } else {
            Ok(MouseAimAction::Ignored)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_core::{Point, Resolution};
    use wroid_runtime::{
        ContactId, MouseAimRegion, MouseAimSensitivity, TouchFrame, TouchInjectionError, TouchPhase,
    };

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

    fn controller() -> MouseAimController {
        MouseAimController::new(
            MouseAim::new(
                ContactId::new(11),
                Point { x: 960, y: 540 },
                MouseAimRegion {
                    left: 600,
                    top: 200,
                    right: 1500,
                    bottom: 900,
                },
                Resolution {
                    width: 1920,
                    height: 1080,
                },
                MouseAimSensitivity::one_to_one(),
            )
            .unwrap(),
            MouseAimBinding::default(),
        )
    }

    fn button(button: MouseButton, transition: MouseButtonTransition) -> MouseEvent {
        MouseEvent::Button(MouseButtonEvent::new(button, transition))
    }

    #[test]
    fn right_button_hold_activates_moves_and_releases_aim_contact() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Activated
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    MouseEvent::Motion(RelativeMouseMotion::new(10, -5)),
                )
                .unwrap(),
            MouseAimAction::Moved
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Released),
                )
                .unwrap(),
            MouseAimAction::Released
        );

        let frames = &engine.injector().frames;
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].events()[0].phase, TouchPhase::Down);
        assert_eq!(frames[1].events()[0].phase, TouchPhase::Move);
        assert_eq!(frames[2].events()[0].phase, TouchPhase::Up);
        assert!(!engine.state().is_active(controller.aim().contact_id()));
    }

    #[test]
    fn motion_before_activation_and_other_buttons_are_ignored() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    MouseEvent::Motion(RelativeMouseMotion::new(10, 10)),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Left, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );

        assert!(engine.injector().frames.is_empty());
    }

    #[test]
    fn repeated_activation_and_zero_motion_are_ignored() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Activated
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    button(MouseButton::Right, MouseButtonTransition::Pressed),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );
        assert_eq!(
            controller
                .handle_event(
                    &mut engine,
                    MouseEvent::Motion(RelativeMouseMotion::new(0, 0)),
                )
                .unwrap(),
            MouseAimAction::Ignored
        );

        assert_eq!(engine.injector().frames.len(), 1);
    }

    #[test]
    fn focus_loss_cancels_active_aim_contact() {
        let controller = controller();
        let mut engine = TouchEngine::new(RecordingInjector::default());

        controller
            .handle_event(
                &mut engine,
                button(MouseButton::Right, MouseButtonTransition::Pressed),
            )
            .unwrap();

        assert_eq!(
            controller.cancel(&mut engine).unwrap(),
            MouseAimAction::Cancelled
        );
        assert_eq!(
            controller.cancel(&mut engine).unwrap(),
            MouseAimAction::Ignored
        );
        assert_eq!(
            engine.injector().frames.last().unwrap().events()[0].phase,
            TouchPhase::Cancel
        );
    }

    #[test]
    fn backend_failure_does_not_activate_contact() {
        let controller = controller();
        let injector = RecordingInjector {
            fail_next: true,
            ..RecordingInjector::default()
        };
        let mut engine = TouchEngine::new(injector);

        let error = controller
            .handle_event(
                &mut engine,
                button(MouseButton::Right, MouseButtonTransition::Pressed),
            )
            .unwrap_err();

        assert!(matches!(error, TouchEngineError::Injection(_)));
        assert!(!engine.state().is_active(controller.aim().contact_id()));
    }
}
