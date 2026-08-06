use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use synos_compiler::{
    CompileRequest, Compiler, HostToolchain, Target, ToolchainManager, ToolchainStage,
    verify_bundle,
};
use synos_pkg::{SigningKey, bundle_size, encode_bundle};
use synos_system_model::ContentId;

const USAGE: &str = "\
cargo synos build [--target x86_64|aarch64] [--release] [cargo options]
cargo synos check [--target x86_64|aarch64] [--release] [cargo options]
cargo synos test [--target x86_64|aarch64] [--release] [cargo options]
cargo synos doc [--target x86_64|aarch64] [--release] [cargo options]
cargo synos bundle --artifact PATH --key PATH --output PATH
    [--entry-offset BYTES] [--dependency SHA256]...
cargo synos package --bin NAME --key PATH --output PATH
    [--manifest-path PATH] [--package NAME] [--target x86_64|aarch64]
    [--release] [--locked] [--offline] [--target-dir PATH]
    [--app-manifest PATH] [--debug-symbols PATH] [--stripped-output PATH]
    [--entry-offset BYTES] [--dependency SHA256]...
cargo synos compile --manifest-path PATH --bin NAME
    [--target x86_64|aarch64] [--release] [--locked] [--offline]
    [--target-dir PATH]
cargo synos compile-all [--target x86_64|aarch64] [--release] [--target-dir PATH]
cargo synos reproduce [--target x86_64|aarch64] [--release] [--clean-root PATH]
cargo synos toolchain package --key PATH --output PATH
    [--stage 0|1|2] [--target x86_64|aarch64] [--root PATH] [--rust-version TEXT]
cargo synos toolchain verify --bundle PATH --key PATH
cargo synos toolchain install|update --bundle PATH --key PATH --root PATH
    [--stage 0|1|2] [--target x86_64|aarch64]
cargo synos toolchain select --root PATH --stage 0|1|2
cargo synos toolchain rollback --root PATH
cargo synos run --manifest-path PATH --bin NAME [--release] [-- ARGUMENT]...

