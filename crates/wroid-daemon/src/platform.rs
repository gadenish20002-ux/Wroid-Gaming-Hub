use std::io;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use wroid_core::Resolution;
use wroid_inject::{RuntimeAttachmentReport, RuntimeChannelServer, RuntimeChannelShutdown};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlatformLaunch {
    pub(crate) package_name: String,
    pub(crate) resolution: Resolution,
    pub(crate) show_ui: bool,
    pub(crate) launch_package: bool,
}

pub(crate) trait RuntimePlatformBackend: Send {
    fn prepare(&mut self, launch: &PlatformLaunch) -> io::Result<()>;

    fn serve(
        &mut self,
        channel: RuntimeChannelServer,
        resolution: Resolution,
    ) -> io::Result<RuntimeAttachmentReport>;

    fn shutdown(&mut self) -> io::Result<()>;
}

pub(crate) struct PlatformAttachment {
    completion: Option<Receiver<io::Result<RuntimeAttachmentReport>>>,
    runtime_shutdown: RuntimeChannelShutdown,
}

impl PlatformAttachment {
    pub(crate) fn try_finish(&mut self) -> Option<io::Result<RuntimeAttachmentReport>> {
        let completion = self.completion.as_ref()?;
        match completion.try_recv() {
            Ok(result) => {
                self.completion = None;
                return Some(result);
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.completion = None;
                return Some(Err(attachment_completion_disconnected_error()));
            }
        }
        None
    }

    pub(crate) fn finish(mut self) -> io::Result<RuntimeAttachmentReport> {
        if let Some(result) = self.try_finish() {
            return result;
        }
        let _ = self.runtime_shutdown.shutdown();
        let Some(completion) = self.completion.take() else {
            return Err(attachment_completion_disconnected_error());
        };
        completion
            .recv()
            .map_err(|_| attachment_completion_disconnected_error())?
    }
}

fn attachment_completion_disconnected_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "platform attachment thread ended before reporting completion",
    )
}

pub(crate) struct PersistentPlatform {
    commands: Sender<PlatformCommand>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

type PlatformFactory =
    Arc<dyn Fn() -> io::Result<Box<dyn RuntimePlatformBackend>> + Send + Sync + 'static>;

enum PlatformCommand {
    Attach {
        channel: RuntimeChannelServer,
        completion_shutdown: RuntimeChannelShutdown,
        launch: PlatformLaunch,
        completion: SyncSender<io::Result<RuntimeAttachmentReport>>,
    },
    Shutdown,
}

impl PersistentPlatform {
    pub(crate) fn with_factory(factory: PlatformFactory) -> Self {
        let (commands, incoming) = mpsc::channel();
        let thread = thread::spawn(move || run_platform(factory, incoming));
        Self {
            commands,
            thread: Some(thread),
        }
    }

    pub(crate) fn attach(
        &self,
        channel: RuntimeChannelServer,
        launch: PlatformLaunch,
    ) -> io::Result<PlatformAttachment> {
        let runtime_shutdown = channel.shutdown_handle()?;
        let completion_shutdown = channel.shutdown_handle()?;
        let (completion, result) = mpsc::sync_channel(1);
        self.commands
            .send(PlatformCommand::Attach {
                channel,
                completion_shutdown,
                launch,
                completion,
            })
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "platform thread is not accepting attachments",
                )
            })?;
        Ok(PlatformAttachment {
            completion: Some(result),
            runtime_shutdown,
        })
    }
}

