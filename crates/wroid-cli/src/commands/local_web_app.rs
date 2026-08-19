use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{bail, Context, Result};

const SERVER_JOIN_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WebUiMode {
    Native,
    Browser,
    Headless,
}

impl WebUiMode {
    pub(crate) fn from_flags(browser: bool, no_open: bool) -> Self {
        match (browser, no_open) {
            (true, false) => Self::Browser,
            (false, true) => Self::Headless,
            (false, false) => Self::Native,
            (true, true) => unreachable!("clap rejects conflicting UI modes"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LocalOrigin(String);

impl LocalOrigin {
    pub(crate) fn new(address: SocketAddr) -> Result<Self> {
        match address {
            SocketAddr::V4(address) if *address.ip() == Ipv4Addr::LOCALHOST => {
                Ok(Self(format!("http://{address}")))
            }
            _ => bail!("local web applications must bind to IPv4 localhost"),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn allows_uri(&self, uri: &str) -> bool {
        let Some(suffix) = uri.strip_prefix(self.as_str()) else {
            return false;
        };
        suffix.is_empty()
            || suffix.starts_with('/')
            || suffix.starts_with('?')
            || suffix.starts_with('#')
    }
}

pub(crate) struct LocalWebApp {
    origin: LocalOrigin,
    token: String,
    shutdown: Arc<AtomicBool>,
    completion: Receiver<Result<()>>,
    thread: Option<JoinHandle<()>>,
}

impl LocalWebApp {
    pub(crate) fn spawn<F>(
        address: SocketAddr,
        token: String,
        shutdown: Arc<AtomicBool>,
        server: F,
    ) -> Result<Self>
    where
        F: FnOnce() -> Result<()> + Send + 'static,
    {
        let origin = LocalOrigin::new(address)?;
        let (completion_tx, completion) = mpsc::sync_channel(1);
        let worker_shutdown = Arc::clone(&shutdown);
        let thread = thread::Builder::new()
            .name("wroid-local-web-app".to_owned())
            .spawn(move || {
                let result = server();
                worker_shutdown.store(true, Ordering::Release);
                let _ = completion_tx.send(result);
            })
            .context("failed to start the local web application server")?;

        Ok(Self {
            origin,
            token,
            shutdown,
            completion,
            thread: Some(thread),
        })
    }

    pub(crate) fn authenticated_url(&self) -> String {
        format!("{}/?token={}", self.origin.as_str(), self.token)
    }

    pub(crate) fn origin(&self) -> &LocalOrigin {
        &self.origin
    }

    pub(crate) fn shutdown_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.shutdown)
    }

    pub(crate) fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::Acquire)
    }

    pub(crate) fn wait(mut self) -> Result<()> {
        let server_result = self
            .completion
            .recv()
            .map_err(|error| self.completion_error(error))?;
        self.join_thread()?;
        server_result
    }

    pub(crate) fn shutdown_and_join(mut self) -> Result<()> {
        self.shutdown.store(true, Ordering::Release);
        let server_result = match self.completion.recv_timeout(SERVER_JOIN_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                bail!("local web application server did not stop within 3 seconds")
            }
            Err(RecvTimeoutError::Disconnected) => {
                self.join_thread()?;
                bail!("local web application server exited without reporting a result")
            }
        };
        self.join_thread()?;
        server_result
    }

    fn completion_error(&mut self, _error: RecvError) -> anyhow::Error {
        match self.join_thread() {
            Ok(()) => {
                anyhow::anyhow!("local web application server exited without reporting a result")
            }
            Err(error) => error,
        }
    }

    fn join_thread(&mut self) -> Result<()> {
        let Some(thread) = self.thread.take() else {
            return Ok(());
        };
        thread
            .join()
            .map_err(|_| anyhow::anyhow!("local web application server panicked"))
    }
}

impl Drop for LocalWebApp {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use super::{LocalOrigin, LocalWebApp};

    #[test]
    fn exact_origin_rejects_another_host_scheme_or_port() {
        let origin = LocalOrigin::new("127.0.0.1:37613".parse().unwrap()).unwrap();

        assert!(origin.allows_uri("http://127.0.0.1:37613/"));
        assert!(origin.allows_uri("http://127.0.0.1:37613/api/state?token=secret"));
        assert!(!origin.allows_uri("http://127.0.0.1:37614/"));
        assert!(!origin.allows_uri("http://localhost:37613/"));
        assert!(!origin.allows_uri("https://127.0.0.1:37613/"));
        assert!(!origin.allows_uri("file:///tmp/profile.json"));
        assert!(!origin.allows_uri("data:text/html,blocked"));
        assert!(!origin.allows_uri("http://127.0.0.1:37613.evil.invalid/"));
    }

    #[test]
    fn authenticated_url_uses_the_bound_address_and_private_token() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let app = LocalWebApp::spawn(
            "127.0.0.1:37613".parse().unwrap(),
            "private-token".to_owned(),
            Arc::clone(&shutdown),
            || Ok(()),
        )
        .unwrap();

        assert_eq!(app.origin().as_str(), "http://127.0.0.1:37613");
        assert_eq!(
            app.authenticated_url(),
            "http://127.0.0.1:37613/?token=private-token"
        );
        app.wait().unwrap();
    }

    #[test]
    fn shutdown_stops_and_joins_the_server_thread() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let app = LocalWebApp::spawn(
            "127.0.0.1:37613".parse().unwrap(),
            "token".to_owned(),
            Arc::clone(&shutdown),
            move || {
                while !worker_shutdown.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                Ok(())
            },
        )
        .unwrap();

        app.shutdown_and_join().unwrap();
        assert!(shutdown.load(Ordering::Acquire));
    }

    #[test]
    fn completed_server_notifies_the_native_shell() {
        let shutdown = Arc::new(AtomicBool::new(false));
        let app = LocalWebApp::spawn(
            "127.0.0.1:37613".parse().unwrap(),
            "token".to_owned(),
            Arc::clone(&shutdown),
            || Ok(()),
        )
        .unwrap();

        for _ in 0..100 {
            if app.is_shutdown() {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        assert!(app.is_shutdown());
        app.wait().unwrap();
    }
}
