use anyhow::{Context, Result};
use wroid_core::Resolution;

use crate::backend::{select_input_backend, InputExecutor, SelectedInputBackend};
use crate::cli::InputBackend;

pub(crate) fn detect_device_screen(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<Resolution> {
    let selected_backend = select_input_backend(input_executor, backend);
    detect_device_screen_with_selected_backend(input_executor, selected_backend)
}

pub(crate) fn detect_device_screen_with_selected_backend(
    input_executor: &impl InputExecutor,
    selected_backend: SelectedInputBackend,
) -> Result<Resolution> {
    let output = match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_wm_size(),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_wm_size(),
    }
    .with_context(|| format!("failed to query screen size via {selected_backend}"))?;

    parse_wm_size_output(&output).with_context(|| {
        format!(
            "failed to parse screen size from {selected_backend} output: {}",
            compact_command_output(&output)
        )
    })
}

pub(crate) fn detect_device_density(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<u32> {
    let selected_backend = select_input_backend(input_executor, backend);
    detect_device_density_with_selected_backend(input_executor, selected_backend)
}

pub(crate) fn detect_device_density_with_selected_backend(
    input_executor: &impl InputExecutor,
    selected_backend: SelectedInputBackend,
) -> Result<u32> {
    let output = match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_wm_density(),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_wm_density(),
    }
    .with_context(|| format!("failed to query density via {selected_backend}"))?;

    parse_wm_density_output(&output).with_context(|| {
        format!(
            "failed to parse density from {selected_backend} output: {}",
            compact_command_output(&output)
        )
    })
}

pub(crate) fn device_info_output(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<String> {
    let selected_backend = select_input_backend(input_executor, backend);
    let screen = detect_device_screen_with_selected_backend(input_executor, selected_backend)?;

    let mut output = format!("Screen: {screen}\n");
    match detect_device_density_with_selected_backend(input_executor, selected_backend) {
        Ok(density) => output.push_str(&format!("Density: {density}\n")),
        Err(error) => output.push_str(&format!(
            "Warning: density detection failed via {selected_backend}: {error:#}\n"
        )),
    }

    Ok(output)
}

pub(crate) fn parse_wm_size_output(output: &str) -> Option<Resolution> {
    parse_wm_size_output_with_prefix(output, "Override size:")
        .or_else(|| parse_wm_size_output_with_prefix(output, "Physical size:"))
}

fn parse_wm_size_output_with_prefix(output: &str, prefix: &str) -> Option<Resolution> {
    let value = output
        .lines()
        .find_map(|line| strip_wm_value_prefix(line, prefix))?;
    parse_resolution_value(value)
}

fn parse_resolution_value(value: &str) -> Option<Resolution> {
    let value = value.trim();
    let (width, height) = value.split_once('x')?;
    let width = width.trim().parse().ok()?;
    let height = height.trim().parse().ok()?;
    if width == 0 || height == 0 {
        return None;
    }

    Some(Resolution { width, height })
}

pub(crate) fn parse_wm_density_output(output: &str) -> Option<u32> {
    parse_wm_density_output_with_prefix(output, "Override density:")
        .or_else(|| parse_wm_density_output_with_prefix(output, "Physical density:"))
}

fn parse_wm_density_output_with_prefix(output: &str, prefix: &str) -> Option<u32> {
    let value = output
        .lines()
        .find_map(|line| strip_wm_value_prefix(line, prefix))?;
    let density = value.trim().parse().ok()?;
    if density == 0 {
        None
    } else {
        Some(density)
    }
}

fn strip_wm_value_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let trimmed = line.trim();
    trimmed.strip_prefix(prefix)
}

fn compact_command_output(output: &str) -> String {
    let compacted = output.split_whitespace().collect::<Vec<_>>().join(" ");
    if compacted.is_empty() {
        "<empty>".to_owned()
    } else {
        compacted
    }
}

#[cfg(test)]
mod tests {
    use wroid_core::Resolution;

    use crate::backend::SelectedInputBackend;
    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn wm_size_parser_accepts_physical_size() {
        assert_eq!(
            parse_wm_size_output("Physical size: 1920x1050\n"),
            Some(Resolution {
                width: 1920,
                height: 1050
            })
        );
    }

    #[test]
    fn wm_size_parser_accepts_override_size() {
        assert_eq!(
            parse_wm_size_output("Override size: 1280x720\n"),
            Some(Resolution {
                width: 1280,
                height: 720
            })
        );
    }

    #[test]
    fn wm_size_parser_rejects_unrelated_output() {
        assert_eq!(parse_wm_size_output("Display metrics unavailable\n"), None);
    }

    #[test]
    fn wm_density_parser_accepts_physical_density() {
        assert_eq!(
            parse_wm_density_output("Physical density: 180\n"),
            Some(180)
        );
    }

    #[test]
    fn wm_density_parser_accepts_override_density() {
        assert_eq!(
            parse_wm_density_output("Override density: 240\n"),
            Some(240)
        );
    }

    #[test]
    fn wm_density_parser_rejects_unrelated_output() {
        assert_eq!(
            parse_wm_density_output("Display density unavailable\n"),
            None
        );
    }

    #[test]
    fn device_info_formats_screen_and_density() {
        let executor = FakeInputExecutor::with_waydroid_screen_and_density(1920, 1050, 180);

        let output = device_info_output(&executor, InputBackend::WaydroidShell).unwrap();

        assert_eq!(output, "Screen: 1920x1050\nDensity: 180\n");
        assert_eq!(
            executor.calls(),
            vec![
                InputCall::WaydroidShellWmSize,
                InputCall::WaydroidShellWmDensity
            ]
        );
    }

    #[test]
    fn device_info_warns_when_density_detection_fails_after_screen() {
        let executor = FakeInputExecutor {
            waydroid_wm_size_output: "Physical size: 1920x1050\n".to_owned(),
            fail_density: true,
            ..FakeInputExecutor::default()
        };

        let output = device_info_output(&executor, InputBackend::WaydroidShell).unwrap();

        assert!(output.contains("Screen: 1920x1050\n"));
        assert!(output.contains("Warning: density detection failed via waydroid-shell"));
        assert!(output.contains("waydroid density failed"));
    }

    #[test]
    fn screen_detection_context_includes_backend_output() {
        let executor = FakeInputExecutor {
            waydroid_wm_size_output: "Display metrics unavailable\n".to_owned(),
            ..FakeInputExecutor::default()
        };

        let err = detect_device_screen_with_selected_backend(
            &executor,
            SelectedInputBackend::WaydroidShell,
        )
        .unwrap_err();

        assert!(err.to_string().contains("failed to parse screen size"));
        assert!(format!("{err:#}").contains("Display metrics unavailable"));
    }
}
