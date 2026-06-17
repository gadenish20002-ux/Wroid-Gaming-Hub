use wroid_runtime::{TouchEvent, TouchFrame, TouchPhase};

use super::{config, event, RecordingSink};
use crate::{DeviceConfig, DeviceConfigError, UinputFrameError, UinputTouchInjector};

#[test]
fn rejects_contact_when_all_slots_are_occupied() {
    let mut injector = UinputTouchInjector::with_sink(config(1), RecordingSink::default());
    injector
        .inject_frame(&TouchFrame::single(event(1, TouchPhase::Down, 10, 10)))
        .unwrap();

    let error = injector
        .inject_frame(&TouchFrame::single(event(2, TouchPhase::Down, 20, 20)))
        .unwrap_err();

    assert!(matches!(
        error,
        UinputFrameError::NoFreeSlot { contact_id: 2 }
    ));
    assert_eq!(injector.sink().frames.len(), 1);
    assert_eq!(injector.active_contact_count(), 1);
}

#[test]
fn sink_failure_does_not_commit_slot_state() {
    let sink = RecordingSink {
        fail_next: true,
        ..RecordingSink::default()
    };
    let mut injector = UinputTouchInjector::with_sink(config(10), sink);

    let error = injector
        .inject_frame(&TouchFrame::single(event(3, TouchPhase::Down, 100, 100)))
        .unwrap_err();

    assert!(matches!(error, UinputFrameError::Emit(_)));
    assert_eq!(injector.active_contact_count(), 0);

    injector
        .inject_frame(&TouchFrame::single(event(3, TouchPhase::Down, 100, 100)))
        .unwrap();
    assert_eq!(injector.active_contact_count(), 1);
}

#[test]
fn rejects_coordinates_outside_surface_before_emitting() {
    let mut injector = UinputTouchInjector::with_sink(config(10), RecordingSink::default());

    let error = injector
        .inject_frame(&TouchFrame::single(event(9, TouchPhase::Down, 1920, 10)))
        .unwrap_err();

    assert!(matches!(
        error,
        UinputFrameError::CoordinateOutOfRange {
            contact_id: 9,
            x: 1920,
            y: 10,
            ..
        }
    ));
    assert!(injector.sink().frames.is_empty());
}

#[test]
fn rejects_empty_frame_before_emitting() {
    let mut injector = UinputTouchInjector::with_sink(config(10), RecordingSink::default());

    let error = injector
        .inject_frame(&TouchFrame::new(Vec::<TouchEvent>::new()))
        .unwrap_err();

    assert!(matches!(error, UinputFrameError::EmptyFrame));
    assert!(injector.sink().frames.is_empty());
}

#[test]
fn validates_device_configuration() {
    assert!(matches!(
        DeviceConfig::with_slots(1920, 1080, 0),
        Err(DeviceConfigError::InvalidSlotCount { .. })
    ));
    assert!(matches!(
        DeviceConfig::new(0, 1080),
        Err(DeviceConfigError::InvalidWidth { width: 0 })
    ));
    assert!(matches!(
        DeviceConfig::new(1920, 0),
        Err(DeviceConfigError::InvalidHeight { height: 0 })
    ));
    assert!(matches!(
        DeviceConfig::with_name_and_slots("x".repeat(79), 1920, 1080, 10),
        Err(DeviceConfigError::NameTooLong { .. })
    ));
}
