use evdev::{AbsoluteAxisCode, KeyCode};
use wroid_runtime::{TouchFrame, TouchPhase};

use super::{config, event, RecordingSink};
use crate::{LinuxInputEvent, UinputTouchInjector};

#[test]
fn first_contact_emits_btn_touch_first_and_type_b_sequence() {
    let mut injector = UinputTouchInjector::with_sink(config(10), RecordingSink::default());
    injector
        .inject_frame(&TouchFrame::single(event(7, TouchPhase::Down, 100, 200)))
        .unwrap();

    assert_eq!(
        injector.sink().frames[0],
        vec![
            LinuxInputEvent::key(KeyCode::BTN_TOUCH, 1),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_MT_TRACKING_ID, 7),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_MT_POSITION_X, 100),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_MT_POSITION_Y, 200),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_X, 100),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_Y, 200),
        ]
    );
    assert_eq!(injector.active_contact_count(), 1);
}

#[test]
fn allocates_independent_slots_for_ten_contacts() {
    let mut injector = UinputTouchInjector::with_sink(config(10), RecordingSink::default());
    let frame =
        TouchFrame::new((0..10).map(|id| event(id, TouchPhase::Down, u32::from(id) * 10, 400)));

    injector.inject_frame(&frame).unwrap();

    assert_eq!(injector.active_contact_count(), 10);
    let emitted = &injector.sink().frames[0];
    for slot in 0..10 {
        assert!(emitted.contains(&LinuxInputEvent::absolute(
            AbsoluteAxisCode::ABS_MT_SLOT,
            slot
        )));
    }
}

#[test]
fn final_release_emits_btn_touch_zero_before_slot_release() {
    let mut injector = UinputTouchInjector::with_sink(config(10), RecordingSink::default());
    injector
        .inject_frame(&TouchFrame::single(event(5, TouchPhase::Down, 50, 60)))
        .unwrap();
    injector
        .inject_frame(&TouchFrame::single(event(5, TouchPhase::Up, 50, 60)))
        .unwrap();

    assert_eq!(
        injector.sink().frames[1],
        vec![
            LinuxInputEvent::key(KeyCode::BTN_TOUCH, 0),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_MT_SLOT, 0),
            LinuxInputEvent::absolute(AbsoluteAxisCode::ABS_MT_TRACKING_ID, -1),
        ]
    );
    assert_eq!(injector.active_contact_count(), 0);
}
