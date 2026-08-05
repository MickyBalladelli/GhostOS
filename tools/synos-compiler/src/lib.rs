use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use synos_pkg::{BundleInfo, PackageError, SigningKey, bundle_size, encode_bundle};
use synos_synfs::{DirectoryEntry, Error as SynFsError, FileType, SynFs};
use synos_system_model::ContentId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Target {
    X86_64,
    Aarch64,
}

impl Target {
    pub const fn target_file(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-synos.json",
            Self::Aarch64 => "aarch64-unknown-synos.json",
        }
    }

    pub fn parse(value: &str) -> Result<Self, CompileError> {
        match value {
            "x86_64" | "x86_64-unknown-synos" => Ok(Self::X86_64),
            "aarch64" | "aarch64-unknown-synos" => Ok(Self::Aarch64),
            other => Err(CompileError::UnsupportedTarget(other.to_string())),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CompileRequest {
    pub manifest_path: PathBuf,
    pub binary: String,
    pub package: Option<String>,
    pub target: Target,
    pub release: bool,
    pub target_directory: Option<PathBuf>,
    pub locked: bool,
    pub offline: bool,
}

#[derive(Clone, Debug)]
pub struct SynFsCompileRequest {
    pub source_root: String,
    pub manifest: String,
    pub binary: String,
    pub package: Option<String>,
    pub target: Target,
    pub release: bool,
    pub locked: bool,
    pub offline: bool,
    pub target_directory: Option<PathBuf>,
    pub max_source_bytes: u64,
}

impl SynFsCompileRequest {
    pub fn new(source_root: &str, manifest: &str, binary: &str, target: Target) -> Self {
        Self {
            source_root: source_root.into(),
            manifest: manifest.into(),
            binary: binary.into(),
            package: None,
            target,
            release: false,
            locked: true,
            offline: true,
            target_directory: None,
            max_source_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileOutput {
    pub artifact: PathBuf,
    pub target: Target,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleOutput {
    pub bundle: PathBuf,
    pub artifact: PathBuf,
    pub info: BundleInfo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceOutput {
    pub target: Target,
}

#[derive(Debug)]
pub enum CompileError {
    CargoUnavailable(String),
    InvalidManifest(PathBuf),
    UnsupportedTarget(String),
    BuildFailed(ExitStatus),
    InvalidTargetDirectory,
    InvalidSourceRoot(PathBuf),
    InvalidSynFsSourceRoot(String),
    InvalidSynFsManifest(String),
    SourceTooLarge { limit: u64 },
    UnsupportedSynFsFile(String),
    SynFs(SynFsError),
    Staging { path: PathBuf, error: String },
    Package(PackageError),
    PackageWrite { path: PathBuf, error: String },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CargoUnavailable(error) => write!(formatter, "could not start Cargo: {error}"),
            Self::InvalidManifest(path) => write!(formatter, "manifest does not exist: {}", path.display()),
            Self::UnsupportedTarget(target) => write!(formatter, "unsupported SynOS target `{target}`"),
            Self::BuildFailed(status) => write!(formatter, "SynOS compilation failed with {status}"),
            Self::InvalidTargetDirectory => write!(formatter, "could not locate the workspace root"),
            Self::InvalidSourceRoot(path) => {
                write!(formatter, "source root does not exist: {}", path.display())
            }
            Self::InvalidSynFsSourceRoot(path) => {
                write!(formatter, "SynFS source root is invalid: {path}")
            }
            Self::InvalidSynFsManifest(path) => {
                write!(formatter, "SynFS manifest is invalid: {path}")
            }
            Self::SourceTooLarge { limit } => {
                write!(formatter, "SynFS source exceeds the {limit}-byte limit")
            }
            Self::UnsupportedSynFsFile(path) => {
                write!(formatter, "unsupported SynFS source file: {path}")
            }
            Self::SynFs(error) => write!(formatter, "could not read SynFS source: {error:?}"),
            Self::Staging { path, error } => {
                write!(formatter, "could not stage SynFS source at {}: {error}", path.display())
            }
            Self::Package(error) => write!(formatter, "could not create package: {error:?}"),
            Self::PackageWrite { path, error } => {
                write!(formatter, "could not write package {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for CompileError {}

impl From<PackageError> for CompileError {
    fn from(error: PackageError) -> Self {
        Self::Package(error)
    }
}

impl From<SynFsError> for CompileError {
    fn from(error: SynFsError) -> Self {
        Self::SynFs(error)
    }
}

pub struct Compiler {
    cargo: OsString,
    workspace_root: PathBuf,
}

impl Compiler {
    pub fn new() -> Result<Self, CompileError> {
        let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or(CompileError::InvalidTargetDirectory)?;
        Ok(Self {
            cargo: env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")),
            workspace_root,
        })
    }

    pub fn compile(&self, request: &CompileRequest) -> Result<CompileOutput, CompileError> {
        if !request.manifest_path.is_file() {
            return Err(CompileError::InvalidManifest(request.manifest_path.clone()))
        }
        let source_root = request
            .manifest_path
            .parent()
            .ok_or_else(|| CompileError::InvalidSourceRoot(request.manifest_path.clone()))?;
        if !source_root.is_dir() {
            return Err(CompileError::InvalidSourceRoot(source_root.to_path_buf()))
        }
        let target_file = self.workspace_root.join("targets").join(request.target.target_file());
        let mut command = self.base_command();
        command
            .arg("build")
            .arg("--manifest-path")
            .arg(&request.manifest_path)
            .arg("--bin")
            .arg(&request.binary)
            .arg("--target")
            .arg(&target_file);
        if let Some(package) = &request.package {
            command.arg("--package").arg(package);
        }
        if request.release {
            command.arg("--release");
        }
        if let Some(target_directory) = &request.target_directory {
            command.arg("--target-dir").arg(target_directory);
        }
        if request.locked {
            command.arg("--locked");
        }
        if request.offline {
            command.arg("--offline");
        }
        let status = command
            .status()
            .map_err(|error| CompileError::CargoUnavailable(error.to_string()))?;
        if !status.success() {
            return Err(CompileError::BuildFailed(status))
        }
        let profile = if request.release { "release" } else { "debug" };
        let target_directory = request
            .target_directory
            .clone()
            .or_else(|| env::var_os("CARGO_TARGET_DIR").map(PathBuf::from))
            .unwrap_or_else(|| self.workspace_root.join("target"));
        Ok(CompileOutput {
            artifact: target_directory
                .join(request.target.target_file().trim_end_matches(".json"))
                .join(profile)
                .join(&request.binary),
            target: request.target,
        })
    }

    /// Copy one immutable SynFS project generation into a private host
    /// workspace, then run the normal locked Cargo build against that copy.
    /// The staged directory is retained so the caller can inspect artifacts
    /// and diagnostics after the build.
    pub fn compile_synfs<const MAX_BLOCKS: usize>(
        &self,
        filesystem: &SynFs<MAX_BLOCKS>,
        request: &SynFsCompileRequest,
        staging_root: &Path,
    ) -> Result<CompileOutput, CompileError> {
        let project_root = stage_synfs_project(filesystem, request, staging_root)?;
        let manifest = synfs_manifest_path(&request.source_root, &request.manifest)?;
        self.compile(&CompileRequest {
            manifest_path: project_root.join(relative_synfs_path(&request.source_root, &manifest)?),
            binary: request.binary.clone(),
            package: request.package.clone(),
            target: request.target,
            release: request.release,
            target_directory: request.target_directory.clone(),
            locked: request.locked,
            offline: request.offline,
        })
    }

    pub fn compile_and_bundle(
        &self,
        request: &CompileRequest,
        key: SigningKey,
        output: &Path,
        entry_offset: u64,
        dependencies: &[ContentId],
    ) -> Result<BundleOutput, CompileError> {
        let compiled = self.compile(request)?;
        let payload = fs::read(&compiled.artifact).map_err(|error| {
            CompileError::PackageWrite {
                path: compiled.artifact.clone(),
                error: error.to_string(),
            }
        })?;
        let required = bundle_size(payload.len(), dependencies.len())?;
        let mut encoded = vec![0; required];
        let info = encode_bundle(
            &payload,
            entry_offset,
            dependencies,
            key,
            &mut encoded,
        )?;
        write_atomic(output, &encoded)?;
        Ok(BundleOutput {
            bundle: output.to_path_buf(),
            artifact: compiled.artifact,
            info,
        })
    }

    pub fn compile_workspace(
        &self,
        target: Target,
        release: bool,
        target_directory: Option<&Path>,
    ) -> Result<WorkspaceOutput, CompileError> {
        let target_file = self.workspace_root.join("targets").join(target.target_file());
        let mut command = self.base_command();
        command
            .arg("build")
            .arg("--workspace")
            .arg("--lib")
            .args([
                "--exclude",
                "cargo-synos",
                "--exclude",
                "synos-compiler",
                "--exclude",
                "synos-test-support",
                "--exclude",
                "synos-uefi",
                "--exclude",
                "synos-vm",
            ])
            .arg("--target")
            .arg(target_file);
        if release {
            command.arg("--release");
        }
        if let Some(target_directory) = target_directory {
            command.arg("--target-dir").arg(target_directory);
        }
        let status = command
            .status()
            .map_err(|error| CompileError::CargoUnavailable(error.to_string()))?;
        if !status.success() {
            return Err(CompileError::BuildFailed(status))
        }
        Ok(WorkspaceOutput { target })
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(&self.cargo);
        command
            .env("RUSTC_BOOTSTRAP", "1")
            .args([
                "-Z",
                "build-std=core,alloc",
                "-Z",
                "json-target-spec",
            ]);
        if let Some(linker) = bundled_linker() {
            let mut rustflags = env::var("RUSTFLAGS").unwrap_or_default();
            if !rustflags.is_empty() {
                rustflags.push(' ')
            }
            rustflags.push_str("-C linker=");
            rustflags.push_str(&linker.to_string_lossy());
            command.env("RUSTFLAGS", rustflags);
        }
        command
    }

    pub fn run_host(
        &self,
        manifest_path: &Path,
        binary: &str,
        release: bool,
        arguments: &[String],
    ) -> Result<ExitStatus, CompileError> {
        if !manifest_path.is_file() {
            return Err(CompileError::InvalidManifest(manifest_path.to_path_buf()))
        }
        let mut command = Command::new(&self.cargo);
        command
            .arg("run")
            .arg("--manifest-path")
            .arg(manifest_path)
            .arg("--bin")
            .arg(binary);
        if release {
            command.arg("--release");
        }
        if !arguments.is_empty() {
            command.arg("--").args(arguments);
        }
        command
            .status()
            .map_err(|error| CompileError::CargoUnavailable(error.to_string()))
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CompileError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(CompileError::PackageWrite {
            path: path.to_path_buf(),
            error: "parent directory does not exist".into(),
        })
    }
    let temporary = path.with_extension("synpkg.tmp");
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    result.map_err(|error| CompileError::PackageWrite {
        path: path.to_path_buf(),
        error: error.to_string(),
    })
}

fn stage_synfs_project<const MAX_BLOCKS: usize>(
    filesystem: &SynFs<MAX_BLOCKS>,
    request: &SynFsCompileRequest,
    staging_root: &Path,
) -> Result<PathBuf, CompileError> {
    let source_root = validate_synfs_root(&request.source_root)?;
    let manifest = synfs_manifest_path(source_root, &request.manifest)?;
    let root = filesystem.lookup(source_root)?;
    if root.file_type != FileType::Directory {
        return Err(CompileError::InvalidSynFsSourceRoot(source_root.into()));
    }
    let manifest_info = filesystem.lookup(&manifest)?;
    if manifest_info.file_type != FileType::Regular {
        return Err(CompileError::InvalidSynFsManifest(manifest));
    }

    let identity = format!("{source_root}:{manifest}:{}", filesystem.generation());
    let project_root = staging_root.join(format!("synfs-{:?}", ContentId::hash(identity.as_bytes())));
    fs::create_dir_all(&project_root).map_err(|error| CompileError::Staging {
        path: project_root.clone(),
        error: error.to_string(),
    })?;

    let mut pending = vec![source_root.to_string()];
    let mut total_bytes = 0_u64;
    while let Some(directory) = pending.pop() {
        let mut entries = [DirectoryEntry::EMPTY; 256];
        let count = filesystem.list_directory(&directory, &mut entries)?;
        for entry in entries.into_iter().take(count) {
            let source = join_synfs_path(&directory, entry.name.as_str());
            let relative = relative_synfs_path(source_root, &source)?;
            let destination = project_root.join(&relative);
            match entry.file_type {
                FileType::Directory => {
                    fs::create_dir_all(&destination).map_err(|error| CompileError::Staging {
                        path: destination.clone(),
                        error: error.to_string(),
                    })?;
                    pending.push(source);
                }
                FileType::Regular => {
                    total_bytes = total_bytes
                        .checked_add(entry.size)
                        .ok_or(CompileError::SourceTooLarge {
                            limit: request.max_source_bytes,
                        })?;
                    if total_bytes > request.max_source_bytes {
                        return Err(CompileError::SourceTooLarge {
                            limit: request.max_source_bytes,
                        });
                    }
                    let size = usize::try_from(entry.size).map_err(|_| CompileError::SourceTooLarge {
                        limit: request.max_source_bytes,
                    })?;
                    let mut bytes = vec![0; size];
                    let read = filesystem.read(&source, &mut bytes)?;
                    bytes.truncate(read.bytes_read);
                    if let Some(parent) = destination.parent() {
                        fs::create_dir_all(parent).map_err(|error| CompileError::Staging {
                            path: parent.to_path_buf(),
                            error: error.to_string(),
                        })?;
                    }
                    fs::write(&destination, bytes).map_err(|error| CompileError::Staging {
                        path: destination,
                        error: error.to_string(),
                    })?;
                }
                FileType::Symlink => {
                    return Err(CompileError::UnsupportedSynFsFile(source));
                }
            }
        }
    }
    Ok(project_root)
}

fn validate_synfs_root(root: &str) -> Result<&str, CompileError> {
    if root.is_empty() || root == "/" || root.ends_with('/') || root.contains('\0') {
        return Err(CompileError::InvalidSynFsSourceRoot(root.into()));
    }
    Ok(root)
}

fn synfs_manifest_path(source_root: &str, manifest: &str) -> Result<String, CompileError> {
    if manifest.is_empty() || manifest.contains('\0') {
        return Err(CompileError::InvalidSynFsManifest(manifest.into()));
    }
    let path = if manifest.starts_with('/') {
        manifest.to_string()
    } else {
        format!("{source_root}/{manifest}")
    };
    if !is_synfs_member(source_root, &path) {
        return Err(CompileError::InvalidSynFsManifest(path));
    }
    Ok(path)
}

fn relative_synfs_path(source_root: &str, path: &str) -> Result<PathBuf, CompileError> {
    let relative = path
        .strip_prefix(source_root)
        .and_then(|suffix| suffix.strip_prefix('/'))
        .filter(|suffix| !suffix.is_empty())
        .ok_or_else(|| CompileError::InvalidSynFsManifest(path.into()))?;
    Ok(PathBuf::from(relative))
}

fn is_synfs_member(root: &str, path: &str) -> bool {
    path.strip_prefix(root)
        .is_some_and(|suffix| suffix.starts_with('/'))
}

fn join_synfs_path(directory: &str, name: &str) -> String {
    format!("{}/{}", directory.trim_end_matches('/'), name)
}

fn bundled_linker() -> Option<PathBuf> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
    let output = Command::new(rustc)
        .args(["--print", "target-libdir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None
    }
    let libdir = PathBuf::from(String::from_utf8(output.stdout).ok()?.trim());
    let linker = libdir.parent()?.join("bin").join("rust-lld");
    linker.is_file().then_some(linker)
}
