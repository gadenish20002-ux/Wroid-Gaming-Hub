use anyhow::Result;

use crate::backend::{InputExecutor, SelectedInputBackend};
use crate::cli::InputBackend;
use crate::device::{
    detect_device_density_with_selected_backend, detect_device_screen_with_selected_backend,
};
use crate::output::write_stdout;

pub(crate) fn doctor(input_executor: &impl InputExecutor, backend: InputBackend) -> Result<()> {
    write_stdout(&doctor_output(input_executor, backend))
}

fn doctor_output(input_executor: &impl InputExecutor, backend: InputBackend) -> String {
    let adb_available = wroid_adb::is_available();
    let adb_devices = if adb_available {
        match wroid_adb::devices() {
            Ok(devices) => DeviceQuery::Available(devices),
            Err(error) => DeviceQuery::Failed(format!("{error:#}")),
        }
    } else {
        DeviceQuery::Unavailable
    };

    let waydroid_available = wroid_waydroid::is_available();
    let waydroid_status = if waydroid_available {
        match wroid_waydroid::status() {
            Ok(status) => StatusQuery::Available(status),
            Err(error) => StatusQuery::Failed(format!("{error:#}")),
        }
    } else {
        StatusQuery::Unavailable
    };

    doctor_output_from_parts(
        input_executor,
        backend,
        adb_available,
        &adb_devices,
        waydroid_available,
        &waydroid_status,
    )
}

fn doctor_output_from_parts(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    adb_available: bool,
    adb_devices: &DeviceQuery,
    waydroid_available: bool,
    waydroid_status: &StatusQuery,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("adb: {}\n", availability(adb_available)));
    match adb_devices {
        DeviceQuery::Available(devices) => {
            let connected_count = connected_adb_device_count(devices);
            output.push_str(&format!(
                "adb connected devices: {connected_count}/{}\n",
                devices.len()
            ));
            for device in devices {
                output.push_str(&format!("  {} {}\n", device.serial, device.state));
            }
        }
        DeviceQuery::Failed(error) => {
            output.push_str(&format!("adb devices: unavailable ({error})\n"));
        }
        DeviceQuery::Unavailable => {
            output.push_str("adb devices: unavailable (adb missing)\n");
        }
    }

    output.push_str(&format!("waydroid: {}\n", availability(waydroid_available)));
    match waydroid_status {
        StatusQuery::Available(status) => {
            output.push_str("waydroid status:\n");
            output.push_str(status);
            if !status.ends_with('\n') {
                output.push('\n');
            }
        }
        StatusQuery::Failed(error) => {
            output.push_str(&format!("waydroid status: unavailable ({error})\n"));
        }
        StatusQuery::Unavailable => {
            output.push_str("waydroid status: unavailable (waydroid missing)\n");
        }
    }

    if let Some(warning) = waydroid_unknown_ip_warning(status_text(waydroid_status)) {
        output.push_str(warning);
        output.push('\n');
    }

    let has_adb_device = adb_devices
        .devices()
        .is_some_and(|devices| connected_adb_device_count(devices) > 0);
    let waydroid_running = status_text(waydroid_status).is_some_and(waydroid_is_running);
    if let Some(selected_backend) = doctor_probe_backend(backend, has_adb_device, waydroid_running)
    {
        append_screen_and_density(input_executor, selected_backend, &mut output);
    } else {
        output.push_str("screen: unavailable (no runnable backend detected)\n");
        output.push_str("density: unavailable (no runnable backend detected)\n");
    }

    output.push_str(&backend_recommendation(has_adb_device, waydroid_running));
    output.push('\n');
    output
}

fn append_screen_and_density(
    input_executor: &impl InputExecutor,
    selected_backend: SelectedInputBackend,
    output: &mut String,
) {
    match detect_device_screen_with_selected_backend(input_executor, selected_backend) {
        Ok(screen) => output.push_str(&format!("screen ({selected_backend}): {screen}\n")),
        Err(error) => output.push_str(&format!(
            "screen ({selected_backend}): unavailable ({error:#})\n"
        )),
    }

    match detect_device_density_with_selected_backend(input_executor, selected_backend) {
        Ok(density) => output.push_str(&format!("density ({selected_backend}): {density}\n")),
        Err(error) => output.push_str(&format!(
            "density ({selected_backend}): unavailable ({error:#})\n"
        )),
    }
}

fn doctor_probe_backend(
    requested_backend: InputBackend,
    has_adb_device: bool,
    waydroid_running: bool,
) -> Option<SelectedInputBackend> {
    match requested_backend {
        InputBackend::Adb => Some(SelectedInputBackend::Adb),
        InputBackend::WaydroidShell => Some(SelectedInputBackend::WaydroidShell),
        InputBackend::Auto if has_adb_device => Some(SelectedInputBackend::Adb),
        InputBackend::Auto if waydroid_running => Some(SelectedInputBackend::WaydroidShell),
        InputBackend::Auto => None,
    }
}

