use std::env;
use std::io;
use std::path::{Path, PathBuf};

use wroid_core::Resolution;
use wroid_inject::{
    serve_runtime_attachment, stop_existing_waydroid_session, BridgeHelperCommand,
    BridgeHelperFactory, BridgeHelperSession, DesktopUser, DesktopWaydroidSession, DeviceConfig,
    ProductionBridgeHelperFactory, RuntimeAttachmentReport, RuntimeChannelServer,
    UinputTouchInjector,
};
use wroid_runtime::TouchEngine;

use crate::platform::{PlatformLaunch, RuntimePlatformBackend};

trait PlatformDriver: Send {
    fn initialize(&mut self, resolution: Resolution) -> io::Result<()>;
    fn change_resolution(&mut self, resolution: Resolution) -> io::Result<()>;
    fn verify_health(&mut self) -> io::Result<()>;
    fn show_ui(&mut self) -> io::Result<()>;
    fn launch_package(&mut self, package: &str) -> io::Result<()>;
    fn serve(
        &mut self,
        channel: RuntimeChannelServer,
        resolution: Resolution,
    ) -> io::Result<RuntimeAttachmentReport>;
    fn shutdown(&mut self) -> io::Result<()>;
}

trait PlatformResources {
    fn cancel_contacts(&mut self) -> io::Result<()>;
    fn stop_waydroid(&mut self) -> io::Result<()>;
    fn finish_helper(&mut self, waydroid_stopped: bool) -> io::Result<()>;
    fn drop_uinput(&mut self);
}

pub(crate) struct ProductionRuntimePlatform {
    driver: Box<dyn PlatformDriver>,
    ready: bool,
    resolution: Option<Resolution>,
}

impl ProductionRuntimePlatform {
    pub(crate) fn new(expected_uid: u32) -> Self {
        Self {
            driver: Box::new(LinuxPlatformDriver::new(expected_uid)),
            ready: false,
            resolution: None,
        }
    }

    #[cfg(test)]
    fn with_driver(driver: Box<dyn PlatformDriver>) -> Self {
        Self {
            driver,
            ready: false,
            resolution: None,
        }
    }
}

impl RuntimePlatformBackend for ProductionRuntimePlatform {
    fn prepare(&mut self, launch: &PlatformLaunch) -> io::Result<()> {
        if !self.ready {
            if let Err(initialization_error) = self.driver.initialize(launch.resolution) {
                let rollback_result = self.driver.shutdown();
                self.ready = false;
                self.resolution = None;
                return Err(combine_primary_and_cleanup(
                    "platform initialization",
                    initialization_error,
                    "rollback",
                    rollback_result,
                ));
            }
            self.ready = true;
            self.resolution = Some(launch.resolution);
        } else if self.resolution == Some(launch.resolution) {
            self.driver.verify_health()?;
        } else {
            self.driver.change_resolution(launch.resolution)?;
            self.resolution = Some(launch.resolution);
        }

        if launch.show_ui {
            self.driver.show_ui()?;
        }
        if launch.launch_package {
            self.driver.launch_package(&launch.package_name)?;
        }
        Ok(())
    }

    fn serve(
        &mut self,
        channel: RuntimeChannelServer,
        resolution: Resolution,
    ) -> io::Result<RuntimeAttachmentReport> {
        if !self.ready || self.resolution != Some(resolution) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime attachment does not match the prepared platform resolution",
            ));
        }
        self.driver.serve(channel, resolution)
    }

    fn shutdown(&mut self) -> io::Result<()> {
        let result = self.driver.shutdown();
        self.ready = false;
        self.resolution = None;
        result
    }
}

struct LinuxPlatformDriver {
    expected_uid: u32,
    event_node: Option<PathBuf>,
    waydroid: Option<DesktopWaydroidSession>,
    helper: Option<Box<dyn BridgeHelperSession>>,
    engine: Option<TouchEngine<UinputTouchInjector>>,
}

impl LinuxPlatformDriver {
    const fn new(expected_uid: u32) -> Self {
        Self {
            expected_uid,
            event_node: None,
            waydroid: None,
            helper: None,
            engine: None,
        }
    }

