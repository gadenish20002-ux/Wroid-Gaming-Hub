//! Safe, dependency-light inspection of local Android package artifacts.
//!
//! Inspection reads only the ZIP central directory. It never extracts archive
//! contents, executes Android build tools, or trusts the filename extension as
//! proof of package format.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const CENTRAL_ENTRY_SIGNATURE: u32 = 0x0201_4b50;
const EOCD_BYTES: usize = 22;
const MAX_ZIP_COMMENT: usize = u16::MAX as usize;
const MAX_CENTRAL_DIRECTORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_ENTRY_NAME_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PackageFormat {
    Apk,
    SplitApkBundle,
    Xapk,
    Apkm,
    Obb,
    Unknown,
}

impl PackageFormat {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Apk => "APK",
            Self::SplitApkBundle => "split APK bundle",
            Self::Xapk => "XAPK",
            Self::Apkm => "APKM",
            Self::Obb => "OBB",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageInspection {
    pub path: PathBuf,
    pub format: PackageFormat,
    pub file_size: u64,
    pub archive_entries: usize,
    pub has_android_manifest: bool,
    pub has_dex: bool,
    pub has_resources: bool,
    pub native_abis: Vec<String>,
    pub embedded_apks: Vec<String>,
    pub obb_files: Vec<String>,
    pub encrypted_entries: usize,
}

impl PackageInspection {
    pub fn is_installable_single_apk(&self) -> bool {
        self.format == PackageFormat::Apk
            && self.has_android_manifest
            && self.encrypted_entries == 0
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AbiCompatibility {
    Universal,
    Native,
    NativeTranslation,
    Unknown,
    ArmTranslationMissing,
    Incompatible,
}

impl AbiCompatibility {
    pub const fn blocks_install(self) -> bool {
        matches!(self, Self::ArmTranslationMissing | Self::Incompatible)
    }
}

#[derive(Debug, Error)]
pub enum PackageInspectionError {
    #[error("package path is not a regular file: {0}")]
    NotAFile(PathBuf),
    #[error("package metadata could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("invalid or unsupported ZIP package: {0}")]
    InvalidArchive(String),
}

pub fn inspect_package(
    path: impl AsRef<Path>,
) -> Result<PackageInspection, PackageInspectionError> {
    let path = path.as_ref();
    let metadata = fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(PackageInspectionError::NotAFile(path.to_path_buf()));
    }
    let resolved = path.canonicalize()?;
    let extension = lower_extension(&resolved);
    if extension.as_deref() == Some("obb") {
        return Ok(PackageInspection {
            path: resolved,
            format: PackageFormat::Obb,
            file_size: metadata.len(),
            archive_entries: 0,
            has_android_manifest: false,
            has_dex: false,
            has_resources: false,
            native_abis: Vec::new(),
            embedded_apks: Vec::new(),
            obb_files: Vec::new(),
            encrypted_entries: 0,
        });
    }

    let entries = read_central_directory(&resolved, metadata.len())?;
    let names = entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<BTreeSet<_>>();
    let has_android_manifest = names.contains("AndroidManifest.xml");
    let has_dex = entries.iter().any(|entry| {
        entry.name == "classes.dex"
            || (entry.name.starts_with("classes")
                && entry.name.ends_with(".dex")
                && entry.name[7..entry.name.len() - 4]
                    .bytes()
                    .all(|byte| byte.is_ascii_digit()))
    });
    let has_resources = names.contains("resources.arsc");
    let native_abis = entries
        .iter()
        .filter_map(|entry| native_abi_from_entry(&entry.name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let embedded_apks = entries
        .iter()
        .filter(|entry| !entry.directory && has_extension(&entry.name, "apk"))
        .map(|entry| entry.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let obb_files = entries
        .iter()
        .filter(|entry| !entry.directory && has_extension(&entry.name, "obb"))
        .map(|entry| entry.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let format = detect_format(
        extension.as_deref(),
        has_android_manifest,
        &names,
        &embedded_apks,
    );

    Ok(PackageInspection {
        path: resolved,
        format,
        file_size: metadata.len(),
        archive_entries: entries.len(),
        has_android_manifest,
        has_dex,
        has_resources,
        native_abis,
        embedded_apks,
        obb_files,
        encrypted_entries: entries.iter().filter(|entry| entry.encrypted).count(),
    })
}

pub fn assess_abi_compatibility(
    package_abis: &[String],
    android_abis: &[String],
    arm_translation: Option<bool>,
) -> AbiCompatibility {
    if package_abis.is_empty() {
        return AbiCompatibility::Universal;
    }
    if package_abis
        .iter()
        .any(|package| android_abis.iter().any(|android| abi_eq(package, android)))
    {
        return AbiCompatibility::Native;
    }
    let has_arm = package_abis.iter().any(|abi| is_arm_abi(abi));
    if has_arm {
        return match arm_translation {
            Some(true) => AbiCompatibility::NativeTranslation,
            Some(false) => AbiCompatibility::ArmTranslationMissing,
            None => AbiCompatibility::Unknown,
        };
    }
    if android_abis.is_empty() {
        AbiCompatibility::Unknown
    } else {
        AbiCompatibility::Incompatible
    }
}

fn detect_format(
    extension: Option<&str>,
    has_android_manifest: bool,
    names: &BTreeSet<&str>,
    embedded_apks: &[String],
) -> PackageFormat {
    if has_android_manifest {
        return PackageFormat::Apk;
    }
    if embedded_apks.is_empty() {
        return PackageFormat::Unknown;
    }
    match extension {
        Some("xapk") => PackageFormat::Xapk,
        Some("apkm") => PackageFormat::Apkm,
        Some("apks") => PackageFormat::SplitApkBundle,
        _ if names.contains("manifest.json") => PackageFormat::Xapk,
        _ if names.contains("info.json") => PackageFormat::Apkm,
        _ => PackageFormat::SplitApkBundle,
    }
}

fn native_abi_from_entry(name: &str) -> Option<String> {
    let mut parts = name.split('/');
    if parts.next()? != "lib" {
        return None;
    }
    let abi = parts.next()?;
    let library = parts.next()?;
    if parts.next().is_some()
        || !library.ends_with(".so")
        || abi.is_empty()
        || !abi
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(abi.to_ascii_lowercase())
}

fn abi_eq(left: &str, right: &str) -> bool {
    normalize_abi(left) == normalize_abi(right)
}

fn normalize_abi(abi: &str) -> String {
    let normalized = abi.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "arm64" => "arm64-v8a".to_owned(),
        "arm" => "armeabi-v7a".to_owned(),
        "amd64" => "x86_64".to_owned(),
        "i686" => "x86".to_owned(),
        _ => normalized,
    }
}

fn is_arm_abi(abi: &str) -> bool {
    let abi = abi.to_ascii_lowercase();
    abi.starts_with("arm") || abi.starts_with("armeabi")
}

fn lower_extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
}

fn has_extension(name: &str, extension: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

#[derive(Debug)]
struct ArchiveEntry {
    name: String,
    encrypted: bool,
    directory: bool,
}

fn read_central_directory(
    path: &Path,
    file_size: u64,
) -> Result<Vec<ArchiveEntry>, PackageInspectionError> {
    if file_size < EOCD_BYTES as u64 {
        return Err(invalid_archive("file is too small to be a ZIP archive"));
    }
    let tail_len = file_size.min((EOCD_BYTES + MAX_ZIP_COMMENT) as u64) as usize;
    let mut file = File::open(path)?;
    file.seek(SeekFrom::End(-(tail_len as i64)))?;
    let mut tail = vec![0_u8; tail_len];
    file.read_exact(&mut tail)?;
    let eocd_index = find_eocd(&tail)
        .ok_or_else(|| invalid_archive("end-of-central-directory record is missing"))?;
    if eocd_index + EOCD_BYTES > tail.len() {
        return Err(invalid_archive("truncated end-of-central-directory record"));
    }
    let eocd = &tail[eocd_index..];
    let disk = read_u16(eocd, 4)?;
    let central_disk = read_u16(eocd, 6)?;
    let disk_entries = read_u16(eocd, 8)?;
    let total_entries = read_u16(eocd, 10)?;
    let central_size = u64::from(read_u32(eocd, 12)?);
    let central_offset = u64::from(read_u32(eocd, 16)?);
    let comment_len = usize::from(read_u16(eocd, 20)?);
    if disk != 0 || central_disk != 0 || disk_entries != total_entries {
        return Err(invalid_archive("multi-disk ZIP archives are not supported"));
    }
    if total_entries == u16::MAX
        || central_size == u64::from(u32::MAX)
        || central_offset == u64::from(u32::MAX)
    {
        return Err(invalid_archive("ZIP64 package metadata is not supported"));
    }
    debug_assert_eq!(eocd_index + EOCD_BYTES + comment_len, tail.len());
    if usize::from(total_entries) > MAX_ARCHIVE_ENTRIES
        || central_size > MAX_CENTRAL_DIRECTORY_BYTES
        || central_offset
            .checked_add(central_size)
            .is_none_or(|end| end > file_size)
    {
        return Err(invalid_archive(
            "ZIP central directory exceeds safety limits",
        ));
    }

    file.seek(SeekFrom::Start(central_offset))?;
    let mut bytes = vec![0_u8; central_size as usize];
    file.read_exact(&mut bytes)?;
    parse_central_entries(&bytes, usize::from(total_entries))
}

fn parse_central_entries(
    bytes: &[u8],
    expected_entries: usize,
) -> Result<Vec<ArchiveEntry>, PackageInspectionError> {
    let mut entries = Vec::with_capacity(expected_entries);
    let mut offset = 0_usize;
    while offset < bytes.len() {
        if entries.len() >= MAX_ARCHIVE_ENTRIES || offset + 46 > bytes.len() {
            return Err(invalid_archive("truncated ZIP central directory entry"));
        }
        let entry = &bytes[offset..];
        if read_u32(entry, 0)? != CENTRAL_ENTRY_SIGNATURE {
            return Err(invalid_archive(
                "unexpected record in ZIP central directory",
            ));
        }
        let flags = read_u16(entry, 8)?;
        let compressed_size = read_u32(entry, 20)?;
        let uncompressed_size = read_u32(entry, 24)?;
        let name_len = usize::from(read_u16(entry, 28)?);
        let extra_len = usize::from(read_u16(entry, 30)?);
        let comment_len = usize::from(read_u16(entry, 32)?);
        let disk_start = read_u16(entry, 34)?;
        let local_offset = read_u32(entry, 42)?;
        if name_len == 0 || name_len > MAX_ENTRY_NAME_BYTES {
            return Err(invalid_archive("ZIP entry name exceeds safety limits"));
        }
        if disk_start != 0
            || compressed_size == u32::MAX
            || uncompressed_size == u32::MAX
            || local_offset == u32::MAX
        {
            return Err(invalid_archive("ZIP64 entry metadata is not supported"));
        }
        let record_len = 46_usize
            .checked_add(name_len)
            .and_then(|value| value.checked_add(extra_len))
            .and_then(|value| value.checked_add(comment_len))
            .ok_or_else(|| invalid_archive("ZIP entry length overflow"))?;
        if offset
            .checked_add(record_len)
            .is_none_or(|end| end > bytes.len())
        {
            return Err(invalid_archive("truncated ZIP central directory entry"));
        }
        let name_bytes = &entry[46..46 + name_len];
        if name_bytes.contains(&0) {
            return Err(invalid_archive("ZIP entry name contains a NUL byte"));
        }
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| invalid_archive("ZIP entry name is not valid UTF-8"))?
            .to_owned();
        if name.chars().any(char::is_control) {
            return Err(invalid_archive(
                "ZIP entry name contains a control character",
            ));
        }
        entries.push(ArchiveEntry {
            directory: name.ends_with('/'),
            encrypted: flags & 1 != 0,
            name,
        });
        offset += record_len;
    }
    if entries.len() != expected_entries {
        return Err(invalid_archive(format!(
            "ZIP declared {expected_entries} entries but contained {}",
            entries.len()
        )));
    }
    Ok(entries)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, PackageInspectionError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| invalid_archive("truncated ZIP integer"))?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, PackageInspectionError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| invalid_archive("truncated ZIP integer"))?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn little_u32(bytes: &[u8]) -> Option<u32> {
    let bytes = bytes.get(..4)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn find_eocd(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < EOCD_BYTES {
        return None;
    }
    (0..=bytes.len() - EOCD_BYTES).rev().find(|&index| {
        little_u32(&bytes[index..]) == Some(EOCD_SIGNATURE)
            && bytes
                .get(index + 20..index + 22)
                .map(|value| usize::from(u16::from_le_bytes([value[0], value[1]])))
                .is_some_and(|comment_len| index + EOCD_BYTES + comment_len == bytes.len())
    })
}

fn invalid_archive(message: impl Into<String>) -> PackageInspectionError {
    PackageInspectionError::InvalidArchive(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn stored_zip(entries: &[(&str, bool)]) -> Vec<u8> {
        let mut local = Vec::new();
        let mut central = Vec::new();
        for (name, encrypted) in entries {
            let offset = local.len() as u32;
            local.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            local.extend_from_slice(&20_u16.to_le_bytes());
            local.extend_from_slice(&u16::from(*encrypted).to_le_bytes());
            local.extend_from_slice(&0_u16.to_le_bytes());
            local.extend_from_slice(&[0; 16]);
            local.extend_from_slice(&(name.len() as u16).to_le_bytes());
            local.extend_from_slice(&0_u16.to_le_bytes());
            local.extend_from_slice(name.as_bytes());

            central.extend_from_slice(&CENTRAL_ENTRY_SIGNATURE.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&u16::from(*encrypted).to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&[0; 16]);
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = local.len() as u32;
        let central_size = central.len() as u32;
        local.extend_from_slice(&central);
        local.extend_from_slice(&EOCD_SIGNATURE.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        local.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        local.extend_from_slice(&central_size.to_le_bytes());
        local.extend_from_slice(&central_offset.to_le_bytes());
        local.extend_from_slice(&0_u16.to_le_bytes());
        local
    }

    fn inspect_fixture(name: &str, entries: &[(&str, bool)]) -> PackageInspection {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(name);
        let mut file = File::create(&path).unwrap();
        file.write_all(&stored_zip(entries)).unwrap();
        inspect_package(path).unwrap()
    }

    #[test]
    fn identifies_apk_structure_and_sorted_native_abis() {
        let report = inspect_fixture(
            "game.apk",
            &[
                ("AndroidManifest.xml", false),
                ("classes.dex", false),
                ("lib/arm64-v8a/libgame.so", false),
                ("lib/x86_64/libgame.so", false),
            ],
        );

        assert_eq!(report.format, PackageFormat::Apk);
        assert!(report.has_android_manifest);
        assert!(report.has_dex);
        assert_eq!(report.native_abis, ["arm64-v8a", "x86_64"]);
        assert!(report.is_installable_single_apk());
    }

    #[test]
    fn identifies_xapk_and_embedded_game_data() {
        let report = inspect_fixture(
            "game.xapk",
            &[
                ("manifest.json", false),
                ("base.apk", false),
                ("config.arm64_v8a.apk", false),
                ("Android/obb/com.example/main.1.com.example.obb", false),
            ],
        );

        assert_eq!(report.format, PackageFormat::Xapk);
        assert_eq!(report.embedded_apks.len(), 2);
        assert_eq!(report.obb_files.len(), 1);
        assert!(!report.is_installable_single_apk());
    }

    #[test]
    fn encrypted_apk_is_not_installable() {
        let report = inspect_fixture(
            "encrypted.apk",
            &[("AndroidManifest.xml", true), ("classes.dex", false)],
        );

        assert_eq!(report.encrypted_entries, 1);
        assert!(!report.is_installable_single_apk());
    }

    #[test]
    fn rejects_non_zip_and_truncated_archives() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bad.apk");
        fs::write(&path, b"not an apk").unwrap();
        assert!(matches!(
            inspect_package(&path),
            Err(PackageInspectionError::InvalidArchive(_))
        ));

        fs::write(&path, &stored_zip(&[("AndroidManifest.xml", false)])[..30]).unwrap();
        assert!(inspect_package(path).is_err());
    }

    #[test]
    fn eocd_search_ignores_signature_bytes_inside_the_comment() {
        let mut archive = stored_zip(&[("AndroidManifest.xml", false)]);
        archive.truncate(archive.len() - 2);
        let comment = EOCD_SIGNATURE.to_le_bytes();
        archive.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        archive.extend_from_slice(&comment);

        assert!(find_eocd(&archive).is_some());
    }

    #[test]
    fn classifies_native_and_translation_compatibility() {
        assert_eq!(
            assess_abi_compatibility(&[], &["x86_64".to_owned()], Some(false)),
            AbiCompatibility::Universal
        );
        assert_eq!(
            assess_abi_compatibility(&["x86_64".to_owned()], &["x86_64".to_owned()], Some(false)),
            AbiCompatibility::Native
        );
        assert_eq!(
            assess_abi_compatibility(&["arm64-v8a".to_owned()], &[], Some(true)),
            AbiCompatibility::NativeTranslation
        );
        assert_eq!(
            assess_abi_compatibility(&["arm64-v8a".to_owned()], &[], Some(false)),
            AbiCompatibility::ArmTranslationMissing
        );
        assert_eq!(
            assess_abi_compatibility(&["x86".to_owned()], &["arm64-v8a".to_owned()], Some(false)),
            AbiCompatibility::Incompatible
        );
    }
}
