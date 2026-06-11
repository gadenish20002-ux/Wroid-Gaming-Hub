use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::backend::{select_input_backend, InputExecutor, SelectedInputBackend};
use crate::cli::InputBackend;
use crate::output::write_stdout;

pub(crate) fn app_list(input_executor: &impl InputExecutor, backend: InputBackend) -> Result<()> {
    let packages = app_packages(input_executor, backend)?;

    write_stdout(&package_listing(&packages))
}

pub(crate) fn app_packages(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<Vec<String>> {
    match select_input_backend(input_executor, backend) {
        SelectedInputBackend::Adb => input_executor.adb_list_packages(),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_app_list_packages(),
    }
}

pub(crate) fn app_launch(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    package_name: &str,
) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    launch_profile_package(input_executor, selected_backend, package_name)?;

    println!("Launched {package_name} via {selected_backend}.");
    Ok(())
}

pub(crate) fn launch_profile_package(
    input_executor: &impl InputExecutor,
    selected_backend: SelectedInputBackend,
    package_name: &str,
) -> Result<()> {
    match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_launch_package(package_name),
        SelectedInputBackend::WaydroidShell => {
            input_executor.waydroid_app_launch_package(package_name)
        }
    }
    .with_context(|| {
        format!("failed to launch Android package {package_name} via {selected_backend}")
    })
}

pub(crate) fn app_install_apk(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
    path: PathBuf,
) -> Result<()> {
    ensure_apk_path_exists(&path)?;

    let selected_backend = select_input_backend(input_executor, backend);
    match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_install_apk(&path),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_app_install(&path),
    }
    .with_context(|| {
        format!(
            "failed to install APK {} via {selected_backend}",
            path.display()
        )
    })?;

    println!("Installed APK {} via {selected_backend}.", path.display());
    Ok(())
}

pub(crate) fn app_current(
    input_executor: &impl InputExecutor,
    backend: InputBackend,
) -> Result<()> {
    let selected_backend = select_input_backend(input_executor, backend);
    let current = match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_current_activity(),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_shell_current_activity(),
    }
    .with_context(|| format!("failed to query current Android activity via {selected_backend}"))?;

    if let Some(activity) = current {
        println!("Current Android activity: {}", activity.component_name);
        println!("Package: {}", activity.package_name);
        println!("Activity: {}", activity.activity_name);
    } else {
        println!(
            "Current Android activity: unavailable (dumpsys did not report a focused or resumed activity)."
        );
    }

    Ok(())
}

pub(crate) fn ensure_apk_path_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!("APK path does not exist: {}", path.display());
    }

    if !path.is_file() {
        bail!("APK path is not a file: {}", path.display());
    }

    Ok(())
}

pub(crate) fn package_listing(packages: &[String]) -> String {
    let mut output = String::new();
    for package in packages {
        output.push_str(package);
        output.push('\n');
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::test_support::{FakeInputExecutor, InputCall};

    use super::*;

    #[test]
    fn app_launch_dispatches_to_explicit_waydroid_shell_backend() {
        let executor = FakeInputExecutor::default();

        app_launch(&executor, InputBackend::WaydroidShell, "com.example.game").unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::WaydroidAppLaunchPackage(
                "com.example.game".to_owned()
            )]
        );
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn app_launch_dispatches_to_auto_selected_adb_backend() {
        let executor = FakeInputExecutor::with_device_state("connected-device", "device");

        app_launch(&executor, InputBackend::Auto, "com.example.game").unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::AdbLaunchPackage("com.example.game".to_owned())]
        );
        assert_eq!(executor.device_queries.get(), 1);
    }

    #[test]
    fn app_list_uses_explicit_backend_without_adb_auto_selection() {
        let executor = FakeInputExecutor::with_waydroid_packages(vec!["com.example.game"]);

        let packages = app_packages(&executor, InputBackend::WaydroidShell).unwrap();

        assert_eq!(packages, vec!["com.example.game"]);
        assert_eq!(executor.device_queries.get(), 0);
        assert!(executor.calls().is_empty());
    }

    #[test]
    fn package_listing_prints_one_package_per_line() {
        assert_eq!(
            package_listing(&[
                "com.example.game".to_owned(),
                "org.example.second".to_owned()
            ]),
            "com.example.game\norg.example.second\n"
        );
    }

    #[test]
    fn apk_path_validation_rejects_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.apk");

        let err = ensure_apk_path_exists(&path).unwrap_err();

        assert!(err.to_string().contains("APK path does not exist"));
    }

    #[test]
    fn app_install_apk_validates_path_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.apk");
        let executor = FakeInputExecutor::default();

        let err = app_install_apk(&executor, InputBackend::Adb, path).unwrap_err();

        assert!(err.to_string().contains("APK path does not exist"));
        assert!(executor.calls().is_empty());
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn app_install_apk_dispatches_to_adb_after_path_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.apk");
        fs::write(&path, b"fake apk").unwrap();
        let executor = FakeInputExecutor::default();

        app_install_apk(&executor, InputBackend::Adb, path.clone()).unwrap();

        assert_eq!(executor.calls(), vec![InputCall::AdbInstallApk(path)]);
        assert_eq!(executor.device_queries.get(), 0);
    }
}