    fn waydroid_mut(&mut self) -> io::Result<&mut DesktopWaydroidSession> {
        self.waydroid.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "Waydroid session is not initialized",
            )
        })
    }

    fn helper_mut(&mut self) -> io::Result<&mut Box<dyn BridgeHelperSession>> {
        self.helper.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "privileged bridge helper is not initialized",
            )
        })
    }

    fn configure_resolution(&mut self, resolution: Resolution) -> io::Result<()> {
        let waydroid = self.waydroid_mut()?;
        waydroid.wait_until_android_ready()?;
        if waydroid.configure_resolution(resolution.width, resolution.height)? {
            waydroid.restart()?;
            waydroid.wait_until_android_ready()?;
        }
        waydroid.confirm_resolution(resolution.width, resolution.height)
    }
}

impl PlatformDriver for LinuxPlatformDriver {
    fn initialize(&mut self, resolution: Resolution) -> io::Result<()> {
        if self.engine.is_some() || self.helper.is_some() || self.waydroid.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "persistent runtime platform is already initialized",
            ));
        }

        let config = DeviceConfig::with_slots(65_536, 65_536, 10)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let injector = UinputTouchInjector::open(config)?;
        self.engine = Some(TouchEngine::new(injector));
        let event_nodes = self
            .engine
            .as_mut()
            .expect("uinput engine was stored above")
            .injector_mut()
            .sink_mut()
            .event_nodes()?;
        let event_node = select_direct_event_node(event_nodes)?;
        self.event_node = Some(event_node.clone());

        let helper_factory = paired_helper_factory(self.expected_uid)?;
        let desktop_user = DesktopUser::from_session_environment()?;
        stop_existing_waydroid_session(&desktop_user)?;
        self.helper = Some(helper_factory.start(&event_node)?);
        self.waydroid = Some(DesktopWaydroidSession::start(desktop_user)?);
        self.configure_resolution(resolution)?;
        self.helper_mut()?.verify_android_input()
    }

    fn change_resolution(&mut self, resolution: Resolution) -> io::Result<()> {
        self.helper_mut()?.check_health()?;
        self.configure_resolution(resolution)?;
        self.helper_mut()?.verify_android_input()
    }

    fn verify_health(&mut self) -> io::Result<()> {
        self.helper_mut()?.check_health()
    }

    fn show_ui(&mut self) -> io::Result<()> {
        self.waydroid_mut()?.show_full_ui()
    }

    fn launch_package(&mut self, package: &str) -> io::Result<()> {
        self.waydroid_mut()?.launch_package(package)
    }

    fn serve(
        &mut self,
        channel: RuntimeChannelServer,
        resolution: Resolution,
    ) -> io::Result<RuntimeAttachmentReport> {
        let engine = self.engine.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "uinput touchscreen is not initialized",
            )
        })?;
        let helper = self.helper.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                "privileged bridge helper is not initialized",
            )
        })?;
        serve_runtime_attachment(channel, resolution, engine, || helper.check_health())
    }

    fn shutdown(&mut self) -> io::Result<()> {
        shutdown_platform_resources(self)
    }
}

impl PlatformResources for LinuxPlatformDriver {
    fn cancel_contacts(&mut self) -> io::Result<()> {
        match self.engine.as_mut() {
            Some(engine) => engine
                .cancel_all()
                .map(|_| ())
                .map_err(|error| io::Error::other(error.to_string())),
            None => Ok(()),
        }
    }

    fn stop_waydroid(&mut self) -> io::Result<()> {
        let result = match self.waydroid.as_mut() {
            Some(waydroid) => waydroid.stop(),
            None => Ok(()),
        };
        self.waydroid.take();
        result
    }

    fn finish_helper(&mut self, waydroid_stopped: bool) -> io::Result<()> {
        match self.helper.take() {
            Some(helper) => helper.finish(waydroid_stopped),
            None => Ok(()),
        }
    }

    fn drop_uinput(&mut self) {
        self.event_node.take();
        self.engine.take();
    }
}

fn shutdown_platform_resources(resources: &mut impl PlatformResources) -> io::Result<()> {
    let mut failures = Vec::new();

    if let Err(error) = resources.cancel_contacts() {
        failures.push(("contact cancellation", error));
    }

    let stop_succeeded = match resources.stop_waydroid() {
        Ok(()) => true,
        Err(error) => {
            failures.push(("Waydroid stop", error));
            false
        }
    };

    if let Err(error) = resources.finish_helper(stop_succeeded) {
        failures.push(("helper finish", error));
    }

    resources.drop_uinput();
    combined_failures(failures)
}

