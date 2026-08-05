use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

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
    pub target: Target,
    pub release: bool,
    pub target_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileOutput {
    pub artifact: PathBuf,
    pub target: Target,
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
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CargoUnavailable(error) => write!(formatter, "could not start Cargo: {error}"),
            Self::InvalidManifest(path) => write!(formatter, "manifest does not exist: {}", path.display()),
            Self::UnsupportedTarget(target) => write!(formatter, "unsupported SynOS target `{target}`"),
            Self::BuildFailed(status) => write!(formatter, "SynOS compilation failed with {status}"),
            Self::InvalidTargetDirectory => write!(formatter, "could not locate the workspace root"),
        }
    }
}

impl std::error::Error for CompileError {}

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
        if request.release {
            command.arg("--release");
        }
        if let Some(target_directory) = &request.target_directory {
            command.arg("--target-dir").arg(target_directory);
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
