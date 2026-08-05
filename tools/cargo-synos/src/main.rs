use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use synos_compiler::{CompileRequest, Compiler, Target};
use synos_pkg::{SigningKey, bundle_size, encode_bundle};
use synos_system_model::ContentId;

const USAGE: &str = "\
cargo synos build [--target x86_64|aarch64] [--release] [cargo options]
cargo synos bundle --artifact PATH --key PATH --output PATH
    [--entry-offset BYTES] [--dependency SHA256]...
cargo synos package --bin NAME --key PATH --output PATH
    [--manifest-path PATH] [--package NAME] [--target x86_64|aarch64]
    [--release] [--locked] [--offline] [--target-dir PATH]
    [--entry-offset BYTES] [--dependency SHA256]...
cargo synos compile --manifest-path PATH --bin NAME
    [--target x86_64|aarch64] [--release] [--locked] [--offline]
    [--target-dir PATH]
cargo synos compile-all [--target x86_64|aarch64] [--release] [--target-dir PATH]
cargo synos reproduce [--target x86_64|aarch64] [--release] [--clean-root PATH]
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
        "bundle" => bundle(&arguments[1..]),
        "package" => package(&arguments[1..]),
        "compile" => compile(&arguments[1..]),
        "compile-all" => compile_all(&arguments[1..]),
        "reproduce" => reproduce(&arguments[1..]),
        "run" => run_program(&arguments[1..]),
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
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
        Ok(())
    } else {
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
    let expanded = expand_target(arguments)?;
    run_cargo_build(&expanded)
}

fn bundle(arguments: &[String]) -> Result<(), String> {
    let options = BundleOptions::parse(arguments)?;
    create_bundle(&options)
}

fn package(arguments: &[String]) -> Result<(), String> {
    let options = PackageOptions::parse(arguments)?;
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    let key = read_key(&options.key)?;
    let output = compiler
        .compile_and_bundle(
            &CompileRequest {
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
            },
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
    Ok(())
}

fn run_cargo_build(arguments: &[String]) -> Result<(), String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["-Z", "build-std=core,alloc", "-Z", "json-target-spec"])
        .arg("build")
        .args(arguments)
        .status()
        .map_err(|error| format!("could not start Cargo: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cross-build failed with {status}"))
    }
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

fn expand_target(arguments: &[String]) -> Result<Vec<String>, String> {
    let mut expanded = Vec::with_capacity(arguments.len() + 1);
    let mut index = 0;
    let mut found_target = false;
    while index < arguments.len() {
        if arguments[index] == "--target" {
            let target = arguments
                .get(index + 1)
                .ok_or_else(|| "--target needs a value".to_string())?;
            expanded.push("--target".into());
            expanded.push(target_path(target)?.to_string_lossy().into_owned());
            found_target = true;
            index += 2;
        } else {
            expanded.push(arguments[index].clone());
            index += 1;
        }
    }
    if !found_target {
        expanded.insert(0, target_path("x86_64")?.to_string_lossy().into_owned());
        expanded.insert(0, "--target".into());
    }
    Ok(expanded)
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

impl RunOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut manifest_path = None;
        let mut binary = None;
        let mut release = false;
        let mut program_arguments = Vec::new();
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
        })
    }
}