Keys may contain 32 raw bytes or 64 hexadecimal characters.";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cargo-synos: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|argument| argument == "synos")
    {
        arguments.remove(0);
    }
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(USAGE.into());
    };
    match command {
        "build" => build(&arguments[1..]),
        "check" => cargo_operation("check", &arguments[1..]),
        "test" => cargo_operation("test", &arguments[1..]),
        "doc" => cargo_operation("doc", &arguments[1..]),
        "bundle" => bundle(&arguments[1..]),
        "package" => package(&arguments[1..]),
        "compile" => compile(&arguments[1..]),
        "compile-all" => compile_all(&arguments[1..]),
        "reproduce" => reproduce(&arguments[1..]),
        "toolchain" => toolchain(&arguments[1..]),
        "run" => run_program(&arguments[1..]),
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

fn toolchain(arguments: &[String]) -> Result<(), String> {
    let Some(operation) = arguments.first().map(String::as_str) else {
        return Err("toolchain needs package or verify".into());
    };
    match operation {
        "package" => package_toolchain(&arguments[1..]),
        "verify" => verify_toolchain(&arguments[1..]),
        "install" => manage_toolchain(&arguments[1..], false),
        "update" => manage_toolchain(&arguments[1..], true),
        "select" => select_toolchain(&arguments[1..]),
        "rollback" => rollback_toolchain(&arguments[1..]),
        other => Err(format!("unknown toolchain operation `{other}`")),
    }
}

fn package_toolchain(arguments: &[String]) -> Result<(), String> {
    let options = ToolchainPackageOptions::parse(arguments)?;
    let key = read_key(&options.key)?;
    let toolchain = if let Some(root) = options.root {
        HostToolchain::from_root(
            &root,
            options.stage,
            options.target,
            options.rust_version.unwrap_or_else(|| "prebuilt".into()),
        )
    } else {
        if options.stage != ToolchainStage::Stage0 {
            return Err("--root is required when packaging stage 1 or stage 2".into());
        }
        HostToolchain::discover(options.stage, options.target)
    }
    .map_err(|error| error.to_string())?;
    let output = toolchain
        .package(&options.output, key)
        .map_err(|error| error.to_string())?;
    println!(
        "created {:?} toolchain {} ({:?}, {} assets)",
        output.manifest.stage,
        output.bundle.display(),
        output.info.package,
        output.manifest.assets.len()
    );
    Ok(())
}

fn verify_toolchain(arguments: &[String]) -> Result<(), String> {
    let options = ToolchainVerifyOptions::parse(arguments)?;
    let key = read_key(&options.key)?;
    let bytes = fs::read(&options.bundle)
        .map_err(|error| format!("could not read {}: {error}", options.bundle.display()))?;
    let manifest = verify_bundle(&bytes, key).map_err(|error| error.to_string())?;
    println!(
        "verified {:?} toolchain for {:?}: {:?}, {} assets",
        manifest.stage,
        manifest.target,
        manifest.content,
        manifest.assets.len()
    );
    Ok(())
}

fn manage_toolchain(arguments: &[String], update: bool) -> Result<(), String> {
    let options = ToolchainManageOptions::parse(arguments, true)?;
    let key = read_key(&options.key.ok_or_else(|| "--key is required".to_string())?)?;
    let manager = ToolchainManager::new(options.root).map_err(|error| error.to_string())?;
    let manifest = manager
        .install(
            &options.bundle.ok_or_else(|| "--bundle is required".to_string())?,
            key,
            options.stage,
            options.target,
        )
        .map_err(|error| error.to_string())?;
    println!(
        "{} toolchain {:?} for {:?} installed",
        if update { "updated" } else { "installed" },
        manifest.stage,
        manifest.target
    );
    Ok(())
}

fn select_toolchain(arguments: &[String]) -> Result<(), String> {
    let options = ToolchainManageOptions::parse(arguments, false)?;
    let manager = ToolchainManager::new(options.root).map_err(|error| error.to_string())?;
    manager
        .select(options.stage)
        .map_err(|error| error.to_string())?;
    println!("selected toolchain stage-{}", options.stage as u8);
    Ok(())
}

fn rollback_toolchain(arguments: &[String]) -> Result<(), String> {
    let options = ToolchainManageOptions::parse(arguments, false)?;
    let manager = ToolchainManager::new(options.root).map_err(|error| error.to_string())?;
    let stage = manager.rollback().map_err(|error| error.to_string())?;
    println!("rolled back toolchain stage-{}", stage as u8);
    Ok(())
}

fn compile(arguments: &[String]) -> Result<(), String> {
    let options = CompileOptions::parse(arguments)?;
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    let output = compiler
        .compile(&CompileRequest {
            manifest_path: options.manifest_path,
            binary: options.binary,
            package: None,
            target: options.target,
            release: options.release,
            target_directory: options.target_directory,
            locked: options.locked,
            offline: options.offline,
        })
        .map_err(|error| error.to_string())?;
    println!("compiled {}", output.artifact.display());
    Ok(())
}

fn run_program(arguments: &[String]) -> Result<(), String> {
    let options = RunOptions::parse(arguments)?;
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    let status = compiler
        .run_host(&options.manifest_path, &options.binary, options.release, &options.arguments)
        .map_err(|error| error.to_string())?;
    if status.success() {
        if options.json {
            println!("{{\"command\":\"run\",\"success\":true}}");
        }
        Ok(())
    } else {
        if options.json {
            println!("{{\"command\":\"run\",\"success\":false}}");
        }
        Err(format!("program exited with {status}"))
    }
}

fn compile_all(arguments: &[String]) -> Result<(), String> {
    let options = WorkspaceOptions::parse(arguments)?;
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    compiler
        .compile_workspace(
            options.target,
            options.release,
            options.target_directory.as_deref(),
        )
        .map_err(|error| error.to_string())?;
    println!("compiled SynOS workspace for {:?}", options.target);
    Ok(())
}

fn reproduce(arguments: &[String]) -> Result<(), String> {
    let options = ReproduceOptions::parse(arguments)?;
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    let clean_root = options
        .clean_root
        .unwrap_or_else(|| env::temp_dir().join(format!("synos-reproduce-{}", std::process::id())));
    let output = compiler
        .reproduce_workspace(options.target, options.release, &clean_root)
        .map_err(|error| error.to_string())?;
    println!(
        "reproduced {:?}: {} artifacts, digest {:?}",
        output.target, output.artifact_count, output.digest
    );
    println!(
        "clean workspace: {}; targets: {} and {}",
        output.workspace.display(),
        output.first_target.display(),
        output.second_target.display()
    );
    Ok(())
}

fn build(arguments: &[String]) -> Result<(), String> {
    cargo_operation("build", arguments)
}

fn cargo_operation(operation: &str, arguments: &[String]) -> Result<(), String> {
    let options = CargoOperationOptions::parse(arguments)?;
    let target_file = target_path(&options.target)?;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["-Z", "build-std=core,alloc", "-Z", "json-target-spec"])
        .arg(operation)
        .arg("--target")
        .arg(target_file);
    if let Some(manifest) = options.manifest_path {
        command.arg("--manifest-path").arg(manifest);
    }
    if let Some(package) = options.package {
        command.arg("--package").arg(package);
    }
    if let Some(binary) = options.binary {
        command.arg("--bin").arg(binary);
    }
    if options.release {
        command.arg("--release");
    }
    if options.locked {
        command.arg("--locked");
    }
    if options.offline {
        command.arg("--offline");
    }
    if let Some(target_directory) = options.target_directory {
        command.arg("--target-dir").arg(target_directory);
    }
    if options.json {
        command.arg("--message-format=json");
    }
    command.args(options.extra);
    let status = command
        .status()
        .map_err(|error| format!("could not start Cargo: {error}"))?;
    if options.json {
        println!(
            "{{\"command\":\"{operation}\",\"success\":{},\"target\":\"{}\"}}",
            status.success(),
            options.target
        );
    } else if status.success() {
        println!("rust {operation}: ok");
    }
    if status.success() {
        Ok(())
    } else {
        Err(format!("rust {operation} failed with {status}"))
    }
}

