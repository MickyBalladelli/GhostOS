use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use synos_pkg::{SigningKey, bundle_size, encode_bundle};
use synos_system_model::ContentId;

const USAGE: &str = "\
cargo synos build [--target x86_64|aarch64] [--release] [cargo options]
cargo synos bundle --artifact PATH --key PATH --output PATH
    [--entry-offset BYTES] [--dependency SHA256]...
cargo synos package --bin NAME --key PATH --output PATH
    [--package NAME] [--target x86_64|aarch64] [--release]
    [--entry-offset BYTES] [--dependency SHA256]...

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
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
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
    let target = target_path(&options.target)?;
    let mut build_arguments = vec![
        "--target".into(),
        target.to_string_lossy().into_owned(),
        "--bin".into(),
        options.binary.clone(),
    ];
    if let Some(package) = &options.package {
        build_arguments.push("--package".into());
        build_arguments.push(package.clone());
    }
    if options.release {
        build_arguments.push("--release".into());
    }
    run_cargo_build(&build_arguments)?;

    let profile = if options.release { "release" } else { "debug" };
    let target_name = target
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "invalid target name".to_string())?;
    let target_directory = env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or(workspace_root()?.join("target"));
    let artifact = target_directory
        .join(target_name)
        .join(profile)
        .join(&options.binary);
    create_bundle(&BundleOptions {
        artifact,
        key: options.key,
        output: options.output,
        entry_offset: options.entry_offset,
        dependencies: options.dependencies,
    })
}

fn run_cargo_build(arguments: &[String]) -> Result<(), String> {
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let status = Command::new(cargo)
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["-Z", "build-std=core,alloc"])
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
    package: Option<String>,
    target: String,
    release: bool,
    key: PathBuf,
    output: PathBuf,
    entry_offset: u64,
    dependencies: Vec<ContentId>,
}

impl PackageOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut binary = None;
        let mut package = None;
        let mut target = "x86_64".to_string();
        let mut release = false;
        let mut key = None;
        let mut output = None;
        let mut entry_offset = 0;
        let mut dependencies = Vec::new();
        let mut index = 0;
        while index < arguments.len() {
            if arguments[index] == "--release" {
                release = true;
                index += 1;
                continue;
            }
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{} needs a value", arguments[index]))?;
            match arguments[index].as_str() {
                "--bin" => binary = Some(value.clone()),
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
                other => return Err(format!("unknown package option `{other}`")),
            }
            index += 2;
        }
        Ok(Self {
            binary: binary.ok_or_else(|| "--bin is required".to_string())?,
            package,
            target,
            release,
            key: key.ok_or_else(|| "--key is required".to_string())?,
            output: output.ok_or_else(|| "--output is required".to_string())?,
            entry_offset,
            dependencies,
        })
    }
}
