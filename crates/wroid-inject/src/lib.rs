//! Persistent Linux input injection for the Wroid gaming runtime.
//!
//! The hot path keeps one Linux virtual input device open and translates
//! validated touch frames into multitouch protocol Type-B event batches.
//! No subprocess is spawned per event or per frame.

mod config;
mod error;
mod event;
mod injector;
mod sink;
mod state;
mod transition;
mod translate;
mod waydroid_bridge;

pub use config::{DeviceConfig, DeviceConfigError, DEFAULT_SLOT_COUNT, MAX_SLOT_COUNT};
pub use error::UinputFrameError;
pub use event::{EventSink, LinuxInputEvent};
pub use injector::UinputTouchInjector;
pub use sink::EvdevEventSink;
pub use waydroid_bridge::{
    remove_bridge, remove_default_bridge, render_bridge_config, CgroupMode, InputDeviceNode,
    InstalledWaydroidBridge, WaydroidBridgePaths, DEFAULT_WAYDROID_BRIDGE_CONFIG,
    DEFAULT_WAYDROID_CONFIG,
};

#[cfg(test)]
mod tests;