fn bundle(arguments: &[String]) -> Result<(), String> {
    let options = BundleOptions::parse(arguments)?;
    create_bundle(&options)
}

fn package(arguments: &[String]) -> Result<(), String> {
    let options = PackageOptions::parse(arguments)?;
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    let key = read_key(&options.key)?;
    let request = CompileRequest {
                manifest_path: options
                    .manifest_path
                    .unwrap_or(workspace_root()?.join("Cargo.toml")),
                binary: options.binary,
                package: options.package,
                target: Target::parse(&options.target).map_err(|error| error.to_string())?,
                release: options.release,
                target_directory: options.target_directory,
                locked: options.locked,
                offline: options.offline,
            };
    if let Some(profile) = options.app_manifest {
        let output = compiler
            .compile_application_and_bundle(
                &request,
                &profile,
                key,
                &options.output,
                options.debug_symbols.as_deref(),
                options.stripped_output.as_deref(),
            )
            .map_err(|error| error.to_string())?;
        println!(
            "created {} application {:?} (build {:?})",
            output.bundle.display(),
            output.info.package,
            output.build_record
        );
    } else {
        let output = compiler
            .compile_and_bundle(
                &request,
                key,
                &options.output,
                options.entry_offset,
                &options.dependencies,
            )
            .map_err(|error| error.to_string())?;
        println!(
            "created {} package {:?} ({} bytes)",
            output.bundle.display(),
            output.info.package,
            output.info.payload_length
        );
    }
    Ok(())
}

