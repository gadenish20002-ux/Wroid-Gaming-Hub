use thiserror::Error;
use wroid_core::profile_v2::{ActionV2, BindingV2, InputV2, ProfileV2, ProfileV2ValidationError};
use wroid_core::{Point, Resolution};

use crate::{ContactId, VirtualJoystick, VirtualJoystickConfigError};

/// Runtime-ready controls materialized from a profile v2 document.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeControlPlan {
    pub profile_name: String,
    pub package_name: String,
    pub resolution: Resolution,
    pub controls: Vec<RuntimeControlBinding>,
}

impl RuntimeControlPlan {
    pub fn from_profile_v2(
        profile: &ProfileV2,
        resolution: Resolution,
    ) -> Result<Self, RuntimeControlPlanError> {
        profile.validate()?;

        let mut next_contact_id = 1_u16;
        let mut controls = Vec::with_capacity(profile.bindings.len());
        for binding in &profile.bindings {
            let action = materialize_action(binding, resolution, &mut next_contact_id)?;
            controls.push(RuntimeControlBinding {
                name: binding.name.clone(),
                input: binding.input.clone(),
                action,
            });
        }

        Ok(Self {
            profile_name: profile.name.clone(),
            package_name: profile.package_name.clone(),
            resolution,
            controls,
        })
    }

    pub fn control(&self, name: &str) -> Option<&RuntimeControlBinding> {
        self.controls.iter().find(|control| control.name == name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeControlBinding {
    pub name: String,
    pub input: InputV2,
    pub action: RuntimeControlAction,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeControlAction {
    Tap { point: Point },
    VirtualJoystick { joystick: VirtualJoystick },
}

#[derive(Debug, Error)]
pub enum RuntimeControlPlanError {
    #[error(transparent)]
    InvalidProfile(#[from] ProfileV2ValidationError),
    #[error("binding {binding} uses unsupported runtime action kind: {kind}")]
    UnsupportedAction { binding: String, kind: &'static str },
    #[error("binding {binding} has invalid virtual joystick geometry: {source}")]
    InvalidVirtualJoystick {
        binding: String,
        #[source]
        source: VirtualJoystickConfigError,
    },
}

fn materialize_action(
    binding: &BindingV2,
    resolution: Resolution,
    next_contact_id: &mut u16,
) -> Result<RuntimeControlAction, RuntimeControlPlanError> {
    match &binding.action {
        ActionV2::Tap { point } => Ok(RuntimeControlAction::Tap {
            point: point.materialize(resolution),
        }),
        ActionV2::VirtualJoystick {
            center,
            radius,
            dead_zone,
            ..
        } => {
            let contact_id = ContactId::new(*next_contact_id);
            *next_contact_id = next_contact_id.saturating_add(1);
            let joystick = VirtualJoystick::from_profile_v2_geometry(
                contact_id,
                *center,
                *radius,
                *dead_zone,
                resolution,
            )
            .map_err(|source| RuntimeControlPlanError::InvalidVirtualJoystick {
                binding: binding.name.clone(),
                source,
            })?;
            Ok(RuntimeControlAction::VirtualJoystick { joystick })
        }
        ActionV2::MouseAim { .. } => Err(RuntimeControlPlanError::UnsupportedAction {
            binding: binding.name.clone(),
            kind: "mouse_aim",
        }),
        ActionV2::Macro { .. } => Err(RuntimeControlPlanError::UnsupportedAction {
            binding: binding.name.clone(),
            kind: "macro",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_core::profile_v2::{JoystickMode, NormalizedPoint};

    fn profile() -> ProfileV2 {
        ProfileV2 {
            schema_version: 2,
            name: "Shooter v2".to_owned(),
            package_name: "com.example.shooter".to_owned(),
            orientation: Default::default(),
            bindings: vec![
                BindingV2 {
                    name: "movement".to_owned(),
                    input: InputV2::KeyCluster {
                        up: "w".to_owned(),
                        left: "a".to_owned(),
                        down: "s".to_owned(),
                        right: "d".to_owned(),
                    },
                    action: ActionV2::VirtualJoystick {
                        center: NormalizedPoint { x: 0.18, y: 0.78 },
                        radius: 0.09,
                        dead_zone: 0.02,
                        mode: JoystickMode::Hold,
                        reaffirm_ms: Some(50),
                    },
                },
                BindingV2 {
                    name: "fire".to_owned(),
                    input: InputV2::MouseButton {
                        button: "left".to_owned(),
                    },
                    action: ActionV2::Tap {
                        point: NormalizedPoint { x: 0.86, y: 0.50 },
                    },
                },
            ],
        }
    }

    #[test]
    fn materializes_supported_profile_v2_controls() {
        let resolution = Resolution {
            width: 1920,
            height: 1080,
        };

        let plan = RuntimeControlPlan::from_profile_v2(&profile(), resolution).unwrap();

        assert_eq!(plan.profile_name, "Shooter v2");
        assert_eq!(plan.package_name, "com.example.shooter");
        assert_eq!(plan.resolution, resolution);
        assert_eq!(plan.controls.len(), 2);

        let movement = plan.control("movement").unwrap();
        let RuntimeControlAction::VirtualJoystick { joystick } = &movement.action else {
            panic!("movement should materialize as a virtual joystick");
        };
        assert_eq!(joystick.contact_id(), ContactId::new(1));
        assert_eq!(joystick.center(), Point { x: 345, y: 842 });
        assert_eq!(joystick.radius(), 97);
        assert_eq!(joystick.dead_zone(), 22);

        let fire = plan.control("fire").unwrap();
        assert_eq!(
            fire.action,
            RuntimeControlAction::Tap {
                point: Point { x: 1650, y: 540 }
            }
        );
    }

    #[test]
    fn rejects_unsupported_profile_v2_controls() {
        let mut profile = profile();
        profile.bindings[0].action = ActionV2::MouseAim {
            region: wroid_core::profile_v2::NormalizedRect {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            sensitivity: 1.0,
        };

        let error = RuntimeControlPlan::from_profile_v2(
            &profile,
            Resolution {
                width: 1920,
                height: 1080,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            RuntimeControlPlanError::UnsupportedAction {
                binding,
                kind: "mouse_aim"
            } if binding == "movement"
        ));
    }
}
