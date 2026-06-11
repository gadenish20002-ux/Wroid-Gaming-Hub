use std::fmt;
use std::path::Path;

use anyhow::Result;

use crate::cli::InputBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectedInputBackend {
    Adb,
    WaydroidShell,
}

impl fmt::Display for SelectedInputBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adb => write!(f, "adb"),
            Self::WaydroidShell => write!(f, "waydroid-shell"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CurrentAndroidActivity {
    pub(crate) package_name: String,
    pub(crate) activity_name: String,
    pub(crate) component_name: String,
}

pub(crate) trait InputExecutor {
    fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>>;
    fn adb_tap(&self, x: u32, y: u32) -> Result<()>;
    fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()>;
    fn adb_keyevent(&self, code: u32) -> Result<()>;
    fn adb_list_packages(&self) -> Result<Vec<String>>;
    fn adb_launch_package(&self, package_name: &str) -> Result<()>;
    fn adb_install_apk(&self, path: &Path) -> Result<()>;
    fn adb_current_activity(&self) -> Result<Option<CurrentAndroidActivity>>;
    fn adb_wm_size(&self) -> Result<String>;
    fn adb_wm_density(&self) -> Result<String>;
    fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()>;
    fn waydroid_shell_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()>;
    fn waydroid_shell_keyevent(&self, code: u32) -> Result<()>;
    fn waydroid_app_list_packages(&self) -> Result<Vec<String>>;
    fn waydroid_app_launch_package(&self, package_name: &str) -> Result<()>;
    fn waydroid_app_launch_package_as_user(
        &self,
        package_name: &str,
        user: &str,
        session_env: &wroid_waydroid::WaydroidAppLaunchEnv,
    ) -> Result<()>;
    fn waydroid_app_install(&self, path: &Path) -> Result<()>;
    fn waydroid_shell_current_activity(&self) -> Result<Option<CurrentAndroidActivity>>;
    fn waydroid_shell_wm_size(&self) -> Result<String>;
    fn waydroid_shell_wm_density(&self) -> Result<String>;
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CommandInputExecutor;

impl InputExecutor for CommandInputExecutor {
    fn adb_devices(&self) -> Result<Vec<wroid_adb::AdbDevice>> {
        wroid_adb::devices()
    }

    fn adb_tap(&self, x: u32, y: u32) -> Result<()> {
        wroid_adb::tap(x, y)
    }

    fn adb_swipe(&self, x1: u32, y1: u32, x2: u32, y2: u32, duration_ms: u64) -> Result<()> {
        wroid_adb::swipe(x1, y1, x2, y2, duration_ms)
    }

    fn adb_keyevent(&self, code: u32) -> Result<()> {
        wroid_adb::keyevent(code)
    }

    fn adb_list_packages(&self) -> Result<Vec<String>> {
        wroid_adb::list_packages()
    }

    fn adb_launch_package(&self, package_name: &str) -> Result<()> {
        wroid_adb::launch_package(package_name)
    }

    fn adb_install_apk(&self, path: &Path) -> Result<()> {
        wroid_adb::install_apk(path)
    }

    fn adb_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
        Ok(
            wroid_adb::current_activity()?.map(|activity| CurrentAndroidActivity {
                package_name: activity.package_name,
                activity_name: activity.activity_name,
                component_name: activity.component_name,
            }),
        )
    }

    fn adb_wm_size(&self) -> Result<String> {
        wroid_adb::wm_size()
    }

    fn adb_wm_density(&self) -> Result<String> {
        wroid_adb::wm_density()
    }

    fn waydroid_shell_tap(&self, x: u32, y: u32) -> Result<()> {
        wroid_waydroid::shell_input_tap(x, y)
    }

    fn waydroid_shell_swipe(
        &self,
        x1: u32,
        y1: u32,
        x2: u32,
        y2: u32,
        duration_ms: u64,
    ) -> Result<()> {
        wroid_waydroid::shell_input_swipe(x1, y1, x2, y2, duration_ms)
    }

    fn waydroid_shell_keyevent(&self, code: u32) -> Result<()> {
        wroid_waydroid::shell_input_keyevent(code)
    }

    fn waydroid_app_list_packages(&self) -> Result<Vec<String>> {
        wroid_waydroid::app_list_packages()
    }

    fn waydroid_app_launch_package(&self, package_name: &str) -> Result<()> {
        wroid_waydroid::app_launch_package(package_name)
    }

    fn waydroid_app_launch_package_as_user(
        &self,
        package_name: &str,
        user: &str,
        session_env: &wroid_waydroid::WaydroidAppLaunchEnv,
    ) -> Result<()> {
        wroid_waydroid::app_launch_package_as_user(package_name, user, session_env)
    }

    fn waydroid_app_install(&self, path: &Path) -> Result<()> {
        wroid_waydroid::app_install(path)
    }

    fn waydroid_shell_current_activity(&self) -> Result<Option<CurrentAndroidActivity>> {
        Ok(
            wroid_waydroid::shell_current_activity()?.map(|activity| CurrentAndroidActivity {
                package_name: activity.package_name,
                activity_name: activity.activity_name,
                component_name: activity.component_name,
            }),
        )
    }

    fn waydroid_shell_wm_size(&self) -> Result<String> {
        wroid_waydroid::shell_wm_size()
    }

    fn waydroid_shell_wm_density(&self) -> Result<String> {
        wroid_waydroid::shell_wm_density()
    }
}

pub(crate) fn select_input_backend(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> SelectedInputBackend {
    match backend {
        InputBackend::Adb => SelectedInputBackend::Adb,
        InputBackend::WaydroidShell => SelectedInputBackend::WaydroidShell,
        InputBackend::Auto => {
            let has_adb_device = input_executor
                .adb_devices()
                .unwrap_or_default()
                .iter()
                .any(|device| device.state == "device");

            if has_adb_device {
                SelectedInputBackend::Adb
            } else {
                SelectedInputBackend::WaydroidShell
            }
        }
    }
}

pub(crate) fn execute_tap(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    x: u32,
    y: u32,
) -> Result<()> {
    match backend {
        SelectedInputBackend::Adb => input_executor.adb_tap(x, y),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_tap(x, y),
    }
}

pub(crate) fn execute_swipe(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration_ms: u64,
) -> Result<()> {
    match backend {
        SelectedInputBackend::Adb => input_executor.adb_swipe(x1, y1, x2, y2, duration_ms),
        SelectedInputBackend::WaydroidShell => {
            input_executor.waydroid_shell_swipe(x1, y1, x2, y2, duration_ms)
        }
    }
}

pub(crate) fn execute_keyevent(
    input_executor: &impl InputExecutor,
    backend: SelectedInputBackend,
    code: u32,
) -> Result<()> {
    match backend {
        SelectedInputBackend::Adb => input_executor.adb_keyevent(code),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_keyevent(code),
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::FakeInputExecutor;

    use super::*;

    #[test]
    fn auto_selects_adb_when_connected_device_exists() {
        let executor = FakeInputExecutor::with_devices(vec![
            wroid_adb::AdbDevice {
                serial: "offline-device".to_owned(),
                state: "offline".to_owned(),
            },
            wroid_adb::AdbDevice {
                serial: "connected-device".to_owned(),
                state: "device".to_owned(),
            },
        ]);

        let selected = select_input_backend(&executor, InputBackend::Auto);

        assert_eq!(selected, SelectedInputBackend::Adb);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn auto_selects_waydroid_shell_when_adb_has_no_connected_device() {
        let executor = FakeInputExecutor::with_device_state("offline-device", "offline");

        let selected = select_input_backend(&executor, InputBackend::Auto);

        assert_eq!(selected, SelectedInputBackend::WaydroidShell);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn auto_selects_waydroid_shell_when_adb_device_listing_fails() {
        let executor = FakeInputExecutor::with_device_error();

        let selected = select_input_backend(&executor, InputBackend::Auto);

        assert_eq!(selected, SelectedInputBackend::WaydroidShell);
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn explicit_backend_selection_does_not_query_adb_devices() {
        let executor = FakeInputExecutor::with_device_error();

        let selected = select_input_backend(&executor, InputBackend::Adb);

        assert_eq!(selected, SelectedInputBackend::Adb);
        assert_eq!(executor.device_queries.get(), 0);
    }
}