fn create_bundle(options: &BundleOptions) -> Result<(), String> {
    let payload = fs::read(&options.artifact)
        .map_err(|error| format!("could not read {}: {error}", options.artifact.display()))?;
    let key = read_key(&options.key)?;
    let required = bundle_size(payload.len(), options.dependencies.len())
        .map_err(|error| format!("invalid bundle: {error:?}"))?;
    let mut encoded = vec![0; required];
    let info = encode_bundle(
        &payload,
        options.entry_offset,
        &options.dependencies,
        key,
        &mut encoded,
    )
    .map_err(|error| format!("could not encode bundle: {error:?}"))?;
    fs::write(&options.output, encoded)
        .map_err(|error| format!("could not write {}: {error}", options.output.display()))?;
    println!(
        "created {} package {:?} ({} bytes)",
        options.output.display(),
        info.package,
        info.payload_length
    );
    Ok(())
}

fn target_path(target: &str) -> Result<PathBuf, String> {
    let file = match target {
        "x86_64" | "x86_64-unknown-synos" => "x86_64-unknown-synos.json",
        "aarch64" | "aarch64-unknown-synos" => "aarch64-unknown-synos.json",
        value if value.ends_with(".json") => return Ok(PathBuf::from(value)),
        other => return Err(format!("unsupported SynOS target `{other}`")),
    };
    Ok(workspace_root()?.join("targets").join(file))
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not locate the SynOS workspace".to_string())
}

fn read_key(path: &Path) -> Result<SigningKey, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read key {}: {error}", path.display()))?;
    let decoded = if bytes.len() == 32 {
        bytes
    } else {
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "signing key must be raw bytes or hexadecimal".to_string())?
            .trim();
        decode_hex(text)?
    };
    let key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| "signing key must contain exactly 32 bytes".to_string())?;
    Ok(SigningKey::new(key))
}

fn decode_content_id(value: &str) -> Result<ContentId, String> {
    let bytes = decode_hex(value)?;
    let content: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "dependency digest must contain 32 bytes".to_string())?;
    Ok(ContentId::from_bytes(content))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("hexadecimal value has an odd length".into());
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(byte: u8) -> Result<u8, String> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err("invalid hexadecimal character".into()),
    }
}

struct BundleOptions {
    artifact: PathBuf,
    key: PathBuf,
    output: PathBuf,
    entry_offset: u64,
    dependencies: Vec<ContentId>,
}

struct CargoOperationOptions {
    target: String,
    release: bool,
    locked: bool,
    offline: bool,
    json: bool,
    manifest_path: Option<PathBuf>,
    package: Option<String>,
    binary: Option<String>,
    target_directory: Option<PathBuf>,
    extra: Vec<String>,
}

impl CargoOperationOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut options = Self {
            target: "x86_64".into(),
            release: false,
            locked: false,
            offline: false,
            json: false,
            manifest_path: None,
            package: None,
            binary: None,
            target_directory: None,
            extra: Vec::new(),
        };
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--release" => {
                    options.release = true;
                    index += 1;
                }
                "--locked" => {
                    options.locked = true;
                    index += 1;
                }
                "--offline" => {
                    options.offline = true;
                    index += 1;
                }
                "--json" => {
                    options.json = true;
                    index += 1;
                }
                "--target" => {
                    options.target = required_value(arguments, &mut index)?.into();
                }
                "--manifest-path" => {
                    options.manifest_path = Some(PathBuf::from(required_value(arguments, &mut index)?));
                }
                "--package" => {
                    options.package = Some(required_value(arguments, &mut index)?.into());
                }
                "--bin" => {
                    options.binary = Some(required_value(arguments, &mut index)?.into());
                }
                "--target-dir" => {
                    options.target_directory = Some(PathBuf::from(required_value(arguments, &mut index)?));
                }
                other => {
                    options.extra.push(other.into());
                    index += 1;
                }
            }
        }
        Ok(options)
    }
}

