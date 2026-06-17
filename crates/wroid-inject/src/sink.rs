use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, InputEvent, KeyCode, PropType,
    UinputAbsSetup,
};

use crate::config::MAX_EVENTS_PER_FRAME;
use crate::{DeviceConfig, EventSink, LinuxInputEvent};

#[derive(Debug)]
pub struct EvdevEventSink {
    device: VirtualDevice,
    raw_events: Vec<InputEvent>,
}

impl EvdevEventSink {
    pub fn create(config: &DeviceConfig) -> io::Result<Self> {
        let mut keys = AttributeSet::<KeyCode>::new();
        keys.insert(KeyCode::BTN_TOUCH);

        let mut properties = AttributeSet::<PropType>::new();
        properties.insert(PropType::DIRECT);

        let x_info = AbsInfo::new(0, 0, (config.width - 1) as i32, 0, 0, 0);
        let y_info = AbsInfo::new(0, 0, (config.height - 1) as i32, 0, 0, 0);
        let slot_info = AbsInfo::new(0, 0, i32::from(config.slot_count - 1), 0, 0, 0);
        let tracking_info = AbsInfo::new(0, 0, i32::from(u16::MAX), 0, 0, 0);

        let device = VirtualDevice::builder()?
            .name(config.name.as_str())
            .with_keys(&keys)?
            .with_properties(&properties)?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, x_info))?
            .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, y_info))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_MT_SLOT,
                slot_info,
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                tracking_info,
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_MT_POSITION_X,
                x_info,
            ))?
            .with_absolute_axis(&UinputAbsSetup::new(
                AbsoluteAxisCode::ABS_MT_POSITION_Y,
                y_info,
            ))?
            .build()?;

        Ok(Self {
            device,
            raw_events: Vec::with_capacity(MAX_EVENTS_PER_FRAME),
        })
    }
}

impl EventSink for EvdevEventSink {
    fn emit(&mut self, events: &[LinuxInputEvent]) -> io::Result<()> {
        self.raw_events.clear();
        self.raw_events.extend(
            events
                .iter()
                .map(|event| InputEvent::new(event.event_type, event.code, event.value)),
        );
        self.device.emit(&self.raw_events)
    }
}
