use std::time::Duration;

use thiserror::Error;
use wroid_core::profile_v2::{
    materialize_axis, ActionV2, BindingV2, InputV2, JoystickMode, ProfileV2,
    ProfileV2ValidationError,
};
use wroid_core::{Point, Resolution};

use crate::{
    ContactId, MouseAim, MouseAimConfigError, MouseAimRegion, MouseAimSensitivity,
    MouseAimSettings, VirtualJoystick, VirtualJoystickConfigError,
};

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
    Tap {
        point: Point,
    },
    Hold {
        point: Point,
    },
    VirtualJoystick {
        joystick: VirtualJoystick,
        mode: JoystickMode,
        reaffirm_interval: Option<Duration>,
    },
    MouseAim {
        aim: MouseAim,
        settings: MouseAimSettings,
    },
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
    #[error("binding {binding} has invalid mouse aim geometry: {source}")]
    InvalidMouseAim {
        binding: String,
        #[source]
        source: MouseAimConfigError,
    },
    #[error("binding {binding} mouse aim sensitivity {sensitivity} cannot be represented safely")]
    InvalidMouseAimSensitivity { binding: String, sensitivity: f64 },
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
        ActionV2::Hold { point } => Ok(RuntimeControlAction::Hold {
            point: point.materialize(resolution),
        }),
        ActionV2::VirtualJoystick {
            center,
            radius,
            dead_zone,
            mode,
            reaffirm_ms,
        } => {
            let contact_id = allocate_contact_id(next_contact_id);
            let joystick = VirtualJoystick::from_profile_v2_geometry(
                contact_id, *center, *radius, *dead_zone, resolution,
            )
            .map_err(|source| RuntimeControlPlanError::InvalidVirtualJoystick {
                binding: binding.name.clone(),
                source,
            })?;
            Ok(RuntimeControlAction::VirtualJoystick {
                joystick,
                mode: mode.clone(),
                reaffirm_interval: reaffirm_ms.map(Duration::from_millis),
            })
        }
        ActionV2::MouseAim {
            region,
            sensitivity,
            toggle_key,
            recenter_threshold,
            recenter_gap_ms,
            ads_multiplier,
            reaffirm_ms,
        } => {
            let contact_id = allocate_contact_id(next_contact_id);
            let alternate_contact_id = allocate_contact_id(next_contact_id);
            let left = materialize_axis(region.x, resolution.width);
            let top = materialize_axis(region.y, resolution.height);
            let right = materialize_axis(region.x + region.w, resolution.width);
            let bottom = materialize_axis(region.y + region.h, resolution.height);
            let region = MouseAimRegion {
                left,
                top,
                right,
                bottom,
            };
            let origin = Point {
                x: left + (right - left) / 2,
                y: top + (bottom - top) / 2,
            };
            let sensitivity = materialize_mouse_sensitivity(*sensitivity).ok_or_else(|| {
                RuntimeControlPlanError::InvalidMouseAimSensitivity {
                    binding: binding.name.clone(),
                    sensitivity: *sensitivity,
                }
            })?;
            let aim = MouseAim::new(contact_id, origin, region, resolution, sensitivity).map_err(
                |source| RuntimeControlPlanError::InvalidMouseAim {
                    binding: binding.name.clone(),
                    source,
                },
            )?;
            let ads_multiplier = ads_multiplier
                .map(|value| {
                    materialize_mouse_sensitivity(value).ok_or_else(|| {
                        RuntimeControlPlanError::InvalidMouseAimSensitivity {
                            binding: binding.name.clone(),
                            sensitivity: value,
                        }
                    })
                })
                .transpose()?;
            let settings = MouseAimSettings {
                alternate_contact_id,
                toggle_key: toggle_key.clone(),
                recenter_threshold_milli: (*recenter_threshold * 1_000.0).round() as u16,
                recenter_gap: Duration::from_millis(*recenter_gap_ms),
                ads_multiplier,
                reaffirm_interval: reaffirm_ms.map(Duration::from_millis),
            };
            Ok(RuntimeControlAction::MouseAim { aim, settings })
        }
        ActionV2::Macro { .. } => Err(RuntimeControlPlanError::UnsupportedAction {
            binding: binding.name.clone(),
            kind: "macro",
        }),
    }
}

