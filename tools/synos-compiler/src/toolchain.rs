use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use synos_pkg::{BundleInfo, PackageBundle, PackageError, SigningKey, bundle_size, encode_bundle};
use synos_system_model::ContentId;

use super::{Target, write_atomic};

const ARCHIVE_MAGIC: &[u8; 8] = b"SYNTOOL1";
const ARCHIVE_VERSION: u16 = 1;
const ARCHIVE_HEADER_BYTES: usize = 22;
const ARCHIVE_RECORD_BYTES: usize = 45;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ToolchainStage {
    Stage0 = 0,
    Stage1 = 1,
    Stage2 = 2,
}

impl ToolchainStage {
    pub fn parse(value: &str) -> Result<Self, ToolchainError> {
        match value {
            "0" | "stage-0" => Ok(Self::Stage0),
            "1" | "stage-1" => Ok(Self::Stage1),
            "2" | "stage-2" => Ok(Self::Stage2),
            _ => Err(ToolchainError::InvalidStage),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ToolchainAssetKind {
    Cargo = 1,
    Rustc = 2,
    Rustdoc = 3,
    Linker = 4,
    Codegen = 5,
    Sysroot = 6,
    TargetLibraries = 7,
    Sources = 8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolchainAsset {
    pub kind: ToolchainAssetKind,
    pub path: String,
    pub content: ContentId,
    pub bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolchainManifest {
    pub stage: ToolchainStage,
    pub target: Target,
    pub rust_version: String,
    pub assets: Vec<ToolchainAsset>,
    pub content: ContentId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolchainPackageOutput {
    pub bundle: PathBuf,
    pub info: BundleInfo,
    pub manifest: ToolchainManifest,
}

pub struct ToolchainManager {
    root: PathBuf,
}

impl ToolchainManager {
    pub fn new(root: PathBuf) -> Result<Self, ToolchainError> {
        if root.as_os_str().is_empty() || root == Path::new("/") {
            return Err(ToolchainError::PermissionDenied(root));
        }
        Ok(Self { root })
    }

    pub fn install(
        &self,
        bundle: &Path,
        key: SigningKey,
        stage: ToolchainStage,
        target: Option<Target>,
    ) -> Result<ToolchainManifest, ToolchainError> {
        let bytes = fs::read(bundle).map_err(|error| ToolchainError::Io {
            path: bundle.to_path_buf(),
            error: error.to_string(),
        })?;
        let manifest = verify_bundle(&bytes, key)?;
        if manifest.stage != stage || target.is_some_and(|target| target != manifest.target) {
            return Err(ToolchainError::InvalidTarget);
        }
        fs::create_dir_all(&self.root).map_err(|error| ToolchainError::Io {
            path: self.root.clone(),
            error: error.to_string(),
        })?;
        let current = self.stage_path(stage);
        let previous = self.root.join("previous.synpkg");
        if current.is_file() {
            fs::copy(&current, &previous).map_err(|error| ToolchainError::Io {
                path: previous.clone(),
                error: error.to_string(),
            })?;
        }
        write_manager_file(&current, &bytes)?;
        write_manager_file(&self.root.join("active-stage"), stage_name(stage).as_bytes())?;
        Ok(manifest)
    }

    pub fn select(&self, stage: ToolchainStage) -> Result<(), ToolchainError> {
        let path = self.stage_path(stage);
        if !path.is_file() {
            return Err(ToolchainError::ToolchainNotInstalled(stage));
        }
        write_manager_file(&self.root.join("active-stage"), stage_name(stage).as_bytes())
    }

    pub fn rollback(&self) -> Result<ToolchainStage, ToolchainError> {
        let active = fs::read_to_string(self.root.join("active-stage"))
            .map_err(|_| ToolchainError::NoPreviousToolchain)?;
        let stage = ToolchainStage::parse(active.trim())?;
        let previous = self.root.join("previous.synpkg");
        if !previous.is_file() {
            return Err(ToolchainError::NoPreviousToolchain);
        }
        let current = self.stage_path(stage);
        let rollback = self.root.join("rollback.synpkg");
        if current.is_file() {
            fs::rename(&current, &rollback).map_err(|error| ToolchainError::Io {
                path: rollback.clone(),
                error: error.to_string(),
            })?;
        }
        fs::rename(&previous, &current).map_err(|error| ToolchainError::Io {
            path: current.clone(),
            error: error.to_string(),
        })?;
        if rollback.is_file() {
            fs::rename(&rollback, &self.root.join("previous.synpkg")).map_err(|error| {
                ToolchainError::Io {
                    path: self.root.join("previous.synpkg"),
                    error: error.to_string(),
                }
            })?;
        }
        Ok(stage)
    }

    fn stage_path(&self, stage: ToolchainStage) -> PathBuf {
        self.root.join(format!("{}.synpkg", stage_name(stage)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolchainError {
    InvalidStage,
    InvalidTarget,
    MissingTool { name: String, path: PathBuf },
    MissingSysroot(PathBuf),
    MissingTargetLibraries(PathBuf),
    MissingSources(PathBuf),
    Io { path: PathBuf, error: String },
    InvalidArchive,
    InvalidArchivePath(String),
    ArchiveTooLarge,
    Package(PackageError),
    PackageWrite { path: PathBuf, error: String },
    PermissionDenied(PathBuf),
    ToolchainNotInstalled(ToolchainStage),
    NoPreviousToolchain,
}

impl std::fmt::Display for ToolchainError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidStage => write!(formatter, "toolchain stage must be 0, 1, or 2"),
            Self::InvalidTarget => write!(formatter, "toolchain target is invalid"),
            Self::MissingTool { name, path } => {
                write!(formatter, "missing {name} executable at {}", path.display())
            }
            Self::MissingSysroot(path) => write!(formatter, "missing Rust sysroot: {}", path.display()),
            Self::MissingTargetLibraries(path) => {
                write!(formatter, "missing target libraries: {}", path.display())
            }
            Self::MissingSources(path) => write!(formatter, "missing rust-src: {}", path.display()),
            Self::Io { path, error } => write!(formatter, "{}: {error}", path.display()),
            Self::InvalidArchive => write!(formatter, "invalid SynOS toolchain archive"),
            Self::InvalidArchivePath(path) => write!(formatter, "invalid archive path `{path}`"),
            Self::ArchiveTooLarge => write!(formatter, "toolchain archive is too large"),
            Self::Package(error) => write!(formatter, "package error: {error:?}"),
            Self::PackageWrite { path, error } => {
                write!(formatter, "could not write {}: {error}", path.display())
            }
            Self::PermissionDenied(path) => {
                write!(formatter, "toolchain root is not an allowed capability path: {}", path.display())
            }
            Self::ToolchainNotInstalled(stage) => {
                write!(formatter, "toolchain {} is not installed", stage_name(*stage))
            }
            Self::NoPreviousToolchain => write!(formatter, "no previous toolchain is available"),
        }
    }
}

impl std::error::Error for ToolchainError {}

impl From<PackageError> for ToolchainError {
    fn from(error: PackageError) -> Self {
        Self::Package(error)
    }
}

#[derive(Clone, Debug)]
pub struct HostToolchain {
    pub stage: ToolchainStage,
    pub target: Target,
    pub rustc: PathBuf,
    pub rustdoc: PathBuf,
    pub cargo: PathBuf,
    pub rust_lld: PathBuf,
    pub sysroot: PathBuf,
    pub target_libdir: PathBuf,
    pub rust_src: PathBuf,
    pub rust_version: String,
}

impl HostToolchain {
    pub fn discover(stage: ToolchainStage, target: Target) -> Result<Self, ToolchainError> {
        let rustc = find_executable("rustc", env::var_os("RUSTC"))?;
        let rustdoc = find_executable("rustdoc", env::var_os("RUSTDOC"))?;
        let cargo = find_executable("cargo", env::var_os("CARGO"))?;
        let sysroot = command_path(&rustc, &["--print", "sysroot"])?;
        let target_libdir = command_path(
            &rustc,
            &["--print", "target-libdir", "--target", target.rust_target()],
        )?;
        let rust_src = sysroot.join("lib/rustlib/src/rust");
        if !rust_src.is_dir() {
            return Err(ToolchainError::MissingSources(rust_src));
        }
        if !target_libdir.is_dir() {
            return Err(ToolchainError::MissingTargetLibraries(target_libdir));
        }
        let rust_lld = find_linker(&sysroot, &rustc)?;
        let rust_version = command_text(&rustc, &["-vV"])?;
        Ok(Self {
            stage,
            target,
            rustc,
            rustdoc,
            cargo,
            rust_lld,
            sysroot,
            target_libdir,
            rust_src,
            rust_version,
        })
    }

    pub fn package(&self, output: &Path, key: SigningKey) -> Result<ToolchainPackageOutput, ToolchainError> {
        let mut files = Vec::new();
        add_file(&mut files, ToolchainAssetKind::Rustc, &self.rustc, Path::new("bin/rustc"))?;
        add_file(&mut files, ToolchainAssetKind::Rustdoc, &self.rustdoc, Path::new("bin/rustdoc"))?;
        add_file(&mut files, ToolchainAssetKind::Cargo, &self.cargo, Path::new("bin/cargo"))?;
        add_file(&mut files, ToolchainAssetKind::Linker, &self.rust_lld, Path::new("bin/rust-lld"))?;
        add_tree(&mut files, ToolchainAssetKind::Sysroot, &self.sysroot, Path::new("sysroot"))?;
        add_tree(
            &mut files,
            ToolchainAssetKind::TargetLibraries,
            &self.target_libdir,
            Path::new("target-libraries"),
        )?;
        add_tree(&mut files, ToolchainAssetKind::Sources, &self.rust_src, Path::new("rust-src"))?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        files.dedup_by(|left, right| left.path == right.path);

        let manifest = manifest_for(self, &files);
        let archive = encode_archive(self, &files)?;
        let required = bundle_size(archive.len(), 0)?;
        let mut encoded = vec![0; required];
        let info = encode_bundle(&archive, 0, &[], key, &mut encoded)?;
        write_atomic(output, &encoded).map_err(|error| ToolchainError::PackageWrite {
            path: output.to_path_buf(),
            error: error.to_string(),
        })?;
        Ok(ToolchainPackageOutput {
            bundle: output.to_path_buf(),
            info,
            manifest,
        })
    }

    pub fn from_root(
        root: &Path,
        stage: ToolchainStage,
        target: Target,
        rust_version: String,
    ) -> Result<Self, ToolchainError> {
        let bin = root.join("bin");
        let sysroot = root.join("sysroot");
        let target_libdir = root.join("target-libraries");
        let rust_src = root.join("rust-src");
        let rustc = bin.join("rustc");
        let rustdoc = bin.join("rustdoc");
        let cargo = bin.join("cargo");
        let rust_lld = bin.join("rust-lld");
        for (name, path) in [
            ("rustc", &rustc),
            ("rustdoc", &rustdoc),
            ("cargo", &cargo),
            ("rust-lld", &rust_lld),
        ] {
            if !path.is_file() {
                return Err(ToolchainError::MissingTool { name: name.into(), path: path.clone() });
            }
        }
        if !sysroot.is_dir() {
            return Err(ToolchainError::MissingSysroot(sysroot));
        }
        if !target_libdir.is_dir() {
            return Err(ToolchainError::MissingTargetLibraries(target_libdir));
        }
        if !rust_src.is_dir() {
            return Err(ToolchainError::MissingSources(rust_src));
        }
        Ok(Self {
            stage,
            target,
            rustc,
            rustdoc,
            cargo,
            rust_lld,
            sysroot,
            target_libdir,
            rust_src,
            rust_version,
        })
    }
}

pub fn verify_bundle(bytes: &[u8], key: SigningKey) -> Result<ToolchainManifest, ToolchainError> {
    let bundle = PackageBundle::decode(bytes)?;
    bundle.verify(key)?;
    let (manifest, _) = decode_archive(bundle.payload())?;
    Ok(manifest)
}

fn manifest_for(toolchain: &HostToolchain, files: &[ArchiveFile]) -> ToolchainManifest {
    let assets = files
        .iter()
        .map(|file| ToolchainAsset {
            kind: file.kind,
            path: file.path.to_string_lossy().into_owned(),
            content: file.content,
            bytes: file.data.len() as u64,
        })
        .collect::<Vec<_>>();
    let content = manifest_content(toolchain.stage, toolchain.target, &toolchain.rust_version, &assets);
    ToolchainManifest {
        stage: toolchain.stage,
        target: toolchain.target,
        rust_version: toolchain.rust_version.clone(),
        assets,
        content,
    }
}

struct ArchiveFile {
    kind: ToolchainAssetKind,
    path: PathBuf,
    data: Vec<u8>,
    content: ContentId,
}

fn encode_archive(toolchain: &HostToolchain, files: &[ArchiveFile]) -> Result<Vec<u8>, ToolchainError> {
    let version = toolchain.rust_version.as_bytes();
    if version.len() > u16::MAX as usize || files.len() > u32::MAX as usize {
        return Err(ToolchainError::ArchiveTooLarge);
    }
    let mut archive = Vec::new();
    archive.extend_from_slice(ARCHIVE_MAGIC);
    archive.extend_from_slice(&ARCHIVE_VERSION.to_be_bytes());
    archive.push(toolchain.stage as u8);
    archive.push(toolchain.target as u8);
    archive.extend_from_slice(&(version.len() as u16).to_be_bytes());
    archive.extend_from_slice(&(files.len() as u32).to_be_bytes());
    archive.extend_from_slice(&0u32.to_be_bytes());
    archive.extend_from_slice(version);
    for file in files {
        let path = file.path.to_string_lossy();
        if !valid_archive_path(&path) || path.len() > u32::MAX as usize {
            return Err(ToolchainError::InvalidArchivePath(path.into_owned()));
        }
        archive.push(file.kind as u8);
        archive.extend_from_slice(&(path.len() as u32).to_be_bytes());
        archive.extend_from_slice(&(file.data.len() as u64).to_be_bytes());
        archive.extend_from_slice(file.content.as_bytes());
        archive.extend_from_slice(path.as_bytes());
        archive.extend_from_slice(&file.data);
    }
    Ok(archive)
}

fn decode_archive(bytes: &[u8]) -> Result<(ToolchainManifest, usize), ToolchainError> {
    if bytes.len() < ARCHIVE_HEADER_BYTES
        || &bytes[..8] != ARCHIVE_MAGIC
        || u16::from_be_bytes([bytes[8], bytes[9]]) != ARCHIVE_VERSION
        || bytes[18..22].iter().any(|byte| *byte != 0)
    {
        return Err(ToolchainError::InvalidArchive);
    }
    let stage = match bytes[10] {
        0 => ToolchainStage::Stage0,
        1 => ToolchainStage::Stage1,
        2 => ToolchainStage::Stage2,
        _ => return Err(ToolchainError::InvalidArchive),
    };
    let target = match bytes[11] {
        0 => Target::X86_64,
        1 => Target::Aarch64,
        _ => return Err(ToolchainError::InvalidArchive),
    };
    let version_length = u16::from_be_bytes([bytes[12], bytes[13]]) as usize;
    let count = u32::from_be_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]) as usize;
    let version_start = ARCHIVE_HEADER_BYTES;
    let version_end = version_start.checked_add(version_length).ok_or(ToolchainError::InvalidArchive)?;
    let version = String::from_utf8(bytes.get(version_start..version_end).ok_or(ToolchainError::InvalidArchive)?.to_vec())
        .map_err(|_| ToolchainError::InvalidArchive)?;
    let mut cursor = version_end;
    let mut assets = Vec::with_capacity(count);
    for _ in 0..count {
        if bytes.len().saturating_sub(cursor) < ARCHIVE_RECORD_BYTES {
            return Err(ToolchainError::InvalidArchive);
        }
        let kind = asset_kind(bytes[cursor]).ok_or(ToolchainError::InvalidArchive)?;
        let path_length = u32::from_be_bytes([
            bytes[cursor + 1], bytes[cursor + 2], bytes[cursor + 3], bytes[cursor + 4],
        ]) as usize;
        let data_length = u64::from_be_bytes([
            bytes[cursor + 5], bytes[cursor + 6], bytes[cursor + 7], bytes[cursor + 8],
            bytes[cursor + 9], bytes[cursor + 10], bytes[cursor + 11], bytes[cursor + 12],
        ]);
        let mut content = [0; 32];
        content.copy_from_slice(&bytes[cursor + 13..cursor + 45]);
        cursor += ARCHIVE_RECORD_BYTES;
        let path_end = cursor.checked_add(path_length).ok_or(ToolchainError::InvalidArchive)?;
        let path = String::from_utf8(bytes.get(cursor..path_end).ok_or(ToolchainError::InvalidArchive)?.to_vec())
            .map_err(|_| ToolchainError::InvalidArchive)?;
        if !valid_archive_path(&path) {
            return Err(ToolchainError::InvalidArchivePath(path));
        }
        cursor = path_end;
        let data_end = cursor.checked_add(usize::try_from(data_length).map_err(|_| ToolchainError::InvalidArchive)?)
            .ok_or(ToolchainError::InvalidArchive)?;
        let data = bytes.get(cursor..data_end).ok_or(ToolchainError::InvalidArchive)?;
        if ContentId::hash(data) != ContentId::from_bytes(content) {
            return Err(ToolchainError::InvalidArchive);
        }
        assets.push(ToolchainAsset {
            kind,
            path,
            content: ContentId::from_bytes(content),
            bytes: data_length,
        });
        cursor = data_end;
    }
    if cursor != bytes.len() {
        return Err(ToolchainError::InvalidArchive);
    }
    let content = manifest_content(stage, target, &version, &assets);
    Ok((ToolchainManifest {
        stage,
        target,
        rust_version: version,
        assets,
        content,
    }, cursor))
}

fn manifest_content(
    stage: ToolchainStage,
    target: Target,
    rust_version: &str,
    assets: &[ToolchainAsset],
) -> ContentId {
    let mut material = Vec::new();
    material.push(stage as u8);
    material.push(target as u8);
    material.extend_from_slice(rust_version.as_bytes());
    for asset in assets {
        material.push(asset.kind as u8);
        material.extend_from_slice(asset.path.as_bytes());
        material.extend_from_slice(asset.content.as_bytes());
    }
    ContentId::hash(&material)
}

fn add_file(files: &mut Vec<ArchiveFile>, kind: ToolchainAssetKind, source: &Path, name: &Path) -> Result<(), ToolchainError> {
    let data = fs::read(source).map_err(|error| ToolchainError::Io { path: source.to_path_buf(), error: error.to_string() })?;
    files.push(ArchiveFile { kind, path: name.to_path_buf(), content: ContentId::hash(&data), data });
    Ok(())
}

fn add_tree(files: &mut Vec<ArchiveFile>, kind: ToolchainAssetKind, source: &Path, prefix: &Path) -> Result<(), ToolchainError> {
    if !source.is_dir() {
        return Err(ToolchainError::Io { path: source.to_path_buf(), error: "directory does not exist".into() });
    }
    let mut entries = fs::read_dir(source)
        .map_err(|error| ToolchainError::Io { path: source.to_path_buf(), error: error.to_string() })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ToolchainError::Io { path: source.to_path_buf(), error: error.to_string() })?;
    entries.sort();
    for path in entries {
        let relative = path.strip_prefix(source).map_err(|_| ToolchainError::InvalidArchive)?;
        let file_type = fs::symlink_metadata(&path)
            .map_err(|error| ToolchainError::Io { path: path.clone(), error: error.to_string() })?
            .file_type();
        if file_type.is_symlink() {
            return Err(ToolchainError::Io { path, error: "symbolic links are not allowed".into() });
        }
        let destination = prefix.join(relative);
        if file_type.is_dir() {
            add_tree(files, kind, &path, &destination)?;
        } else if file_type.is_file() {
            add_file(files, kind, &path, &destination)?;
        }
    }
    Ok(())
}

fn asset_kind(raw: u8) -> Option<ToolchainAssetKind> {
    Some(match raw {
        1 => ToolchainAssetKind::Cargo,
        2 => ToolchainAssetKind::Rustc,
        3 => ToolchainAssetKind::Rustdoc,
        4 => ToolchainAssetKind::Linker,
        5 => ToolchainAssetKind::Codegen,
        6 => ToolchainAssetKind::Sysroot,
        7 => ToolchainAssetKind::TargetLibraries,
        8 => ToolchainAssetKind::Sources,
        _ => return None,
    })
}

fn valid_archive_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
}

fn stage_name(stage: ToolchainStage) -> &'static str {
    match stage {
        ToolchainStage::Stage0 => "stage-0",
        ToolchainStage::Stage1 => "stage-1",
        ToolchainStage::Stage2 => "stage-2",
    }
}

fn write_manager_file(path: &Path, bytes: &[u8]) -> Result<(), ToolchainError> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, bytes).map_err(|error| ToolchainError::Io {
        path: temporary.clone(),
        error: error.to_string(),
    })?;
    fs::rename(&temporary, path).map_err(|error| ToolchainError::Io {
        path: path.to_path_buf(),
        error: error.to_string(),
    })
}

