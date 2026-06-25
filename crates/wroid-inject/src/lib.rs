//! Persistent Linux input injection for the Wroid gaming runtime.
//!
//! The hot path keeps one Linux virtual input device open and translates
//! validated touch frames into multitouch protocol Type-B event batches.
//! No subprocess is spawned per event or per frame.

mod config;
mod error;
mod event;
mod injector;
#[allow(private_interfaces)]
mod live_keyboard;
mod live_mouse_aim;
mod sink;
mod state;
mod transition;
mod translate;
mod waydroid_bridge;
mod waydroid_session;

pub use config::{DeviceConfig, DeviceConfigError, DEFAULT_SLOT_COUNT, MAX_SLOT_COUNT};
pub use error::UinputFrameError;
pub use event::{EventSink, LinuxInputEvent};
pub use injector::UinputTouchInjector;
pub use live_keyboard::{
    cleanup_live_keyboard_bridge, default_joystick_center, default_joystick_radius,
    parse_live_keyboard_command, print_live_keyboard_usage, run_live_keyboard_cli,
    run_live_keyboard_session, KeyTapBinding, LiveKeyboardCommand, LiveKeyboardOptions,
    DEFAULT_HOLD_LOG_INTERVAL, DEFAULT_LIVE_HEIGHT, DEFAULT_LIVE_WIDTH, DEFAULT_READY_DELAY,
    DEFAULT_REAFFIRM_INTERVAL,
};
pub use live_mouse_aim::{
    cleanup_live_mouse_aim_bridge, default_mouse_aim_origin, parse_live_mouse_aim_command,
    print_live_mouse_aim_usage, run_live_mouse_aim_cli, run_live_mouse_aim_session,
    LiveMouseAimCommand, LiveMouseAimOptions, MouseAimAction, MouseAimBinding, MouseAimController,
    DEFAULT_MOUSE_AIM_HEIGHT, DEFAULT_MOUSE_AIM_READY_DELAY, DEFAULT_MOUSE_AIM_WIDTH,
};
pub use sink::EvdevEventSink;
pub use waydroid_bridge::{
    remove_bridge, remove_default_bridge, render_bridge_config, CgroupMode, InputDeviceNode,
    InstalledWaydroidBridge, WaydroidBridgePaths, DEFAULT_WAYDROID_BRIDGE_CONFIG,
    DEFAULT_WAYDROID_CONFIG,
};
pub use waydroid_session::{
    ensure_container_stopped, ensure_root, spawn_android_getevent_trace, stop_child,
    wait_for_android_boot_completed, wait_for_android_input_device, DesktopUser,
    DesktopWaydroidSession, WROID_TOUCHSCREEN_NAME,
};

#[cfg(test)]
mod tests;