struct CompileOptions {
    manifest_path: PathBuf,
    binary: String,
    target: Target,
    release: bool,
    target_directory: Option<PathBuf>,
    locked: bool,
    offline: bool,
}

impl CompileOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut manifest_path = None;
        let mut binary = None;
        let mut target = Target::X86_64;
        let mut release = false;
        let mut target_directory = None;
        let mut locked = false;
        let mut offline = false;
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--manifest-path" => {
                    manifest_path = Some(PathBuf::from(required_value(arguments, &mut index)?))
                }
                "--bin" => binary = Some(required_value(arguments, &mut index)?.to_string()),
                "--target" => {
                    target = Target::parse(required_value(arguments, &mut index)?)
                        .map_err(|error| error.to_string())?
                }
                "--release" => {
                    release = true;
                    index += 1;
                }
                "--locked" => {
                    locked = true;
                    index += 1;
                }
                "--offline" => {
                    offline = true;
                    index += 1;
                }
                "--target-dir" => {
                    target_directory = Some(PathBuf::from(required_value(arguments, &mut index)?))
                }
                other => return Err(format!("unknown compile option `{other}`")),
            }
        }
        Ok(Self {
            manifest_path: manifest_path.ok_or_else(|| "--manifest-path is required".to_string())?,
            binary: binary.ok_or_else(|| "--bin is required".to_string())?,
            target,
            release,
            target_directory,
            locked,
            offline,
        })
    }
}

struct RunOptions {
    manifest_path: PathBuf,
    binary: String,
    release: bool,
    arguments: Vec<String>,
    json: bool,
}

struct WorkspaceOptions {
    target: Target,
    release: bool,
    target_directory: Option<PathBuf>,
}

struct ReproduceOptions {
    target: Target,
    release: bool,
    clean_root: Option<PathBuf>,
}

struct ToolchainPackageOptions {
    stage: ToolchainStage,
    target: Target,
    root: Option<PathBuf>,
    rust_version: Option<String>,
    key: PathBuf,
    output: PathBuf,
}

struct ToolchainVerifyOptions {
    bundle: PathBuf,
    key: PathBuf,
}

struct ToolchainManageOptions {
    root: PathBuf,
    stage: ToolchainStage,
    target: Option<Target>,
    bundle: Option<PathBuf>,
    key: Option<PathBuf>,
}

impl WorkspaceOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut target = Target::X86_64;
        let mut release = false;
        let mut target_directory = None;
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--target" => {
                    target = Target::parse(required_value(arguments, &mut index)?)
                        .map_err(|error| error.to_string())?
                }
                "--release" => {
                    release = true;
                    index += 1;
                }
                "--target-dir" => {
                    target_directory = Some(PathBuf::from(required_value(arguments, &mut index)?))
                }
                other => return Err(format!("unknown compile-all option `{other}`")),
            }
        }
        Ok(Self {
            target,
            release,
            target_directory,
        })
    }
}

impl ReproduceOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut target = Target::X86_64;
        let mut release = false;
        let mut clean_root = None;
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--target" => {
                    target = Target::parse(required_value(arguments, &mut index)?)
                        .map_err(|error| error.to_string())?
                }
                "--release" => {
                    release = true;
                    index += 1;
                }
                "--clean-root" => {
                    clean_root = Some(PathBuf::from(required_value(arguments, &mut index)?))
                }
                other => return Err(format!("unknown reproduce option `{other}`")),
            }
        }
        Ok(Self {
            target,
            release,
            clean_root,
        })
    }
}

