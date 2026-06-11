use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::backend::{CurrentAndroidActivity, InputExecutor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InputCall {
    AdbTap(u32, u32),
    AdbSwipe(u32, u32, u32, u32, u64),
    AdbKeyevent(u32),
    AdbLaunchPackage(String),
    AdbInstallApk(PathBuf),
    AdbWmSize,
    AdbWmDensity,
    LaunchDelay(u128),
    StartKeymapper,
    WaydroidShellTap(u32, u32),
    WaydroidShellSwipe(u32, u32, u32, u32, u64),
    WaydroidShellKeyevent(u32),
    WaydroidAppLaunchPackage(String),
    WaydroidAppLaunchPackageAsUser {
        package: String,
        user: String,
        session_env: wroid_waydroid::WaydroidAppLaunchEnv,
    },
    WaydroidAppInstall(PathBuf),
    WaydroidShellWmSize,
    WaydroidShellWmDensity,
}

#[derive(Debug, Default)]
pub(crate) struct FakeInputExecutor {
    pub(crate) devices: Vec<wroid_adb::AdbDevice>,
    pub(crate) fail_devices: bool,
    pub(crate) device_queries: Cell<usize>,
    pub(crate) adb_packages: Vec<String>,
    pub(crate) waydroid_packages: Vec<String>,
    pub(crate) adb_current_activity: Option<CurrentAndroidActivity>,
    pub(crate) waydroid_current_activity: Option<CurrentAndroidActivity>,
    pub(crate) adb_wm_size_output: String,
    pub(crate) adb_wm_density_output: String,
    pub(crate) waydroid_wm_size_output: String,
    pub(crate) waydroid_wm_density_output: String,
    pub(crate) fail_density: bool,
    pub(crate) fail_launch: bool,
    pub(crate) calls: RefCell<Vec<InputCall>>,
}

impl FakeInputExecutor {
    pub(crate) fn with_devices(devices: Vec<wroid_adb::AdbDevice>) -> Self {
        Self {
            devices,
            ..Self::default()
        }
    }

    pub(crate) fn with_device_state(serial: &str, state: &str) -> Self {
        Self::with_devices(vec![wroid_adb::AdbDevice {
            serial: serial.to_owned(),
            state: state.to_owned(),
        }])
    }

    pub(crate) fn with_device_error() -> Self {
        Self {
            fail_devices: true,
            ..Self::default()
        }
    }

    pub(crate) fn calls(&self) -> Vec<InputCall> {
        self.calls.borrow().clone()
    }

    pub(crate) fn with_waydroid_packages(packages: Vec<&str>) -> Self {
        Self {
            waydroid_packages: packages
                .into_iter()
                .map(std::borrow::ToOwned::to_owned)
                .collect(),
            ..Self::default()
        }
    }

    pub(crate) fn with_waydroid_screen(width: u32, height: u32) -> Self {
        Self {
            waydroid_wm_size_output: format!("Physical size: {width}x{height}\n"),
            ..Self::default()
        }
    }

    pub(crate) fn with_waydroid_screen_and_density(width: u32, height: u32, density: u32) -> Self {
        Self {
            waydroid_wm_size_output: format!("Physical size: {width}x{height}\n"),
            waydroid_wm_density_output: format!("Physical density: {density}\n"),
            ..Self::default()
        }
    }
}

impl InputExecutor for FakeInputExecutor {
    fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>> {
        self.device_queries.set(self.device_queries.get() + 1);
        if self.fail_devices {
            Err(anyhow!("adb devices failed"))
        } else {
            Ok(self.devices.clone())
        }
    }

    fn adb_tap(&self, x: u32, y: u32) -> Result<()> {
        self.calls.borrow_mut().push(InputCall::AdbTap(x, y));
        Ok(())
    }

    fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::AdbSwipe(x1, y1, x2, y2, duration_ms));
        Ok(())
    }

    fn adb_keyevent(&self, code: u32) -> Result<()> {
        self.calls.borrow_mut().push(InputCall::AdbKeyevent(code));
        Ok(())
    }

    fn adb_list_packages(&self) -> Result<Vec<String>> {
        Ok(self.adb_packages.clone())
    }

    fn adb_launch_package(&self, package_name: &str) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::AdbLaunchPackage(package_name.to_owned()));
        if self.fail_launch {
            Err(anyhow!("adb launch failed"))
        } else {
            Ok(())
        }
    }

    fn adb_install_apk(&self, path: &Path) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::AdbInstallApk(path.to_owned()));
        Ok(())
    }

    fn adb_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
        Ok(self.adb_current_activity.clone())
    }

    fn adb_wm_size(&self) -> Result<String> {
        self.calls.borrow_mut().push(InputCall::AdbWmSize);
        Ok(self.adb_wm_size_output.clone())
    }

    fn adb_wm_density(&self) -> Result<String> {
        self.calls.borrow_mut().push(InputCall::AdbWmDensity);
        if self.fail_density {
            Err(anyhow!("adb density failed"))
        } else {
            Ok(self.adb_wm_density_output.clone())
        }
    }

    fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidShellTap(x, y));
        Ok(())
    }

    fn waydroid_shell_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidShellSwipe(x1, y1, x2, y2, duration_ms));
        Ok(())
    }

    fn waydroid_shell_keyevent(&self, code: u32) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidShellKeyevent(code));
        Ok(())
    }

    fn waydroid_app_list_packages(&self) -> Result<Vec<String>> {
        Ok(self.waydroid_packages.clone())
    }

    fn waydroid_app_launch_package(&self, package_name: &str) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidAppLaunchPackage(package_name.to_owned()));
        if self.fail_launch {
            Err(anyhow!("waydroid launch failed"))
        } else {
            Ok(())
        }
    }

    fn waydroid_app_launch_package_as_user(
        &self,
        package_name: &str,
        user: &str,
        session_env: &wroid_waydroid::WaydroidAppLaunchEnv,
    ) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidAppLaunchPackageAsUser {
                package: package_name.to_owned(),
                user: user.to_owned(),
                session_env: session_env.clone(),
            });
        if self.fail_launch {
            Err(anyhow!("waydroid launch as user failed"))
        } else {
            Ok(())
        }
    }

    fn waydroid_app_install(&self, path: &Path) -> Result<()> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidAppInstall(path.to_owned()));
        Ok(())
    }

    fn waydroid_shell_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
        Ok(self.waydroid_current_activity.clone())
    }

    fn waydroid_shell_wm_size(&self) -> Result<String> {
        self.calls.borrow_mut().push(InputCall::WaydroidShellWmSize);
        Ok(self.waydroid_wm_size_output.clone())
    }

    fn waydroid_shell_wm_density(&self) -> Result<String> {
        self.calls
            .borrow_mut()
            .push(InputCall::WaydroidShellWmDensity);
        if self.fail_density {
            Err(anyhow!("waydroid density failed"))
        } else {
            Ok(self.waydroid_wm_density_output.clone())
        }
    }
}
