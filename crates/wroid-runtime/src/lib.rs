//! Runtime primitives for low-latency game input.
//!
//! This crate deliberately does not depend on ADB, Waydroid shell commands, a
//! terminal, or a GUI toolkit. It owns the state machine that turns logical
//! touch contact changes into atomic frames for a persistent injection backend.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;

use thiserror::Error;
use wroid_core::Point;

/// Stable identifier for one logical finger on the Android touch surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContactId(u16);

impl ContactId {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Lifecycle transition for a touch contact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchPhase {
    Down,
    Move,
    Up,
    Cancel,
}

/// One contact transition within a synchronized touch frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchEvent {
    pub contact_id: ContactId,
    pub phase: TouchPhase,
    pub position: Point,
}

impl TouchEvent {
    pub const fn new(contact_id: ContactId, phase: TouchPhase, position: Point) -> Self {
        Self {
            contact_id,
            phase,
            position,
        }
    }
}

/// Contact transitions that must be submitted to Android as one synchronized frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchFrame {
    events: Vec<TouchEvent>,
}

impl TouchFrame {
    pub fn new(events: impl IntoIterator<Item = TouchEvent>) -> Self {
        Self {
            events: events.into_iter().collect(),
        }
    }

    pub fn single(event: TouchEvent) -> Self {
        Self {
            events: vec![event],
        }
    }

