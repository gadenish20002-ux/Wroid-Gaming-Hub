//! Per-user runtime daemon foundation.
//!
//! This crate owns daemon-side session bookkeeping without depending on ADB,
//! Waydroid shell commands, terminal input, GUI toolkits, or privileged helper
//! implementation details. The first implementation is intentionally in-memory
//! so the CLI and future UI can migrate to the typed runtime contracts before
//! IPC and helper processes are introduced.

use std::collections::BTreeMap;

use thiserror::Error;
use wroid_core::ProfileValidationError;
use wroid_runtime::{
    PreparedSession, SessionId, SessionLifecycle, SessionRequest, SessionState, StopReason,
    StopReport,
};

/// Session metadata owned by the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSession {
    session_id: SessionId,
    state: SessionState,
    active_package: String,
    launch_package: bool,
}

impl RuntimeSession {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub const fn state(&self) -> SessionState {
        self.state
    }

    pub fn active_package(&self) -> &str {
        &self.active_package
    }

    pub const fn launch_package(&self) -> bool {
        self.launch_package
    }

    fn prepared(&self) -> PreparedSession {
        PreparedSession {
            session_id: self.session_id.clone(),
            state: self.state,
            active_package: self.active_package.clone(),
        }
    }
}

/// In-memory daemon session manager used before the IPC daemon is introduced.
#[derive(Debug, Default)]
pub struct DaemonSessionManager {
    sessions: BTreeMap<SessionId, RuntimeSession>,
}

impl DaemonSessionManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn session(&self, session_id: &SessionId) -> Option<&RuntimeSession> {
        self.sessions.get(session_id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn has_session(&self, session_id: &SessionId) -> bool {
        self.sessions.contains_key(session_id)
    }
}

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error(transparent)]
    InvalidProfile(#[from] ProfileValidationError),
    #[error("session {0} already exists")]
    SessionAlreadyExists(String),
    #[error("session {0} was not found")]
    SessionNotFound(String),
    #[error("session {session_id} cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        session_id: String,
        from: SessionState,
        to: SessionState,
    },
}

impl SessionLifecycle for DaemonSessionManager {
    type Error = DaemonError;

    fn prepare(&mut self, request: SessionRequest) -> Result<PreparedSession, Self::Error> {
        request.profile.validate()?;

        if self.sessions.contains_key(&request.session_id) {
            return Err(DaemonError::SessionAlreadyExists(
                request.session_id.as_str().to_owned(),
            ));
        }

        let session = RuntimeSession {
            session_id: request.session_id.clone(),
            state: SessionState::Preparing,
            active_package: request.profile.package_name.clone(),
            launch_package: request.launch_package,
        };
        let prepared = session.prepared();
        self.sessions.insert(request.session_id, session);
        Ok(prepared)
    }

    fn start(&mut self, session_id: &SessionId) -> Result<(), Self::Error> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound(session_id.as_str().to_owned()))?;

        match session.state {
            SessionState::Preparing => {
                session.state = SessionState::Running;
                Ok(())
            }
            other => Err(DaemonError::InvalidTransition {
                session_id: session_id.as_str().to_owned(),
                from: other,
                to: SessionState::Running,
            }),
        }
    }

    fn stop(
        &mut self,
        session_id: &SessionId,
        reason: StopReason,
    ) -> Result<StopReport, Self::Error> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| DaemonError::SessionNotFound(session_id.as_str().to_owned()))?;

        match session.state {
            SessionState::Preparing | SessionState::Running | SessionState::Failed => {
                session.state = SessionState::Stopping;
                let report = stop_report_for_reason(reason);
                session.state = SessionState::Stopped;
                Ok(report)
            }
            SessionState::Stopped => Ok(StopReport::default()),
            SessionState::Stopping => Err(DaemonError::InvalidTransition {
                session_id: session_id.as_str().to_owned(),
                from: SessionState::Stopping,
                to: SessionState::Stopped,
            }),
        }
    }

    fn state(&self, session_id: &SessionId) -> Result<SessionState, Self::Error> {
        self.sessions
            .get(session_id)
            .map(RuntimeSession::state)
            .ok_or_else(|| DaemonError::SessionNotFound(session_id.as_str().to_owned()))
    }
}

fn stop_report_for_reason(reason: StopReason) -> StopReport {
    StopReport {
        contacts_cancelled: 0,
        leases_released: 0,
        settings_restored: matches!(
            reason,
            StopReason::UserRequested
                | StopReason::FocusLost
                | StopReason::BackendFailed
                | StopReason::ClientDisconnected
                | StopReason::RuntimeShutdown
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wroid_core::{ControlProfile, Resolution};
    use wroid_runtime::{DisplayInfo, SessionId};

    fn valid_request(session_id: &str) -> SessionRequest {
        let profile = ControlProfile {
            name: "Test Game".to_owned(),
            package_name: "com.example.game".to_owned(),
            resolution: Resolution {
                width: 1920,
                height: 1080,
            },
            bindings: Vec::new(),
        };
        let resolution = profile.resolution;

        SessionRequest {
            session_id: SessionId::new(session_id).unwrap(),
            profile,
            display: DisplayInfo::new(resolution),
            launch_package: true,
        }
    }

    #[test]
    fn prepares_valid_session() {
        let mut manager = DaemonSessionManager::new();
        let request = valid_request("session-1");
        let session_id = request.session_id.clone();

        let prepared = manager.prepare(request).unwrap();

        assert_eq!(prepared.session_id, session_id);
        assert_eq!(prepared.state, SessionState::Preparing);
        assert_eq!(prepared.active_package, "com.example.game");
        assert_eq!(manager.session_count(), 1);
        assert!(manager.has_session(&session_id));
    }

    #[test]
    fn rejects_invalid_profile() {
        let mut request = valid_request("invalid-profile");
        request.profile.package_name.clear();
        let mut manager = DaemonSessionManager::new();

        let error = manager.prepare(request).unwrap_err();

        assert!(matches!(error, DaemonError::InvalidProfile(_)));
        assert_eq!(manager.session_count(), 0);
    }

    #[test]
    fn start_requires_prepared_state() {
        let mut manager = DaemonSessionManager::new();
        let request = valid_request("session-2");
        let session_id = request.session_id.clone();
        manager.prepare(request).unwrap();

        manager.start(&session_id).unwrap();
        assert_eq!(manager.state(&session_id).unwrap(), SessionState::Running);

        let error = manager.start(&session_id).unwrap_err();
        assert!(matches!(
            error,
            DaemonError::InvalidTransition {
                from: SessionState::Running,
                to: SessionState::Running,
                ..
            }
        ));
    }

    #[test]
    fn stop_moves_running_session_to_stopped() {
        let mut manager = DaemonSessionManager::new();
        let request = valid_request("session-3");
        let session_id = request.session_id.clone();
        manager.prepare(request).unwrap();
        manager.start(&session_id).unwrap();

        let report = manager
            .stop(&session_id, StopReason::UserRequested)
            .unwrap();

        assert!(report.settings_restored);
        assert_eq!(manager.state(&session_id).unwrap(), SessionState::Stopped);
    }

    #[test]
    fn duplicate_prepare_is_rejected() {
        let mut manager = DaemonSessionManager::new();
        manager.prepare(valid_request("dup")).unwrap();

        let error = manager.prepare(valid_request("dup")).unwrap_err();

        assert!(matches!(error, DaemonError::SessionAlreadyExists(_)));
    }
}