fn find_executable(name: &str, configured: Option<std::ffi::OsString>) -> Result<PathBuf, ToolchainError> {
    let configured = configured.map(PathBuf::from).unwrap_or_else(|| PathBuf::from(name));
    if configured.is_file() {
        return Ok(configured);
    }
    if configured.components().count() > 1 {
        return Err(ToolchainError::MissingTool { name: name.into(), path: configured });
    }
    let path_directories = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();
    for directory in path_directories {
        let candidate = directory.join(&configured);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(ToolchainError::MissingTool { name: name.into(), path: configured })
}

fn find_linker(sysroot: &Path, rustc: &Path) -> Result<PathBuf, ToolchainError> {
    let candidates = [
        sysroot.join("bin/rust-lld"),
        sysroot.join("lib/rustlib/bin/rust-lld"),
        rustc.parent().unwrap_or_else(|| Path::new(".")).join("rust-lld"),
    ];
    candidates.into_iter().find(|path| path.is_file()).ok_or_else(|| ToolchainError::MissingTool {
        name: "rust-lld".into(),
        path: sysroot.join("bin/rust-lld"),
    })
}

fn command_path(program: &Path, arguments: &[&str]) -> Result<PathBuf, ToolchainError> {
    let output = std::process::Command::new(program).args(arguments).output()
        .map_err(|error| ToolchainError::Io { path: program.to_path_buf(), error: error.to_string() })?;
    if !output.status.success() {
        return Err(ToolchainError::Io { path: program.to_path_buf(), error: String::from_utf8_lossy(&output.stderr).trim().into() });
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_string()))
}

fn command_text(program: &Path, arguments: &[&str]) -> Result<String, ToolchainError> {
    let output = std::process::Command::new(program).args(arguments).output()
        .map_err(|error| ToolchainError::Io { path: program.to_path_buf(), error: error.to_string() })?;
    if !output.status.success() {
        return Err(ToolchainError::Io { path: program.to_path_buf(), error: String::from_utf8_lossy(&output.stderr).trim().into() });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
