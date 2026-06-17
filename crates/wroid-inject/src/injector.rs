use std::io;

use wroid_runtime::{TouchFrame, TouchInjectionError, TouchInjector};

use crate::config::MAX_EVENTS_PER_FRAME;
use crate::state::SlotState;
use crate::{DeviceConfig, EvdevEventSink, EventSink, LinuxInputEvent};

pub struct UinputTouchInjector<S = EvdevEventSink> {
    pub(crate) config: DeviceConfig,
    pub(crate) sink: S,
    pub(crate) state: SlotState,
    pub(crate) scratch: Vec<LinuxInputEvent>,
}

impl UinputTouchInjector<EvdevEventSink> {
    pub fn open(config: DeviceConfig) -> io::Result<Self> {
        let sink = EvdevEventSink::create(&config)?;
        Ok(Self::with_sink(config, sink))
    }
}

impl<S: EventSink> UinputTouchInjector<S> {
    pub fn with_sink(config: DeviceConfig, sink: S) -> Self {
        let state = SlotState::new(config.slot_count);
        Self {
            config,
            sink,
            state,
            scratch: Vec::with_capacity(MAX_EVENTS_PER_FRAME),
        }
    }

    pub const fn active_contact_count(&self) -> u16 {
        self.state.active_count
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub fn into_sink(self) -> S {
        self.sink
    }
}

impl<S: EventSink> TouchInjector for UinputTouchInjector<S> {
    fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError> {
        self.inject_frame(frame).map_err(|error| {
            TouchInjectionError::with_source("persistent uinput frame submission failed", error)
        })
    }
}
