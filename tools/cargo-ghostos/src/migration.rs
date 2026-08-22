use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use ghostos_ghostfs::{
    BackgroundIoLimit, FormatMigrationPhase, SynFs, Error as SynFsError,
    VOLUME_FORMAT_VERSION, SYSTEM_VOLUME_BLOCKS,
};

const STAGING_SUFFIX: &str = ".ghostos-migrate.stage";

type SystemFilesystem = SynFs<SYSTEM_VOLUME_BLOCKS>;

pub fn run(arguments: &[String]) -> Result<(), String> {
    let operation = arguments
        .first()
        .map(String::as_str)
        .ok_or_else(|| "migrate needs inspect or system-state".to_string())?;
    match operation {
        "inspect" => inspect(&arguments[1..]),
        "system-state" | "ghostfs" => migrate_system_state(&arguments[1..]),
        other => Err(format!("unknown migrate operation `{other}`")),
    }
}

fn inspect(arguments: &[String]) -> Result<(), String> {
    let options = InspectOptions::parse(arguments)?;
    let image = read_image(&options.input)?;
    let filesystem = load_system_filesystem(&image, &options.input)?;
    let diagnostics = filesystem
        .diagnostics()
        .map_err(|error| format_ghostfs_error("inspect", error))?;

    if options.json {
        println!(
            "{{\"command\":\"migrate inspect\",\"input\":{},\"format\":{},\"target_format\":{},\"generation\":{},\"used_blocks\":{},\"capacity_blocks\":{},\"files\":{}}}",
            json_string(&options.input.display().to_string()),
            filesystem.format_version(),
            VOLUME_FORMAT_VERSION,
            filesystem.generation(),
            diagnostics.allocated_blocks,
            diagnostics.capacity_blocks,
            diagnostics.file_count
        )
    } else {
        println!("system state: {}", options.input.display());
        println!("format: {}", filesystem.format_version());
        println!("migration target: {}", VOLUME_FORMAT_VERSION);
        println!("generation: {}", filesystem.generation());
        println!("blocks: {}/{}", diagnostics.allocated_blocks, diagnostics.capacity_blocks);
        println!("files: {}", diagnostics.file_count);
        if filesystem.format_version() == VOLUME_FORMAT_VERSION {
            println!("status: current")
        } else {
            println!("status: migration available")
        }
    }
    Ok(())
}

fn migrate_system_state(arguments: &[String]) -> Result<(), String> {
    let options = MigrationOptions::parse(arguments)?;
    let input = canonical_input(&options.input)?;
    let output = options.output;
    reject_same_path(&input, &output)?;

    let staging = staging_path(&output);
    let (mut image, source_format) = if staging.exists() {
        let mut staged = read_image(&staging)?;
        let filesystem = load_system_filesystem(&staged, &staging)?;
        if filesystem.format_version() == VOLUME_FORMAT_VERSION {
            publish_staging(&staging, &output)?;
            println!("published already-complete migration to {}", output.display());
            return Ok(())
        }
        if SynFs::<SYSTEM_VOLUME_BLOCKS>::resume_format_migration(&mut staged)
            .map_err(|error| format_ghostfs_error("resume migration", error))?
            .is_none()
        {
            return Err(format!(
                "staging image {} has no resumable migration",
                staging.display()
            ))
        }
        (staged, filesystem.format_version())
    } else {
        let source = read_image(&input)?;
        let filesystem = load_system_filesystem(&source, &input)?;
        if filesystem.format_version() == VOLUME_FORMAT_VERSION {
            return Err("system state is already at the current format".into())
        }
        let mut staged = source;
        SynFs::<SYSTEM_VOLUME_BLOCKS>::begin_format_migration(
            &mut staged,
            VOLUME_FORMAT_VERSION,
        )
        .map_err(|error| format_ghostfs_error("begin migration", error))?;
        persist_image(&staging, &staged)?;
        (staged, filesystem.format_version())
    };

    let mut migration = SynFs::<SYSTEM_VOLUME_BLOCKS>::resume_format_migration(&mut image)
        .map_err(|error| format_ghostfs_error("resume migration", error))?
        .ok_or_else(|| "could not find the migration journal".to_string())?;

    while migration.progress().phase != FormatMigrationPhase::Committed {
        let progress = migration
            .step(&mut image, BackgroundIoLimit::new(options.io_blocks))
            .map_err(|error| format_ghostfs_error("migration step", error))?;
        persist_image(&staging, &image)?;
        if !options.json {
            println!(
                "migration: {:?}, {}/{} blocks",
                progress.phase, progress.blocks_copied, progress.total_blocks
            )
        }
    }

    let migrated = load_system_filesystem(&image, &staging)?;
    if migrated.format_version() != VOLUME_FORMAT_VERSION {
        return Err("migration completed without publishing the current format".into())
    }
    persist_image(&staging, &image)?;
    publish_staging(&staging, &output)?;

    if options.json {
        println!(
            "{{\"command\":\"migrate system-state\",\"input\":{},\"output\":{},\"from\":{},\"to\":{},\"generation\":{}}}",
            json_string(&input.display().to_string()),
            json_string(&output.display().to_string()),
            source_format,
            VOLUME_FORMAT_VERSION,
            migrated.generation()
        )
    } else {
        println!("migrated {} to {}", input.display(), output.display())
    }
    Ok(())
}

