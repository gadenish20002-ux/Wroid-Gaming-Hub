use std::env;
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
    allow_any_extension: bool,
) -> Result<()> {
    let apk_path = validate_apk_path(&path, allow_any_extension)?;

    let selected_backend = select_input_backend(input_executor, backend);
    println!(
        "Installing APK {} via {selected_backend}...",
        apk_path.display()
    );
    match selected_backend {
        SelectedInputBackend::Adb => input_executor.adb_install_apk(&apk_path),
        SelectedInputBackend::WaydroidShell => input_executor.waydroid_app_install(&apk_path),
    }
    .with_context(|| {
        format!(
            "failed to install APK {} via {selected_backend}",
            apk_path.display()
        )
    })?;

    println!(
        "Installed APK {} via {selected_backend}.",
        apk_path.display()
    );
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

pub(crate) fn validate_apk_path(path: &Path, allow_any_extension: bool) -> Result<PathBuf> {
    let display_path = absolute_display_path(path)?;

    if !path.exists() {
        bail!("APK path does not exist: {}", display_path.display());
    }

    if !path.is_file() {
        bail!("APK path is not a file: {}", display_path.display());
    }

    if !allow_any_extension && !has_apk_extension(path) {
        bail!(
            "APK path must end with .apk: {}. Pass --allow-any-extension to install anyway.",
            display_path.display()
        );
    }

    path.canonicalize()
        .with_context(|| format!("failed to resolve APK path {}", display_path.display()))
}

fn absolute_display_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(env::current_dir()
            .context("failed to determine current directory")?
            .join(path))
    }
}

fn has_apk_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("apk"))
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

        let err = validate_apk_path(&path, false).unwrap_err();

        assert!(err.to_string().contains("APK path does not exist"));
        assert!(err.to_string().contains(path.to_str().unwrap()));
    }

    #[test]
    fn app_install_apk_validates_path_before_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.apk");
        let executor = FakeInputExecutor::default();

        let err = app_install_apk(&executor, InputBackend::Adb, path, false).unwrap_err();

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

        app_install_apk(&executor, InputBackend::Adb, path.clone(), false).unwrap();

        assert_eq!(
            executor.calls(),
            vec![InputCall::AdbInstallApk(path.canonicalize().unwrap())]
        );
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn apk_path_validation_rejects_non_apk_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.zip");
        fs::write(&path, b"fake apk").unwrap();

        let err = validate_apk_path(&path, false).unwrap_err();

        assert!(err.to_string().contains("must end with .apk"));
        assert!(err.to_string().contains("--allow-any-extension"));
    }

    #[test]
    fn apk_path_validation_accepts_non_apk_with_allow_flag() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.zip");
        fs::write(&path, b"fake apk").unwrap();

        let validated = validate_apk_path(&path, true).unwrap();

        assert_eq!(validated, path.canonicalize().unwrap());
    }
}