fn paired_helper_factory(expected_uid: u32) -> io::Result<ProductionBridgeHelperFactory> {
    let executable = env::current_exe()?;
    let release_directory = executable.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "wroidd executable has no release directory",
        )
    })?;
    let command = BridgeHelperCommand::production_release(
        &release_directory.join("wroid-helper"),
        expected_uid,
    )?;
    Ok(ProductionBridgeHelperFactory::new(command))
}

fn select_direct_event_node(event_nodes: Vec<PathBuf>) -> io::Result<PathBuf> {
    event_nodes
        .into_iter()
        .find(|path| {
            path.parent() == Some(Path::new("/dev/input"))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.strip_prefix("event"))
                    .is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                    })
        })
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "uinput event node not found"))
}

fn combine_primary_and_cleanup(
    primary_context: &str,
    primary: io::Error,
    cleanup_context: &str,
    cleanup: io::Result<()>,
) -> io::Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup_error) => io::Error::new(
            primary.kind(),
            format!(
                "{primary_context} failed: {primary}; {cleanup_context} also failed: {cleanup_error}"
            ),
        ),
    }
}

fn combined_failures(failures: Vec<(&'static str, io::Error)>) -> io::Result<()> {
    if failures.is_empty() {
        return Ok(());
    }
    let kind = failures[0].1.kind();
    let detail = failures
        .into_iter()
        .map(|(context, error)| format!("{context} failed: {error}"))
        .collect::<Vec<_>>()
        .join("; ");
    Err(io::Error::new(kind, detail))
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use wroid_core::Resolution;
    use wroid_inject::{runtime_socket_pair, RuntimeAttachmentReport, RuntimeChannelServer};

    use crate::platform::{PlatformLaunch, RuntimePlatformBackend};

    use super::{
        select_direct_event_node, shutdown_platform_resources, PlatformDriver, PlatformResources,
        ProductionRuntimePlatform,
    };

    type Calls = Arc<Mutex<Vec<String>>>;

    struct RecordingPlatformDriver {
        calls: Calls,
        fail_initializations: usize,
        fail_health_checks: usize,
        fail_shutdowns: usize,
        fail_cleanup_steps: bool,
        resources_open: bool,
    }

    impl PlatformResources for RecordingPlatformDriver {
        fn cancel_contacts(&mut self) -> io::Result<()> {
            if !self.resources_open {
                return Ok(());
            }
            self.calls.lock().unwrap().push("cancel".to_owned());
            if self.fail_cleanup_steps {
                return Err(io::Error::other("cancel failed"));
            }
            Ok(())
        }

        fn stop_waydroid(&mut self) -> io::Result<()> {
            if !self.resources_open {
                return Ok(());
            }
            self.calls.lock().unwrap().push("waydroid:stop".to_owned());
            if self.fail_cleanup_steps {
                return Err(io::Error::other("stop failed"));
            }
            Ok(())
        }

        fn finish_helper(&mut self, _waydroid_stopped: bool) -> io::Result<()> {
            if !self.resources_open {
                return Ok(());
            }
            self.calls.lock().unwrap().push("helper:finish".to_owned());
            if self.fail_cleanup_steps {
                return Err(io::Error::other("finish failed"));
            }
            if self.fail_shutdowns > 0 {
                self.fail_shutdowns -= 1;
                return Err(io::Error::other("rollback failed"));
            }
            Ok(())
        }

        fn drop_uinput(&mut self) {
            if self.resources_open {
                self.calls.lock().unwrap().push("uinput:drop".to_owned());
                self.resources_open = false;
            }
        }
    }

    impl PlatformDriver for RecordingPlatformDriver {
        fn initialize(&mut self, resolution: Resolution) -> io::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .extend(["uinput:create".to_owned(), "helper:start".to_owned()]);
            self.resources_open = true;
            if self.fail_initializations > 0 {
                self.fail_initializations -= 1;
                return Err(io::Error::other("initialization failed"));
            }
            self.calls.lock().unwrap().push(format!(
                "waydroid:start:{}x{}",
                resolution.width, resolution.height
            ));
            Ok(())
        }

        fn change_resolution(&mut self, resolution: Resolution) -> io::Result<()> {
            self.calls.lock().unwrap().push(format!(
                "waydroid:restart:{}x{}",
                resolution.width, resolution.height
            ));
            Ok(())
        }

        fn verify_health(&mut self) -> io::Result<()> {
            self.calls.lock().unwrap().push("helper:health".to_owned());
            if self.fail_health_checks > 0 {
                self.fail_health_checks -= 1;
                return Err(io::Error::other("helper exited"));
            }
            Ok(())
        }

        fn show_ui(&mut self) -> io::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push("waydroid:show-ui".to_owned());
            Ok(())
        }

        fn launch_package(&mut self, package: &str) -> io::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("package:{package}"));
            Ok(())
        }

        fn serve(
            &mut self,
            _channel: RuntimeChannelServer,
            _resolution: Resolution,
        ) -> io::Result<RuntimeAttachmentReport> {
            self.calls.lock().unwrap().push("serve".to_owned());
            Ok(RuntimeAttachmentReport {
                frames_submitted: 0,
                peak_contacts: 0,
                contacts_cancelled: 0,
            })
        }

        fn shutdown(&mut self) -> io::Result<()> {
            shutdown_platform_resources(self)
        }
    }

    fn shared_calls() -> Calls {
        Arc::new(Mutex::new(Vec::new()))
    }

    fn fixture_backend(calls: Calls) -> ProductionRuntimePlatform {
        backend_with_failures(calls, 0, 0, 0)
    }

    fn backend_with_failures(
        calls: Calls,
        fail_initializations: usize,
        fail_health_checks: usize,
        fail_shutdowns: usize,
    ) -> ProductionRuntimePlatform {
        ProductionRuntimePlatform::with_driver(Box::new(RecordingPlatformDriver {
            calls,
            fail_initializations,
            fail_health_checks,
            fail_shutdowns,
            fail_cleanup_steps: false,
            resources_open: false,
        }))
    }

    fn platform_launch(package: &str, width: u32, height: u32) -> PlatformLaunch {
        PlatformLaunch {
            package_name: package.to_owned(),
            resolution: Resolution { width, height },
            show_ui: true,
            launch_package: true,
        }
    }

    fn attachment_server() -> RuntimeChannelServer {
        let (_client, server) = runtime_socket_pair().unwrap();
        RuntimeChannelServer::from_owned_fd(server).unwrap()
    }

    fn count(calls: &Calls, expected: &str) -> usize {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| call.as_str() == expected)
            .count()
    }

    fn packages(calls: &Calls) -> Vec<String> {
        calls
            .lock()
            .unwrap()
            .iter()
            .filter_map(|call| call.strip_prefix("package:").map(str::to_owned))
            .collect()
    }

    #[test]
    fn event_node_selection_accepts_only_direct_numeric_event_paths() {
        let selected = select_direct_event_node(vec![
            PathBuf::from("/dev/input/mouse0"),
            PathBuf::from("/tmp/event7"),
            PathBuf::from("/dev/input/event-name"),
            PathBuf::from("/dev/input/event42"),
        ])
        .unwrap();

        assert_eq!(selected, Path::new("/dev/input/event42"));
    }

    #[test]
    fn same_resolution_reuses_touchscreen_helper_and_waydroid() {
        let calls = shared_calls();
        let mut backend = fixture_backend(calls.clone());

        backend
            .prepare(&platform_launch("com.example.one", 1920, 1080))
            .unwrap();
        backend
            .prepare(&platform_launch("com.example.two", 1920, 1080))
            .unwrap();

        assert_eq!(count(&calls, "uinput:create"), 1);
        assert_eq!(count(&calls, "helper:start"), 1);
        assert_eq!(count(&calls, "waydroid:start:1920x1080"), 1);
        assert_eq!(count(&calls, "helper:health"), 1);
        assert_eq!(packages(&calls), ["com.example.one", "com.example.two"]);
    }

    #[test]
    fn resolution_change_restarts_only_waydroid() {
        let calls = shared_calls();
        let mut backend = fixture_backend(calls.clone());

        backend
            .prepare(&platform_launch("com.example.one", 1920, 1080))
            .unwrap();
        backend
            .prepare(&platform_launch("com.example.two", 1280, 720))
            .unwrap();

        assert_eq!(count(&calls, "uinput:create"), 1);
        assert_eq!(count(&calls, "helper:start"), 1);
        assert_eq!(count(&calls, "waydroid:restart:1280x720"), 1);
    }

    #[test]
    fn helper_health_failure_prevents_a_second_launch() {
        let calls = shared_calls();
        let mut backend = backend_with_failures(calls.clone(), 0, 1, 0);
        backend
            .prepare(&platform_launch("com.example.one", 1920, 1080))
            .unwrap();

        let error = backend
            .prepare(&platform_launch("com.example.two", 1920, 1080))
            .unwrap_err();

        assert_eq!(error.to_string(), "helper exited");
        assert_eq!(packages(&calls), ["com.example.one"]);
    }

    #[test]
    fn launch_flags_suppress_optional_waydroid_actions() {
        let calls = shared_calls();
        let mut backend = fixture_backend(calls.clone());
        let mut launch = platform_launch("com.example.hidden", 1920, 1080);
        launch.show_ui = false;
        launch.launch_package = false;

        backend.prepare(&launch).unwrap();

        assert_eq!(count(&calls, "waydroid:show-ui"), 0);
        assert!(packages(&calls).is_empty());
    }

    #[test]
    fn prepared_backend_delegates_runtime_attachment_to_driver() {
        let calls = shared_calls();
        let mut backend = fixture_backend(calls.clone());
        backend
            .prepare(&platform_launch("com.example.one", 1920, 1080))
            .unwrap();

        let report = backend
            .serve(
                attachment_server(),
                Resolution {
                    width: 1920,
                    height: 1080,
                },
            )
            .unwrap();

        assert_eq!(report.frames_submitted, 0);
        assert_eq!(count(&calls, "serve"), 1);
    }

    #[test]
    fn shutdown_orders_all_cleanup_before_uinput_drop() {
        let calls = shared_calls();
        let mut backend = fixture_backend(calls.clone());
        backend
            .prepare(&platform_launch("com.example.one", 1920, 1080))
            .unwrap();

        backend.shutdown().unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(
            &calls[calls.len() - 4..],
            ["cancel", "waydroid:stop", "helper:finish", "uinput:drop"]
        );
    }

    #[test]
    fn shutdown_continues_after_failures_and_combines_every_error() {
        let calls = shared_calls();
        let mut driver = RecordingPlatformDriver {
            calls: calls.clone(),
            fail_initializations: 0,
            fail_health_checks: 0,
            fail_shutdowns: 0,
            fail_cleanup_steps: true,
            resources_open: true,
        };

        let error = shutdown_platform_resources(&mut driver).unwrap_err();

        assert_eq!(
            *calls.lock().unwrap(),
            ["cancel", "waydroid:stop", "helper:finish", "uinput:drop"]
        );
        assert!(error.to_string().contains("cancel failed"));
        assert!(error.to_string().contains("stop failed"));
        assert!(error.to_string().contains("finish failed"));
    }

    #[test]
    fn failed_first_initialization_rolls_back_and_retries_cleanly() {
        let calls = shared_calls();
        let mut backend = backend_with_failures(calls.clone(), 1, 0, 0);

        assert!(backend
            .prepare(&platform_launch("com.example.fail", 1920, 1080))
            .is_err());
        backend
            .prepare(&platform_launch("com.example.retry", 1920, 1080))
            .unwrap();

        assert_eq!(count(&calls, "uinput:create"), 2);
        assert_eq!(count(&calls, "helper:start"), 2);
        assert_eq!(count(&calls, "uinput:drop"), 1);
        assert_eq!(packages(&calls), ["com.example.retry"]);
    }

    #[test]
    fn initialization_and_rollback_failures_are_both_reported() {
        let calls = shared_calls();
        let mut backend = backend_with_failures(calls, 1, 0, 1);

        let error = backend
            .prepare(&platform_launch("com.example.fail", 1920, 1080))
            .unwrap_err();

        assert!(error.to_string().contains("initialization failed"));
        assert!(error.to_string().contains("rollback failed"));
    }
}