impl ToolchainPackageOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut stage = ToolchainStage::Stage0;
        let mut target = Target::X86_64;
        let mut root = None;
        let mut rust_version = None;
        let mut key = None;
        let mut output = None;
        let mut index = 0;
        while index < arguments.len() {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{} needs a value", arguments[index]))?;
            match arguments[index].as_str() {
                "--stage" => stage = ToolchainStage::parse(value).map_err(|error| error.to_string())?,
                "--target" => target = Target::parse(value).map_err(|error| error.to_string())?,
                "--root" => root = Some(PathBuf::from(value)),
                "--rust-version" => rust_version = Some(value.to_string()),
                "--key" => key = Some(PathBuf::from(value)),
                "--output" => output = Some(PathBuf::from(value)),
                other => return Err(format!("unknown toolchain package option `{other}`")),
            }
            index += 2;
        }
        Ok(Self {
            stage,
            target,
            root,
            rust_version,
            key: key.ok_or_else(|| "--key is required".to_string())?,
            output: output.ok_or_else(|| "--output is required".to_string())?,
        })
    }
}

impl ToolchainVerifyOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut bundle = None;
        let mut key = None;
        let mut index = 0;
        while index < arguments.len() {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{} needs a value", arguments[index]))?;
            match arguments[index].as_str() {
                "--bundle" => bundle = Some(PathBuf::from(value)),
                "--key" => key = Some(PathBuf::from(value)),
                other => return Err(format!("unknown toolchain verify option `{other}`")),
            }
            index += 2;
        }
        Ok(Self {
            bundle: bundle.ok_or_else(|| "--bundle is required".to_string())?,
            key: key.ok_or_else(|| "--key is required".to_string())?,
        })
    }
}

impl ToolchainManageOptions {
    fn parse(arguments: &[String], require_bundle: bool) -> Result<Self, String> {
        let mut root = None;
        let mut stage = ToolchainStage::Stage0;
        let mut target = None;
        let mut bundle = None;
        let mut key = None;
        let mut index = 0;
        while index < arguments.len() {
            let value = required_value(arguments, &mut index)?;
            match arguments[index - 2].as_str() {
                "--root" => root = Some(PathBuf::from(value)),
                "--stage" => stage = ToolchainStage::parse(value).map_err(|error| error.to_string())?,
                "--target" => target = Some(Target::parse(value).map_err(|error| error.to_string())?),
                "--bundle" => bundle = Some(PathBuf::from(value)),
                "--key" => key = Some(PathBuf::from(value)),
                other => return Err(format!("unknown toolchain option `{other}`")),
            }
        }
        if require_bundle && bundle.is_none() {
            return Err("--bundle is required".into());
        }
        Ok(Self {
            root: root.ok_or_else(|| "--root is required".to_string())?,
            stage,
            target,
            bundle,
            key,
        })
    }
}

impl RunOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut manifest_path = None;
        let mut binary = None;
        let mut release = false;
        let mut program_arguments = Vec::new();
        let mut json = false;
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--manifest-path" => {
                    manifest_path = Some(PathBuf::from(required_value(arguments, &mut index)?))
                }
                "--bin" => binary = Some(required_value(arguments, &mut index)?.to_string()),
                "--release" => {
                    release = true;
                    index += 1;
                }
                "--json" => {
                    json = true;
                    index += 1;
                }
                "--" => {
                    program_arguments.extend_from_slice(&arguments[index + 1..]);
                    break
                }
                other => return Err(format!("unknown run option `{other}`")),
            }
        }
        Ok(Self {
            manifest_path: manifest_path.ok_or_else(|| "--manifest-path is required".to_string())?,
            binary: binary.ok_or_else(|| "--bin is required".to_string())?,
            release,
            arguments: program_arguments,
            json,
        })
    }
}

fn required_value<'a>(arguments: &'a [String], index: &mut usize) -> Result<&'a str, String> {
    let value = arguments
        .get(*index + 1)
        .ok_or_else(|| format!("{} needs a value", arguments[*index]))?;
    *index += 2;
    Ok(value)
}