fn backend_recommendation(has_adb_device: bool, waydroid_running: bool) -> String {
    if has_adb_device {
        "backend recommendation: adb (ADB has a connected device)".to_owned()
    } else if waydroid_running {
        "backend recommendation: waydroid-shell (Waydroid is running and ADB has no connected device)"
            .to_owned()
    } else {
        "backend recommendation: start Waydroid or connect an ADB device before running input commands"
            .to_owned()
    }
}

fn waydroid_unknown_ip_warning(status: Option<&str>) -> Option<&'static str> {
    let status = status?;
    if waydroid_is_running(status) && waydroid_ip_is_unknown(status) {
        Some(
            "warning: Waydroid session/container is running but IP address is UNKNOWN; ADB may not connect, but --backend waydroid-shell can still work.",
        )
    } else {
        None
    }
}

fn waydroid_is_running(status: &str) -> bool {
    status_value(status, "Session").is_some_and(|value| value.eq_ignore_ascii_case("RUNNING"))
        && status_value(status, "Container")
            .is_some_and(|value| value.eq_ignore_ascii_case("RUNNING"))
}

fn waydroid_ip_is_unknown(status: &str) -> bool {
    status_value(status, "IP address").is_some_and(|value| value.eq_ignore_ascii_case("UNKNOWN"))
}

fn status_value<'a>(status: &'a str, key: &str) -> Option<&'a str> {
    status.lines().find_map(|line| {
        let (candidate, value) = line.split_once(':')?;
        if candidate.trim().eq_ignore_ascii_case(key) {
            Some(value.trim())
        } else {
            None
        }
    })
}

fn connected_adb_device_count(devices: &[wroid_adb::AdbDevice]) -> usize {
    devices
        .iter()
        .filter(|device| device.state == "device")
        .count()
}

fn availability(is_available: bool) -> &'static str {
    if is_available {
        "available"
    } else {
        "missing"
    }
}

fn status_text(status: &StatusQuery) -> Option<&str> {
    match status {
        StatusQuery::Available(status) => Some(status),
        StatusQuery::Failed(_) | StatusQuery::Unavailable => None,
    }
}

#[derive(Debug)]
enum DeviceQuery {
    Available(Vec<wroid_adb::AdbDevice>),
    Failed(String),
    Unavailable,
}

impl DeviceQuery {
    fn devices(&self) -> Option<&[wroid_adb::AdbDevice]> {
        match self {
            Self::Available(devices) => Some(devices),
            Self::Failed(_) | Self::Unavailable => None,
        }
    }
}

#[derive(Debug)]
enum StatusQuery {
    Available(String),
    Failed(String),
    Unavailable,
}

#[cfg(test)]
mod tests {
    use crate::test_support::FakeInputExecutor;

    use super::*;

    #[test]
    fn recommendation_prefers_adb_when_device_connected() {
        assert_eq!(
            backend_recommendation(true, true),
            "backend recommendation: adb (ADB has a connected device)"
        );
    }

    #[test]
    fn recommendation_uses_waydroid_shell_when_waydroid_runs_without_adb() {
        assert_eq!(
            backend_recommendation(false, true),
            "backend recommendation: waydroid-shell (Waydroid is running and ADB has no connected device)"
        );
    }

    #[test]
    fn recommendation_explains_when_no_backend_is_ready() {
        assert!(backend_recommendation(false, false).contains("start Waydroid"));
    }

    #[test]
    fn detects_waydroid_running_status() {
        assert!(waydroid_is_running(
            "Session:\tRUNNING\nContainer:\tRUNNING\nIP address:\tUNKNOWN\n"
        ));
        assert!(!waydroid_is_running(
            "Session:\tRUNNING\nContainer:\tSTOPPED\nIP address:\tUNKNOWN\n"
        ));
    }

    #[test]
    fn detects_unknown_waydroid_ip_issue() {
        let warning = waydroid_unknown_ip_warning(Some(
            "Session: RUNNING\nContainer: RUNNING\nIP address: UNKNOWN\n",
        ))
        .unwrap();

        assert!(warning.contains("ADB may not connect"));
        assert!(warning.contains("waydroid-shell"));
    }

    #[test]
    fn doctor_output_reports_screen_density_and_recommendation() {
        let executor = FakeInputExecutor::with_waydroid_screen_and_density(1920, 1050, 180);
        let output = doctor_output_from_parts(
            &executor,
            InputBackend::Auto,
            true,
            &DeviceQuery::Available(Vec::new()),
            true,
            &StatusQuery::Available(
                "Session: RUNNING\nContainer: RUNNING\nIP address: UNKNOWN\n".to_owned(),
            ),
        );

        assert!(output.contains("adb: available\n"));
        assert!(output.contains("adb connected devices: 0/0\n"));
        assert!(output.contains("waydroid: available\n"));
        assert!(output.contains("screen (waydroid-shell): 1920x1050\n"));
        assert!(output.contains("density (waydroid-shell): 180\n"));
        assert!(output.contains("backend recommendation: waydroid-shell"));
        assert!(output.contains("ADB may not connect"));
    }
}