struct InspectOptions {
    input: PathBuf,
    json: bool,
}

impl InspectOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut input = None;
        let mut json = false;
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--input" => input = Some(PathBuf::from(required_value(arguments, &mut index)?)),
                "--json" => {
                    json = true;
                    index += 1;
                }
                other => return Err(format!("unknown migrate inspect option `{other}`")),
            }
        }
        Ok(Self {
            input: input.ok_or_else(|| "--input is required".to_string())?,
            json,
        })
    }
}

struct MigrationOptions {
    input: PathBuf,
    output: PathBuf,
    io_blocks: usize,
    json: bool,
}

impl MigrationOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut input = None;
        let mut output = None;
        let mut io_blocks = 8;
        let mut json = false;
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--input" => input = Some(PathBuf::from(required_value(arguments, &mut index)?)),
                "--output" => output = Some(PathBuf::from(required_value(arguments, &mut index)?)),
                "--io-blocks" => {
                    io_blocks = required_value(arguments, &mut index)?
                        .parse()
                        .map_err(|_| "--io-blocks must be a positive integer".to_string())?;
                    if io_blocks == 0 {
                        return Err("--io-blocks must be a positive integer".into())
                    }
                }
                "--json" => {
                    json = true;
                    index += 1;
                }
                other => return Err(format!("unknown migrate system-state option `{other}`")),
            }
        }
        Ok(Self {
            input: input.ok_or_else(|| "--input is required".to_string())?,
            output: output.ok_or_else(|| "--output is required".to_string())?,
            io_blocks,
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

fn read_image(path: &Path) -> Result<Vec<u8>, String> {
    let image = fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let expected = SystemFilesystem::volume_bytes();
    if image.len() != expected {
        return Err(format!(
            "{} is {} bytes; system state needs exactly {} bytes",
            path.display(),
            image.len(),
            expected
        ))
    }
    Ok(image)
}

fn load_system_filesystem(image: &[u8], path: &Path) -> Result<SystemFilesystem, String> {
    SystemFilesystem::load(image)
        .map_err(|error| format!("{} is not a valid GhostFS system state: {error:?}", path.display()))
}

fn canonical_input(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|error| format!("could not resolve {}: {error}", path.display()))
}

fn reject_same_path(input: &Path, output: &Path) -> Result<(), String> {
    if output.exists() {
        let resolved = fs::canonicalize(output)
            .map_err(|error| format!("could not resolve {}: {error}", output.display()))?;
        if resolved == input {
            return Err("input and output must be different paths".into())
        }
        return Err(format!(
            "output {} already exists; choose a new path",
            output.display()
        ))
    }
    if let Some(parent) = output.parent() {
        if !parent.exists() {
            return Err(format!("output directory {} does not exist", parent.display()))
        }
    }
    Ok(())
}

fn staging_path(output: &Path) -> PathBuf {
    let mut staging = output.as_os_str().to_os_string();
    staging.push(STAGING_SUFFIX);
    PathBuf::from(staging)
}

fn persist_image(path: &Path, image: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    file.write_all(image)
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("could not sync {}: {error}", path.display()))
}

fn publish_staging(staging: &Path, output: &Path) -> Result<(), String> {
    if output.exists() {
        return Err(format!("output {} already exists; refusing overwrite", output.display()))
    }
    fs::rename(staging, output)
        .map_err(|error| format!("could not publish {} as {}: {error}", staging.display(), output.display()))
}

fn format_ghostfs_error(operation: &str, error: SynFsError) -> String {
    format!("{operation} failed: {error:?}")
}

fn json_string(value: &str) -> String {
    let mut output = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => output.push_str(&format!("\\u{:04x}", character as u32)),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