fn allocate_contact_id(next_contact_id: &mut u16) -> ContactId {
    let contact_id = ContactId::new(*next_contact_id);
    *next_contact_id = (*next_contact_id).saturating_add(1);
    contact_id
}

fn materialize_mouse_sensitivity(value: f64) -> Option<MouseAimSensitivity> {
    const DENOMINATOR: u32 = 1_000;

    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let scaled = (value * f64::from(DENOMINATOR)).round();
    if scaled < 1.0 || scaled > f64::from(u32::MAX) {
        return None;
    }
    MouseAimSensitivity::new(scaled as u32, DENOMINATOR).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_core::profile_v2::{JoystickMode, NormalizedPoint, NormalizedRect};

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
                    name: "aim".to_owned(),
                    input: InputV2::MouseMove,
                    action: ActionV2::MouseAim {
                        region: NormalizedRect {
                            x: 0.35,
                            y: 0.06,
                            w: 0.60,
                            h: 0.78,
                        },
                        sensitivity: 1.2,
                        toggle_key: Some("tab".to_owned()),
                        recenter_threshold: 0.7,
                        recenter_gap_ms: 0,
                        ads_multiplier: Some(0.6),
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
                BindingV2 {
                    name: "automatic_fire".to_owned(),
                    input: InputV2::MouseButton {
                        button: "side".to_owned(),
                    },
                    action: ActionV2::Hold {
                        point: NormalizedPoint { x: 0.80, y: 0.40 },
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
        assert_eq!(plan.controls.len(), 4);

        let movement = plan.control("movement").unwrap();
        let RuntimeControlAction::VirtualJoystick {
            joystick,
            mode,
            reaffirm_interval,
        } = &movement.action
        else {
            panic!("movement should materialize as a virtual joystick");
        };
        assert_eq!(joystick.contact_id(), ContactId::new(1));
        assert_eq!(joystick.center(), Point { x: 345, y: 842 });
        assert_eq!(joystick.radius(), 97);
        assert_eq!(joystick.dead_zone(), 22);
        assert_eq!(mode, &JoystickMode::Hold);
        assert_eq!(*reaffirm_interval, Some(Duration::from_millis(50)));

        let aim = plan.control("aim").unwrap();
        let RuntimeControlAction::MouseAim { aim, settings } = &aim.action else {
            panic!("aim should materialize as mouse aim");
        };
        assert_eq!(aim.contact_id(), ContactId::new(2));
        assert_eq!(settings.alternate_contact_id, ContactId::new(3));
        assert_eq!(settings.toggle_key.as_deref(), Some("tab"));
        assert_eq!(settings.recenter_threshold_milli, 700);
        assert_eq!(
            settings.ads_multiplier,
            Some(MouseAimSensitivity::new(600, 1000).unwrap())
        );
        assert_eq!(settings.reaffirm_interval, Some(Duration::from_millis(50)));
        assert_eq!(
            aim.region(),
            MouseAimRegion {
                left: 672,
                top: 65,
                right: 1823,
                bottom: 906,
            }
        );
        assert_eq!(aim.origin(), Point { x: 1247, y: 485 });
        assert_eq!(
            aim.sensitivity(),
            MouseAimSensitivity::new(1200, 1000).unwrap()
        );

        let fire = plan.control("fire").unwrap();
        assert_eq!(
            &fire.action,
            &RuntimeControlAction::Tap {
                point: Point { x: 1650, y: 540 }
            }
        );

        let automatic_fire = plan.control("automatic_fire").unwrap();
        assert_eq!(
            &automatic_fire.action,
            &RuntimeControlAction::Hold {
                point: Point { x: 1535, y: 432 }
            }
        );
    }

    #[test]
    fn macro_remains_explicitly_unsupported() {
        let mut profile = profile();
        profile.bindings[0].action = ActionV2::Macro {
            steps: vec![ActionV2::Tap {
                point: NormalizedPoint { x: 0.5, y: 0.5 },
            }],
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
                kind: "macro"
            } if binding == "movement"
        ));
    }
}