impl BundleOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut artifact = None;
        let mut key = None;
        let mut output = None;
        let mut entry_offset = 0;
        let mut dependencies = Vec::new();
        let mut index = 0;
        while index < arguments.len() {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{} needs a value", arguments[index]))?;
            match arguments[index].as_str() {
                "--artifact" => artifact = Some(PathBuf::from(value)),
                "--key" => key = Some(PathBuf::from(value)),
                "--output" => output = Some(PathBuf::from(value)),
                "--entry-offset" => {
                    entry_offset = value
                        .parse()
                        .map_err(|_| "entry offset must be an integer".to_string())?
                }
                "--dependency" => dependencies.push(decode_content_id(value)?),
                other => return Err(format!("unknown bundle option `{other}`")),
            }
            index += 2;
        }
        Ok(Self {
            artifact: artifact.ok_or_else(|| "--artifact is required".to_string())?,
            key: key.ok_or_else(|| "--key is required".to_string())?,
            output: output.ok_or_else(|| "--output is required".to_string())?,
            entry_offset,
            dependencies,
        })
    }
}

struct PackageOptions {
    binary: String,
    manifest_path: Option<PathBuf>,
    package: Option<String>,
    target: String,
    release: bool,
    key: PathBuf,
    output: PathBuf,
    entry_offset: u64,
    dependencies: Vec<ContentId>,
    locked: bool,
    offline: bool,
    target_directory: Option<PathBuf>,
    app_manifest: Option<PathBuf>,
    debug_symbols: Option<PathBuf>,
    stripped_output: Option<PathBuf>,
}

impl PackageOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut binary = None;
        let mut manifest_path = None;
        let mut package = None;
        let mut target = "x86_64".to_string();
        let mut release = false;
        let mut key = None;
        let mut output = None;
        let mut entry_offset = 0;
        let mut dependencies = Vec::new();
        let mut locked = false;
        let mut offline = false;
        let mut target_directory = None;
        let mut app_manifest = None;
        let mut debug_symbols = None;
        let mut stripped_output = None;
        let mut index = 0;
        while index < arguments.len() {
            if arguments[index] == "--release" {
                release = true;
                index += 1;
                continue;
            }
            if arguments[index] == "--locked" {
                locked = true;
                index += 1;
                continue;
            }
            if arguments[index] == "--offline" {
                offline = true;
                index += 1;
                continue;
            }
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{} needs a value", arguments[index]))?;
            match arguments[index].as_str() {
                "--bin" => binary = Some(value.clone()),
                "--manifest-path" => manifest_path = Some(PathBuf::from(value)),
                "--package" => package = Some(value.clone()),
                "--target" => target = value.clone(),
                "--key" => key = Some(PathBuf::from(value)),
                "--output" => output = Some(PathBuf::from(value)),
                "--entry-offset" => {
                    entry_offset = value
                        .parse()
                        .map_err(|_| "entry offset must be an integer".to_string())?
                }
                "--dependency" => dependencies.push(decode_content_id(value)?),
                "--target-dir" => target_directory = Some(PathBuf::from(value)),
                "--app-manifest" => app_manifest = Some(PathBuf::from(value)),
                "--debug-symbols" => debug_symbols = Some(PathBuf::from(value)),
                "--stripped-output" => stripped_output = Some(PathBuf::from(value)),
                other => return Err(format!("unknown package option `{other}`")),
            }
            index += 2;
        }
        Ok(Self {
            binary: binary.ok_or_else(|| "--bin is required".to_string())?,
            manifest_path,
            package,
            target,
            release,
            key: key.ok_or_else(|| "--key is required".to_string())?,
            output: output.ok_or_else(|| "--output is required".to_string())?,
            entry_offset,
            dependencies,
            locked,
            offline,
            target_directory,
            app_manifest,
            debug_symbols,
            stripped_output,
        })
    }
}
