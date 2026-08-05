use std::env;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use wroid_android::{
    assess_abi_compatibility, inspect_package, AbiCompatibility, PackageFormat, PackageInspection,
};

use crate::backend::{select_input_backend, InputExecutor, SelectedInputBackend};
use crate::cli::InputBackend;
use crate::output::write_stdout;

use super::compatibility::CompatibilityReport;

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
    force_incompatible: bool,
) -> Result<()> {
    let apk_path = validate_apk_path(&path, allow_any_extension)?;
    let preflight = package_preflight(&apk_path)?;
    validate_install_preflight(&preflight, force_incompatible)?;
    print_preflight_summary(&preflight);

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

pub(crate) fn app_inspect(path: PathBuf, json: bool) -> Result<()> {
    let display_path = absolute_display_path(&path)?;
    if !path.is_file() {
        bail!(
            "package path is not a file or does not exist: {}",
            display_path.display()
        );
    }
    let preflight = package_preflight(&path)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&preflight)?);
    } else {
        print_inspection(&preflight);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackagePreflight {
    pub(crate) artifact: PackageInspection,
    pub(crate) abi_compatibility: AbiCompatibility,
    pub(crate) android_abis: Vec<String>,
    pub(crate) arm_translation: Option<bool>,
}

pub(crate) fn package_preflight(path: &Path) -> Result<PackagePreflight> {
    let artifact = inspect_package(path)
        .with_context(|| format!("failed to inspect Android package {}", path.display()))?;
    let compatibility = CompatibilityReport::probe();
    let android_abis = compatibility.android_abis();
    let arm_translation = compatibility.arm_translation_status();
    let abi_compatibility =
        assess_abi_compatibility(&artifact.native_abis, &android_abis, arm_translation);
    Ok(PackagePreflight {
        artifact,
        abi_compatibility,
        android_abis,
        arm_translation,
    })
}

pub(crate) fn validate_install_preflight(
    preflight: &PackagePreflight,
    force_incompatible: bool,
) -> Result<()> {
    match preflight.artifact.format {
        PackageFormat::Apk => {}
        PackageFormat::SplitApkBundle | PackageFormat::Xapk | PackageFormat::Apkm => bail!(
            "{} contains {} embedded APK(s); single-APK install cannot install bundles yet",
            preflight.artifact.format.label(),
            preflight.artifact.embedded_apks.len()
        ),
        PackageFormat::Obb => bail!(
            "OBB is game data, not an installable APK; install its matching base package first"
        ),
        PackageFormat::Unknown => {
            bail!("the file is not a recognized APK or supported Android package bundle")
        }
    }
    if !preflight.artifact.has_android_manifest {
        bail!("APK is missing AndroidManifest.xml");
    }
    if preflight.artifact.encrypted_entries > 0 {
        bail!(
            "APK contains {} encrypted archive entry/entries and cannot be inspected safely",
            preflight.artifact.encrypted_entries
        );
    }
    if preflight.abi_compatibility.blocks_install() && !force_incompatible {
        let detail = match preflight.abi_compatibility {
            AbiCompatibility::ArmTranslationMissing => {
                "the APK contains ARM native code but Waydroid ARM translation is disabled"
            }
            AbiCompatibility::Incompatible => {
                "the APK native ABIs do not match the configured Waydroid ABIs"
            }
            _ => unreachable!("only blocking compatibility states reach this branch"),
        };
        bail!("{detail}; pass --force-incompatible only if you accept a likely install/runtime failure");
    }
    Ok(())
}

fn print_preflight_summary(preflight: &PackagePreflight) {
    println!(
        "Package preflight: {} · ABI {}",
        preflight.artifact.format.label(),
        abi_compatibility_label(preflight.abi_compatibility)
    );
}

fn print_inspection(preflight: &PackagePreflight) {
    println!("Path: {}", preflight.artifact.path.display());
    println!("Format: {}", preflight.artifact.format.label());
    println!("Size: {}", format_file_size(preflight.artifact.file_size));
    println!("Archive entries: {}", preflight.artifact.archive_entries);
    println!(
        "Android manifest: {}",
        yes_no(preflight.artifact.has_android_manifest)
    );
    println!("DEX bytecode: {}", yes_no(preflight.artifact.has_dex));
    println!("Resources: {}", yes_no(preflight.artifact.has_resources));
    println!(
        "Native ABIs: {}",
        joined_or(&preflight.artifact.native_abis, "none (universal)")
    );
    println!(
        "Embedded APKs: {}",
        joined_or(&preflight.artifact.embedded_apks, "none")
    );
    println!(
        "OBB files: {}",
        joined_or(&preflight.artifact.obb_files, "none")
    );
    println!(
        "Encrypted entries: {}",
        preflight.artifact.encrypted_entries
    );
    println!(
        "Waydroid ABIs: {}",
        joined_or(&preflight.android_abis, "unknown")
    );
    println!(
        "ABI compatibility: {}",
        abi_compatibility_label(preflight.abi_compatibility)
    );
}

fn joined_or(values: &[String], fallback: &str) -> String {
    if values.is_empty() {
        fallback.to_owned()
    } else {
        values.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn abi_compatibility_label(compatibility: AbiCompatibility) -> &'static str {
    match compatibility {
        AbiCompatibility::Universal => "universal",
        AbiCompatibility::Native => "native",
        AbiCompatibility::NativeTranslation => "ARM translation",
        AbiCompatibility::Unknown => "unknown",
        AbiCompatibility::ArmTranslationMissing => "ARM translation missing",
        AbiCompatibility::Incompatible => "incompatible",
    }
}

fn format_file_size(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB)
    }
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

    fn write_test_zip(path: &Path, names: &[&str]) {
        const LOCAL_SIGNATURE: u32 = 0x0403_4b50;
        const CENTRAL_SIGNATURE: u32 = 0x0201_4b50;
        const EOCD_SIGNATURE: u32 = 0x0605_4b50;
        let mut local = Vec::new();
        let mut central = Vec::new();
        for name in names {
            let offset = local.len() as u32;
            local.extend_from_slice(&LOCAL_SIGNATURE.to_le_bytes());
            local.extend_from_slice(&20_u16.to_le_bytes());
            local.extend_from_slice(&0_u16.to_le_bytes());
            local.extend_from_slice(&0_u16.to_le_bytes());
            local.extend_from_slice(&[0; 16]);
            local.extend_from_slice(&(name.len() as u16).to_le_bytes());
            local.extend_from_slice(&0_u16.to_le_bytes());
            local.extend_from_slice(name.as_bytes());

            central.extend_from_slice(&CENTRAL_SIGNATURE.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&[0; 16]);
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = local.len() as u32;
        let central_size = central.len() as u32;
        local.extend_from_slice(&central);
        local.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        local.extend_from_slice(&[0; 4]);
        local.extend_from_slice(&(names.len() as u16).to_le_bytes());
        local.extend_from_slice(&(names.len() as u16).to_le_bytes());
        local.extend_from_slice(&central_size.to_le_bytes());
        local.extend_from_slice(&central_offset.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        fs::write(path, local).unwrap();
    }

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

        let err = app_install_apk(&executor, InputBackend::Adb, path, false, false).unwrap_err();

        assert!(err.to_string().contains("APK path does not exist"));
        assert!(executor.calls().is_empty());
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn app_install_apk_dispatches_to_adb_after_path_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.apk");
        write_test_zip(&path, &["AndroidManifest.xml", "classes.dex"]);
        let executor = FakeInputExecutor::default();

        app_install_apk(&executor, InputBackend::Adb, path.clone(), false, false).unwrap();

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

    #[test]
    fn package_preflight_detects_bundle_before_backend_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("game.xapk");
        write_test_zip(&path, &["manifest.json", "base.apk", "config.arm64.apk"]);
        let executor = FakeInputExecutor::default();

        let error = app_install_apk(&executor, InputBackend::Adb, path, true, false).unwrap_err();

        assert!(error.to_string().contains("embedded APK"));
        assert!(executor.calls().is_empty());
        assert_eq!(executor.device_queries.get(), 0);
    }

    #[test]
    fn confirmed_incompatible_abi_needs_explicit_override() {
        let preflight = PackagePreflight {
            artifact: PackageInspection {
                path: PathBuf::from("/game.apk"),
                format: PackageFormat::Apk,
                file_size: 100,
                archive_entries: 3,
                has_android_manifest: true,
                has_dex: true,
                has_resources: false,
                native_abis: vec!["x86".to_owned()],
                embedded_apks: Vec::new(),
                obb_files: Vec::new(),
                encrypted_entries: 0,
            },
            abi_compatibility: AbiCompatibility::Incompatible,
            android_abis: vec!["arm64-v8a".to_owned()],
            arm_translation: Some(false),
        };

        assert!(validate_install_preflight(&preflight, false).is_err());
        validate_install_preflight(&preflight, true).unwrap();
    }
}