impl Drop for PersistentPlatform {
    fn drop(&mut self) {
        let _ = self.commands.send(PlatformCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_platform(factory: PlatformFactory, commands: Receiver<PlatformCommand>) -> io::Result<()> {
    let mut backend = None;
    while let Ok(command) = commands.recv() {
        match command {
            PlatformCommand::Attach {
                mut channel,
                completion_shutdown,
                launch,
                completion,
            } => {
                if backend.is_none() {
                    match factory() {
                        Ok(created) => backend = Some(created),
                        Err(error) => {
                            let _ = channel.send_startup_error(&error.to_string());
                            let _ = completion_shutdown.shutdown();
                            let _ = completion.send(Err(error));
                            continue;
                        }
                    }
                }

                let active_backend = backend.as_mut().expect("backend was created above");
                if let Err(error) = active_backend.prepare(&launch) {
                    let _ = channel.send_startup_error(&error.to_string());
                    let _ = completion_shutdown.shutdown();
                    let _ = completion.send(Err(error));
                    continue;
                }
                let resolution = launch.resolution;
                let result = active_backend.serve(channel, resolution);
                let _ = completion_shutdown.shutdown();
                let _ = completion.send(result);
            }
            PlatformCommand::Shutdown => break,
        }
    }

    match backend {
        Some(mut backend) => backend.shutdown(),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::mpsc::{self, Receiver, SyncSender};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use wroid_core::Resolution;
    use wroid_inject::{
        runtime_socket_pair, serve_runtime_attachment, RuntimeAttachmentReport,
        RuntimeChannelClient, RuntimeChannelServer,
    };
    use wroid_runtime::{TouchEngine, TouchFrame, TouchInjectionError, TouchInjector};

    use super::{PersistentPlatform, PlatformFactory, PlatformLaunch, RuntimePlatformBackend};

    const FAKE_EVENT_NODE: &str = "/dev/input/event42";

    struct FakeBackend {
        calls: Arc<Mutex<Vec<String>>>,
        prepare_failure: Option<String>,
        serve_started: Option<SyncSender<()>>,
        serve_release: Option<Receiver<()>>,
        event_node: &'static str,
    }

    impl RuntimePlatformBackend for FakeBackend {
        fn prepare(&mut self, launch: &PlatformLaunch) -> io::Result<()> {
            assert_eq!(self.event_node, FAKE_EVENT_NODE);
            self.calls
                .lock()
                .unwrap()
                .push(format!("prepare:{}", launch.package_name));
            if let Some(detail) = self.prepare_failure.take() {
                return Err(io::Error::other(detail));
            }
            Ok(())
        }

        fn serve(
            &mut self,
            _channel: RuntimeChannelServer,
            _resolution: Resolution,
        ) -> io::Result<RuntimeAttachmentReport> {
            self.calls.lock().unwrap().push("serve".to_owned());
            if let Some(started) = self.serve_started.take() {
                started.send(()).unwrap();
            }
            if let Some(release) = self.serve_release.take() {
                release.recv().unwrap();
            }
            Ok(RuntimeAttachmentReport {
                frames_submitted: 0,
                peak_contacts: 0,
                contacts_cancelled: 0,
            })
        }

        fn shutdown(&mut self) -> io::Result<()> {
            self.calls.lock().unwrap().push("shutdown".to_owned());
            Ok(())
        }
    }

    struct RuntimeServingBackend;

    impl RuntimePlatformBackend for RuntimeServingBackend {
        fn prepare(&mut self, _launch: &PlatformLaunch) -> io::Result<()> {
            Ok(())
        }

        fn serve(
            &mut self,
            channel: RuntimeChannelServer,
            resolution: Resolution,
        ) -> io::Result<RuntimeAttachmentReport> {
            let mut engine = TouchEngine::new(NoopInjector);
            serve_runtime_attachment(channel, resolution, &mut engine, || Ok(()))
        }

        fn shutdown(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct NoopInjector;

    impl TouchInjector for NoopInjector {
        fn inject(&mut self, _frame: &TouchFrame) -> Result<(), TouchInjectionError> {
            Ok(())
        }
    }

    fn fake_factory(calls: Arc<Mutex<Vec<String>>>) -> PlatformFactory {
        Arc::new(move || {
            calls.lock().unwrap().push("factory".to_owned());
            Ok(Box::new(FakeBackend {
                calls: calls.clone(),
                prepare_failure: None,
                serve_started: None,
                serve_release: None,
                event_node: FAKE_EVENT_NODE,
            }))
        })
    }

    fn platform_launch(package_name: &str, width: u32, height: u32) -> PlatformLaunch {
        PlatformLaunch {
            package_name: package_name.to_owned(),
            resolution: Resolution { width, height },
            show_ui: true,
            launch_package: true,
        }
    }

    fn runtime_pair() -> (RuntimeChannelClient, RuntimeChannelServer) {
        let (client, server) = runtime_socket_pair().unwrap();
        (
            RuntimeChannelClient::from_owned_fd(client).unwrap(),
            RuntimeChannelServer::from_owned_fd(server).unwrap(),
        )
    }

    fn run_fake_attachment(
        platform: &PersistentPlatform,
        launch: PlatformLaunch,
    ) -> io::Result<RuntimeAttachmentReport> {
        let (_client, server) = runtime_pair();
        platform.attach(server, launch)?.finish()
    }

    #[test]
    fn attachment_finish_after_worker_exit_failure_does_not_hang_on_leaked_runtime_peer() {
        let platform = PersistentPlatform::with_factory(Arc::new(|| {
            Ok(Box::new(RuntimeServingBackend) as Box<dyn RuntimePlatformBackend>)
        }));
        let (mut client, server) = runtime_pair();
        let attachment = platform
            .attach(
                server,
                platform_launch("com.example.leaked-peer", 1600, 900),
            )
            .unwrap();
        client.wait_until_ready().unwrap();
        let (finished, result) = mpsc::channel();

        let join = thread::spawn(move || {
            let _ = finished.send(attachment.finish());
        });
        let completion = result.recv_timeout(Duration::from_secs(1));
        if completion.is_err() {
            drop(client);
            join.join().unwrap();
            drop(platform);
            panic!("platform attachment finish hung after worker exit");
        }

        completion.unwrap().unwrap();
        drop(client);
        join.join().unwrap();
    }

    #[test]
    fn two_attachments_reuse_one_lazy_backend() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let platform = PersistentPlatform::with_factory(fake_factory(calls.clone()));
        assert!(calls.lock().unwrap().is_empty());

        run_fake_attachment(&platform, platform_launch("com.example.one", 1920, 1080)).unwrap();
        run_fake_attachment(&platform, platform_launch("com.example.two", 1920, 1080)).unwrap();

        assert_eq!(
            *calls.lock().unwrap(),
            [
                "factory",
                "prepare:com.example.one",
                "serve",
                "prepare:com.example.two",
                "serve"
            ]
        );
    }

    #[test]
    fn failed_prepare_sends_one_bounded_startup_error_and_allows_retry() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let failure_detail = "preparation failed: ".to_owned() + &"x".repeat(1024);
        let factory_calls = calls.clone();
        let factory_failure = failure_detail.clone();
        let factory: PlatformFactory = Arc::new(move || {
            factory_calls.lock().unwrap().push("factory".to_owned());
            Ok(Box::new(FakeBackend {
                calls: factory_calls.clone(),
                prepare_failure: Some(factory_failure.clone()),
                serve_started: None,
                serve_release: None,
                event_node: FAKE_EVENT_NODE,
            }))
        });
        let platform = PersistentPlatform::with_factory(factory);
        let (mut client, server) = runtime_pair();

        let attachment = platform
            .attach(server, platform_launch("com.example.fail", 1920, 1080))
            .unwrap();
        let startup_error = client.wait_until_ready().unwrap_err();
        assert_eq!(startup_error.kind(), io::ErrorKind::Other);
        assert!(startup_error
            .to_string()
            .starts_with("preparation failed: "));
        assert!(startup_error.to_string().len() <= 120);
        assert_eq!(
            client.wait_until_ready().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(attachment.finish().unwrap_err().to_string(), failure_detail);

        run_fake_attachment(&platform, platform_launch("com.example.retry", 1920, 1080)).unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            [
                "factory",
                "prepare:com.example.fail",
                "prepare:com.example.retry",
                "serve"
            ]
        );
    }

    #[test]
    fn drop_waits_for_current_attachment_then_shuts_down_and_joins() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (serve_started_tx, serve_started_rx) = mpsc::sync_channel(1);
        let (serve_release_tx, serve_release_rx) = mpsc::sync_channel(1);
        let release = Arc::new(Mutex::new(Some(serve_release_rx)));
        let factory_calls = calls.clone();
        let factory_release = release.clone();
        let factory: PlatformFactory = Arc::new(move || {
            factory_calls.lock().unwrap().push("factory".to_owned());
            Ok(Box::new(FakeBackend {
                calls: factory_calls.clone(),
                prepare_failure: None,
                serve_started: Some(serve_started_tx.clone()),
                serve_release: factory_release.lock().unwrap().take(),
                event_node: FAKE_EVENT_NODE,
            }))
        });
        let platform = PersistentPlatform::with_factory(factory);
        let (_client, server) = runtime_pair();
        let attachment = platform
            .attach(server, platform_launch("com.example.one", 1920, 1080))
            .unwrap();
        serve_started_rx.recv().unwrap();

        let (drop_started_tx, drop_started_rx) = mpsc::sync_channel(1);
        let (drop_finished_tx, drop_finished_rx) = mpsc::sync_channel(1);
        let drop_join = thread::spawn(move || {
            drop_started_tx.send(()).unwrap();
            drop(platform);
            drop_finished_tx.send(()).unwrap();
        });
        drop_started_rx.recv().unwrap();
        assert_eq!(
            drop_finished_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );

        serve_release_tx.send(()).unwrap();
        drop_join.join().unwrap();
        drop_finished_rx.recv().unwrap();
        attachment.finish().unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            ["factory", "prepare:com.example.one", "serve", "shutdown"]
        );
    }
}