    pub fn events(&self) -> &[TouchEvent] {
        &self.events
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Error returned by a persistent input injection backend.
#[derive(Debug, Error)]
#[error("touch injection backend failed: {message}")]
pub struct TouchInjectionError {
    message: String,
    #[source]
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl TouchInjectionError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    pub fn with_source(
        message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// Persistent transport capable of submitting one synchronized multitouch frame.
///
/// Production implementations are expected to keep their device or IPC channel
/// open across calls. Starting a subprocess from this method violates the runtime
/// performance contract and is reserved for an explicitly selected compatibility
/// backend.
pub trait TouchInjector {
    fn inject(&mut self, frame: &TouchFrame) -> Result<(), TouchInjectionError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TouchStateError {
    #[error("touch frame must contain at least one event")]
    EmptyFrame,
    #[error("contact {contact_id} appears more than once in one frame")]
    DuplicateContactInFrame { contact_id: u16 },
    #[error("contact {contact_id} is already active")]
    ContactAlreadyActive { contact_id: u16 },
    #[error("contact {contact_id} is not active")]
    ContactNotActive { contact_id: u16 },
}

/// Last committed multitouch state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TouchState {
    active_contacts: BTreeMap<ContactId, Point>,
}

impl TouchState {
    pub fn active_contact_count(&self) -> usize {
        self.active_contacts.len()
    }

    pub fn is_active(&self, contact_id: ContactId) -> bool {
        self.active_contacts.contains_key(&contact_id)
    }

    pub fn position(&self, contact_id: ContactId) -> Option<Point> {
        self.active_contacts.get(&contact_id).copied()
    }

    pub fn active_contacts(&self) -> impl Iterator<Item = (ContactId, Point)> + '_ {
        self.active_contacts
            .iter()
            .map(|(contact_id, point)| (*contact_id, *point))
    }

    fn next_state(&self, frame: &TouchFrame) -> Result<Self, TouchStateError> {
        if frame.is_empty() {
            return Err(TouchStateError::EmptyFrame);
        }

        let mut seen = BTreeSet::new();
        let mut next = self.clone();

        for event in frame.events() {
            if !seen.insert(event.contact_id) {
                return Err(TouchStateError::DuplicateContactInFrame {
                    contact_id: event.contact_id.get(),
                });
            }

            match event.phase {
                TouchPhase::Down => {
                    if next
                        .active_contacts
                        .insert(event.contact_id, event.position)
                        .is_some()
                    {
                        return Err(TouchStateError::ContactAlreadyActive {
                            contact_id: event.contact_id.get(),
                        });
                    }
                }
                TouchPhase::Move => {
                    let Some(position) = next.active_contacts.get_mut(&event.contact_id) else {
                        return Err(TouchStateError::ContactNotActive {
                            contact_id: event.contact_id.get(),
                        });
                    };
                    *position = event.position;
                }
                TouchPhase::Up | TouchPhase::Cancel => {
                    if next.active_contacts.remove(&event.contact_id).is_none() {
                        return Err(TouchStateError::ContactNotActive {
                            contact_id: event.contact_id.get(),
                        });
                    }
                }
            }
        }

        Ok(next)
    }
}

#[derive(Debug, Error)]
pub enum TouchEngineError {
    #[error(transparent)]
    InvalidState(#[from] TouchStateError),
    #[error(transparent)]
    Injection(#[from] TouchInjectionError),
}

/// Applies validated touch frames to a persistent injector.
///
/// A frame is validated against a cloned state first. The committed state is
/// updated only after the backend confirms successful injection, which prevents
/// runtime state from diverging from Android when a backend fails.
pub struct TouchEngine<I> {
    injector: I,
    state: TouchState,
}

impl<I: TouchInjector> TouchEngine<I> {
    pub fn new(injector: I) -> Self {
        Self {
            injector,
            state: TouchState::default(),
        }
    }

    pub fn state(&self) -> &TouchState {
        &self.state
    }

    pub fn injector(&self) -> &I {
        &self.injector
    }

    pub fn injector_mut(&mut self) -> &mut I {
        &mut self.injector
    }

    pub fn into_injector(self) -> I {
        self.injector
    }

    pub fn submit(&mut self, frame: TouchFrame) -> Result<(), TouchEngineError> {
        let next_state = self.state.next_state(&frame)?;
        self.injector.inject(&frame)?;
        self.state = next_state;
        Ok(())
    }

    pub fn begin_contact(
        &mut self,
        contact_id: ContactId,
        position: Point,
    ) -> Result<(), TouchEngineError> {
        self.submit(TouchFrame::single(TouchEvent::new(
            contact_id,
            TouchPhase::Down,
            position,
        )))
    }

    pub fn move_contact(
        &mut self,
        contact_id: ContactId,
        position: Point,
    ) -> Result<(), TouchEngineError> {
        self.submit(TouchFrame::single(TouchEvent::new(
            contact_id,
            TouchPhase::Move,
            position,
        )))
    }

    pub fn end_contact(&mut self, contact_id: ContactId) -> Result<(), TouchEngineError> {
        let position = self
            .state
            .position(contact_id)
            .ok_or(TouchStateError::ContactNotActive {
                contact_id: contact_id.get(),
            })?;
        self.submit(TouchFrame::single(TouchEvent::new(
            contact_id,
            TouchPhase::Up,
            position,
        )))
    }

    /// Cancels every active contact in one synchronized frame.
    ///
    /// Returns `false` when there was nothing to cancel. This method is intended
    /// for focus loss, shutdown, and crash-recovery paths.
    pub fn cancel_all(&mut self) -> Result<bool, TouchEngineError> {
        if self.state.active_contact_count() == 0 {
            return Ok(false);
        }

        let events: Vec<_> = self
            .state
            .active_contacts()
            .map(|(contact_id, position)| {
                TouchEvent::new(contact_id, TouchPhase::Cancel, position)
            })
            .collect();
        self.submit(TouchFrame::new(events))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn point(x: u32, y: u32) -> Point {
        Point { x, y }
    }

    #[test]
    fn tracks_multiple_contacts_independently() {
        let mut engine = TouchEngine::new(RecordingInjector::default());
        let movement = ContactId::new(1);
        let aim = ContactId::new(2);

        engine.begin_contact(movement, point(100, 600)).unwrap();
        engine.begin_contact(aim, point(900, 400)).unwrap();
        engine.move_contact(movement, point(140, 560)).unwrap();
        engine.end_contact(aim).unwrap();

        assert_eq!(engine.state().active_contact_count(), 1);
        assert_eq!(engine.state().position(movement), Some(point(140, 560)));
        assert!(!engine.state().is_active(aim));
        assert_eq!(engine.injector().frames.len(), 4);
    }

    #[test]
    fn rejects_duplicate_down_before_calling_backend() {
        let mut engine = TouchEngine::new(RecordingInjector::default());
        let contact = ContactId::new(7);
        engine.begin_contact(contact, point(10, 20)).unwrap();

        let error = engine.begin_contact(contact, point(30, 40)).unwrap_err();

        assert!(matches!(
            error,
            TouchEngineError::InvalidState(TouchStateError::ContactAlreadyActive {
                contact_id: 7
            })
        ));
        assert_eq!(engine.injector().frames.len(), 1);
        assert_eq!(engine.state().position(contact), Some(point(10, 20)));
    }

    #[test]
    fn backend_failure_does_not_commit_runtime_state() {
        let mut injector = RecordingInjector::default();
        injector.fail_next = true;
        let mut engine = TouchEngine::new(injector);
        let contact = ContactId::new(4);

        let error = engine.begin_contact(contact, point(200, 300)).unwrap_err();

        assert!(matches!(error, TouchEngineError::Injection(_)));
        assert!(!engine.state().is_active(contact));
        assert!(engine.injector().frames.is_empty());
    }

    #[test]
    fn cancel_all_releases_every_contact_in_one_frame() {
        let mut engine = TouchEngine::new(RecordingInjector::default());
        engine
            .begin_contact(ContactId::new(1), point(100, 100))
            .unwrap();
        engine
            .begin_contact(ContactId::new(2), point(200, 200))
            .unwrap();

        assert!(engine.cancel_all().unwrap());
        assert_eq!(engine.state().active_contact_count(), 0);

        let frame = engine.injector().frames.last().unwrap();
        assert_eq!(frame.events().len(), 2);
        assert!(frame
            .events()
            .iter()
            .all(|event| event.phase == TouchPhase::Cancel));
        assert!(!engine.cancel_all().unwrap());
    }

    #[test]
    fn rejects_multiple_events_for_same_contact_in_one_frame() {
        let mut engine = TouchEngine::new(RecordingInjector::default());
        let contact = ContactId::new(3);
        let frame = TouchFrame::new([
            TouchEvent::new(contact, TouchPhase::Down, point(1, 1)),
            TouchEvent::new(contact, TouchPhase::Move, point(2, 2)),
        ]);

        let error = engine.submit(frame).unwrap_err();

        assert!(matches!(
            error,
            TouchEngineError::InvalidState(TouchStateError::DuplicateContactInFrame {
                contact_id: 3
            })
        ));
        assert!(!engine.state().is_active(contact));
        assert!(engine.injector().frames.is_empty());
    }
}
