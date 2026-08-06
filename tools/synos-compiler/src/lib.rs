use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use synos_app::{parse_image, AppManifest, AppTarget, ImageArchitecture};
use synos_pkg::{
    application_bundle_size, bundle_size, encode_application_bundle, encode_bundle,
    ApplicationBundleInfo, ApplicationPackageManifest, BundleInfo, PackageError, SigningKey,
};
use synos_synfs::{DirectoryEntry, Error as SynFsError, FileType, SynFs};
use synos_system_model::ContentId;

mod toolchain;

pub use toolchain::{
    HostToolchain, ToolchainAsset, ToolchainAssetKind, ToolchainError, ToolchainManifest,
    ToolchainManager, ToolchainPackageOutput, ToolchainStage, verify_bundle,
};

pub const SYNOS_TOOLCHAIN_ROOT: &str = "/system/toolchains/stage-2";
pub const SYNOS_REGISTRY_ROOT: &str = "/system/registries";
pub const SYNOS_SOURCE_ROOT: &str = "/system/sources";
pub const SYNOS_BUILD_ROOT: &str = "/system/builds";
pub const SYNOS_TEMP_ROOT: &str = "/system/tmp";

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

    pub const fn rust_target(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-none",
            Self::Aarch64 => "aarch64-unknown-none",
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
pub struct BuildRecord {
    pub source: ContentId,
    pub artifact: ContentId,
    pub target: Target,
    pub release: bool,
    pub reproducible: bool,
}

impl BuildRecord {
    pub fn encode(self) -> [u8; 67] {
        let mut bytes = [0; 67];
        bytes[..32].copy_from_slice(self.source.as_bytes());
        bytes[32..64].copy_from_slice(self.artifact.as_bytes());
        bytes[64] = match self.target {
            Target::X86_64 => 1,
            Target::Aarch64 => 2,
        };
        bytes[65] = self.release as u8;
        bytes[66] = self.reproducible as u8;
        bytes
    }

    pub fn content_id(self) -> ContentId {
        ContentId::hash(&self.encode())
    }
}

#[derive(Clone, Debug)]
pub struct ApplicationBundleOutput {
    pub bundle: PathBuf,
    pub artifact: PathBuf,
    pub debug_symbols: Option<PathBuf>,
    pub stripped_artifact: Option<PathBuf>,
    pub info: ApplicationBundleInfo,
    pub build_record: ContentId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceOutput {
    pub target: Target,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReproducibleWorkspaceOutput {
    pub target: Target,
    pub workspace: PathBuf,
    pub first_target: PathBuf,
    pub second_target: PathBuf,
    pub artifact_count: usize,
    pub digest: ContentId,
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
    CleanWorkspaceExists(PathBuf),
    WorkspaceCopy { path: PathBuf, error: String },
    NoArtifacts(PathBuf),
    NonReproducible { path: String },
    ApplicationManifest(PathBuf),
    ApplicationProfile(String),
    ApplicationTargetMismatch { profile: String, requested: Target },
    InvalidApplicationImage(String),
    StripperUnavailable,
    StripFailed(ExitStatus),
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
            Self::CleanWorkspaceExists(path) => {
                write!(formatter, "clean workspace already exists: {}", path.display())
            }
            Self::WorkspaceCopy { path, error } => {
                write!(formatter, "could not copy workspace item {}: {error}", path.display())
            }
            Self::NoArtifacts(path) => {
                write!(formatter, "clean build produced no artifacts under {}", path.display())
            }
            Self::NonReproducible { path } => {
                write!(formatter, "clean builds differ at {path}")
            }
            Self::ApplicationManifest(path) => {
                write!(formatter, "could not read application manifest {}", path.display())
            }
            Self::ApplicationProfile(error) => write!(formatter, "invalid application profile: {error}"),
            Self::ApplicationTargetMismatch { profile, requested } => {
                write!(formatter, "application targets {profile}, compiler requested {requested:?}")
            }
            Self::InvalidApplicationImage(error) => write!(formatter, "invalid application image: {error}"),
            Self::StripperUnavailable => write!(formatter, "llvm-strip is required for a stripped release image"),
            Self::StripFailed(status) => write!(formatter, "stripping application image failed with {status}"),
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
    rustc: Option<PathBuf>,
    rustdoc: Option<PathBuf>,
    linker: Option<PathBuf>,
    stripper: Option<PathBuf>,
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
            rustc: None,
            rustdoc: None,
            linker: bundled_linker(),
            stripper: find_tool("llvm-strip"),
            workspace_root,
        })
    }

    /// Construct a compiler view for an in-guest SynOS stage-2 toolchain.
    /// Paths are explicit capabilities/roots; no host `PATH`, `HOME`, or
    /// temporary directory is consulted.
    pub fn with_synos_toolchain(workspace_root: PathBuf, toolchain_root: &Path) -> Self {
        Self {
            cargo: toolchain_root.join("bin/cargo").into_os_string(),
            rustc: Some(toolchain_root.join("bin/rustc")),
            rustdoc: Some(toolchain_root.join("bin/rustdoc")),
            linker: Some(toolchain_root.join("bin/rust-lld")),
            stripper: Some(toolchain_root.join("bin/llvm-strip")),
            workspace_root,
        }
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

    pub fn compile_application_and_bundle(
        &self,
        request: &CompileRequest,
        profile_path: &Path,
        key: SigningKey,
        output: &Path,
        debug_symbols: Option<&Path>,
        stripped_output: Option<&Path>,
    ) -> Result<ApplicationBundleOutput, CompileError> {
        let profile_source = fs::read_to_string(profile_path)
            .map_err(|_| CompileError::ApplicationManifest(profile_path.to_path_buf()))?;
        let profile = AppManifest::parse(&profile_source)
            .map_err(|error| CompileError::ApplicationProfile(format!("{error:?}")))?;
        profile
            .validate_profile()
            .map_err(|error| CompileError::ApplicationProfile(format!("{error:?}")))?;
        let expected_target = match profile.target().expect("validated application target") {
            AppTarget::X86_64 => Target::X86_64,
            AppTarget::Aarch64 => Target::Aarch64,
        };
        if expected_target != request.target {
            return Err(CompileError::ApplicationTargetMismatch {
                profile: expected_target.target_file().into(),
                requested: request.target,
            });
        }
        let compiled = self.compile(request)?;
        let payload = fs::read(&compiled.artifact).map_err(|error| CompileError::PackageWrite {
            path: compiled.artifact.clone(),
            error: error.to_string(),
        })?;
        let architecture = match request.target {
            Target::X86_64 => ImageArchitecture::X86_64,
            Target::Aarch64 => ImageArchitecture::Aarch64,
        };
        parse_image(&payload, architecture)
            .map_err(|error| CompileError::InvalidApplicationImage(format!("{error:?}")))?;
        let entry_offset = profile.entry_offset().expect("validated entry offset");
        if entry_offset >= payload.len() as u64 {
            return Err(CompileError::InvalidApplicationImage(
                "entry offset is outside the linked image".into(),
            ));
        }
        let artifact_id = ContentId::hash(&payload);
        let source_id = ContentId::hash(profile_source.as_bytes());
        let build = BuildRecord {
            source: source_id,
            artifact: artifact_id,
            target: request.target,
            release: request.release,
            reproducible: request.locked && request.offline,
        };
        let build_record = build.content_id();
        let debug_path = debug_symbols.map(Path::to_path_buf);
        if let Some(path) = &debug_path {
            fs::copy(&compiled.artifact, path).map_err(|error| CompileError::PackageWrite {
                path: path.clone(),
                error: error.to_string(),
            })?;
        }
        let stripped_path = if let Some(path) = stripped_output {
            self.strip_image(&compiled.artifact, path)?;
            Some(path.to_path_buf())
        } else {
            None
        };
        let debug_id = debug_path
            .as_ref()
            .map(|path| ContentId::hash(&fs::read(path).unwrap_or_default()))
            .unwrap_or_else(|| ContentId::from_bytes([0; 32]));
        let metadata = ApplicationPackageManifest::new(
            profile.schema(),
            match expected_target {
                Target::X86_64 => 1,
                Target::Aarch64 => 2,
            },
            profile.kind() as u8 + 1,
            profile.name().as_str(),
            entry_offset,
            profile.resources().memory_bytes,
            profile.resources().cpu_time_us,
            profile.resources().heap_bytes,
            debug_id,
            build_record,
        )?;
        let dependencies = profile.dependencies().map(|dependency| dependency.package).collect::<Vec<_>>();
        let inner_size = bundle_size(payload.len(), dependencies.len())?;
        let mut inner = vec![0; inner_size];
        encode_bundle(&payload, entry_offset, &dependencies, key, &mut inner)?;
        let required = application_bundle_size(inner.len())?;
        let mut encoded = vec![0; required];
        let info = encode_application_bundle(&inner, metadata, key, &mut encoded)?;
        write_atomic(output, &encoded)?;
        let record_path = output.with_extension("build-record");
        write_atomic(&record_path, &build.encode())?;
        Ok(ApplicationBundleOutput {
            bundle: output.to_path_buf(),
            artifact: compiled.artifact,
            debug_symbols: debug_path,
            stripped_artifact: stripped_path,
            info,
            build_record,
        })
    }

    fn strip_image(&self, input: &Path, output: &Path) -> Result<(), CompileError> {
        let stripper = self.stripper.as_ref().ok_or(CompileError::StripperUnavailable)?;
        let status = Command::new(stripper)
            .arg(input)
            .arg("-o")
            .arg(output)
            .status()
            .map_err(|_| CompileError::StripperUnavailable)?;
        if status.success() {
            Ok(())
        } else {
            Err(CompileError::StripFailed(status))
        }
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

    /// Build one clean source workspace twice with fresh target directories,
    /// Cargo locked and offline, then compare final artifacts by content ID.
    pub fn reproduce_workspace(
        &self,
        target: Target,
        release: bool,
        clean_root: &Path,
    ) -> Result<ReproducibleWorkspaceOutput, CompileError> {
        if clean_root.exists() {
            return Err(CompileError::CleanWorkspaceExists(clean_root.to_path_buf()));
        }
        let workspace = clean_root.join("workspace");
        copy_clean_workspace(&self.workspace_root, &workspace)?;

        let first_target = clean_root.join("target-first");
        let second_target = clean_root.join("target-second");
        build_clean_workspace(&workspace, &first_target, target, release)?;
        fs::rename(&first_target, &second_target).map_err(|error| {
            CompileError::WorkspaceCopy {
                path: first_target.clone(),
                error: error.to_string(),
            }
        })?;
        build_clean_workspace(&workspace, &first_target, target, release)?;

        let first_output = workspace_output_directory(&second_target, target, release);
        let second_output = workspace_output_directory(&first_target, target, release);
        let first_artifacts = collect_artifacts(&first_output)?;
        let second_artifacts = collect_artifacts(&second_output)?;
        if first_artifacts.is_empty() {
            return Err(CompileError::NoArtifacts(first_output));
        }
        compare_artifacts(&first_artifacts, &second_artifacts)?;
        let digest = digest_artifacts(&first_artifacts);
        Ok(ReproducibleWorkspaceOutput {
            target,
            workspace,
            first_target,
            second_target,
            artifact_count: first_artifacts.len(),
            digest,
        })
    }

    fn base_command(&self) -> Command {
        let mut command = Command::new(&self.cargo);
        command
            .env("RUSTC_BOOTSTRAP", "1")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .args([
                "-Z",
                "build-std=core,alloc",
                "-Z",
                "json-target-spec",
            ]);
        if let Some(rustc) = self.rustc.as_ref() {
            command.env("RUSTC", rustc);
        }
        if let Some(rustdoc) = self.rustdoc.as_ref() {
            command.env("RUSTDOC", rustdoc);
        }
        if let Some(linker) = self.linker.as_ref() {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArtifactDigest {
    path: String,
    digest: ContentId,
}

fn copy_clean_workspace(source: &Path, destination: &Path) -> Result<(), CompileError> {
    if destination.exists() {
        return Err(CompileError::CleanWorkspaceExists(destination.to_path_buf()));
    }
    copy_workspace_tree(source, destination)
}

fn copy_workspace_tree(source: &Path, destination: &Path) -> Result<(), CompileError> {
    fs::create_dir_all(destination).map_err(|error| CompileError::WorkspaceCopy {
        path: destination.to_path_buf(),
        error: error.to_string(),
    })?;
    let entries = fs::read_dir(source).map_err(|error| CompileError::WorkspaceCopy {
        path: source.to_path_buf(),
        error: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| CompileError::WorkspaceCopy {
            path: source.to_path_buf(),
            error: error.to_string(),
        })?;
        let name = entry.file_name();
        if name == ".git" || name == "target" || name == "build" || name == "clients" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(name);
        let file_type = entry
            .file_type()
            .map_err(|error| CompileError::WorkspaceCopy {
                path: source_path.clone(),
                error: error.to_string(),
            })?;
        if file_type.is_symlink() {
            return Err(CompileError::WorkspaceCopy {
                path: source_path,
                error: "symbolic links are not allowed in a clean workspace".into(),
            });
        }
        if file_type.is_dir() {
            copy_workspace_tree(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                CompileError::WorkspaceCopy {
                    path: source_path,
                    error: error.to_string(),
                }
            })?;
        }
    }
    Ok(())
}

fn build_clean_workspace(
    workspace: &Path,
    target_directory: &Path,
    target: Target,
    release: bool,
) -> Result<(), CompileError> {
    let target_file = workspace.join("targets").join(target.target_file());
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")));
    command
        .current_dir(workspace)
        .env("CARGO_INCREMENTAL", "0")
        .env("SOURCE_DATE_EPOCH", "0")
        .env("CONST_RANDOM_SEED", "synos-reproducible-seed-v1")
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["-Z", "build-std=core,alloc", "-Z", "json-target-spec"])
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
        .arg(target_file)
        .arg("--target-dir")
        .arg(target_directory)
        .arg("--locked")
        .arg("--offline");
    if release {
        command.arg("--release");
    }
    let mut rustflags = env::var("RUSTFLAGS").unwrap_or_default();
    if let Some(linker) = bundled_linker() {
        if !rustflags.is_empty() {
            rustflags.push(' ')
        }
        rustflags.push_str("-C linker=");
        rustflags.push_str(&linker.to_string_lossy());
    }
    rustflags.push_str(" --remap-path-prefix=");
    rustflags.push_str(&workspace.to_string_lossy());
    rustflags.push_str("=/synos-clean-workspace");
    rustflags.push_str(" --remap-path-prefix=");
    rustflags.push_str(&target_directory.to_string_lossy());
    rustflags.push_str("=/synos-clean-target");
    command.env("RUSTFLAGS", rustflags);
    let status = command
        .status()
        .map_err(|error| CompileError::CargoUnavailable(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(CompileError::BuildFailed(status))
    }
}

fn workspace_output_directory(target_directory: &Path, target: Target, release: bool) -> PathBuf {
    target_directory
        .join(target.target_file().trim_end_matches(".json"))
        .join(if release { "release" } else { "debug" })
}

fn collect_artifacts(root: &Path) -> Result<Vec<ArtifactDigest>, CompileError> {
    let mut artifacts = Vec::new();
    collect_artifacts_from(root, root, &mut artifacts)?;
    artifacts.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(artifacts)
}

fn collect_artifacts_from(
    root: &Path,
    directory: &Path,
    artifacts: &mut Vec<ArtifactDigest>,
) -> Result<(), CompileError> {
    let entries = fs::read_dir(directory).map_err(|error| CompileError::WorkspaceCopy {
        path: directory.to_path_buf(),
        error: error.to_string(),
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| CompileError::WorkspaceCopy {
            path: directory.to_path_buf(),
            error: error.to_string(),
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| CompileError::WorkspaceCopy {
                path: path.clone(),
                error: error.to_string(),
            })?;
        if file_type.is_file() && is_reproducible_artifact(&path) {
            let bytes = fs::read(&path).map_err(|error| CompileError::WorkspaceCopy {
                path: path.clone(),
                error: error.to_string(),
            })?;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            artifacts.push(ArtifactDigest {
                path: relative,
                digest: artifact_digest(&path, &bytes),
            });
        }
    }
    Ok(())
}

fn artifact_digest(path: &Path, bytes: &[u8]) -> ContentId {
    if path.extension().is_some_and(|extension| extension == "rlib") {
        if let Some(digest) = rlib_code_digest(bytes) {
            return digest;
        }
    }
    ContentId::hash(bytes)
}

fn rlib_code_digest(bytes: &[u8]) -> Option<ContentId> {
    const AR_MAGIC: &[u8; 8] = b"!<arch>\n";
    if bytes.get(..AR_MAGIC.len())? != AR_MAGIC {
        return None;
    }

    let mut offset = AR_MAGIC.len();
    let mut members = Vec::new();
    while offset < bytes.len() {
        let header_end = offset.checked_add(60)?;
        let header = bytes.get(offset..header_end)?;
        if header[58..60] != *b"`\n" {
            return None;
        }
        let name = core::str::from_utf8(&header[..16])
            .ok()?
            .trim()
            .trim_end_matches('/')
            .to_string();
        let length = core::str::from_utf8(&header[48..58])
            .ok()?
            .trim()
            .parse::<usize>()
            .ok()?;
        let data_start = header_end;
        let data_end = data_start.checked_add(length)?;
        let data = bytes.get(data_start..data_end)?;
        offset = data_end.checked_add(length & 1)?;
        if matches!(name.as_str(), "" | "lib.rmeta" | "lib.rmeta-link") {
            continue;
        }
        members.push((name, ContentId::hash(data)));
    }
    members.sort_by(|left, right| left.0.cmp(&right.0));
    if members.is_empty() {
        return None;
    }
    let mut material = Vec::new();
    for (name, digest) in members {
        material.extend_from_slice(name.as_bytes());
        material.push(0);
        material.extend_from_slice(digest.as_bytes());
    }
    Some(ContentId::hash(&material))
}

fn is_reproducible_artifact(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        matches!(
            extension.to_str(),
            Some("rlib") | Some("rmeta") | Some("a") | Some("o") | Some("so")
        )
    })
}

fn compare_artifacts(
    first: &[ArtifactDigest],
    second: &[ArtifactDigest],
) -> Result<(), CompileError> {
    if first.len() != second.len() {
        return Err(CompileError::NonReproducible {
            path: format!("artifact count {} != {}", first.len(), second.len()),
        });
    }
    for (left, right) in first.iter().zip(second) {
        if left != right {
            return Err(CompileError::NonReproducible {
                path: left.path.clone(),
            });
        }
    }
    Ok(())
}

fn digest_artifacts(artifacts: &[ArtifactDigest]) -> ContentId {
    let mut material = Vec::new();
    for artifact in artifacts {
        material.extend_from_slice(artifact.path.as_bytes());
        material.push(0);
        material.extend_from_slice(artifact.digest.as_bytes());
    }
    ContentId::hash(&material)
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

fn find_tool(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}
