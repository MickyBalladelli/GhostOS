use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use synos_vm::devices::{ApicTrigger, InterruptController, LocalApic, PortBus};
use synos_vm::firmware::bios::BiosContext;
use synos_vm::{
    translate_input_bytes, Cpu, DiskImage, ExecutionEngine, ExecutionEngineConfig, LoopbackHub,
    LoopbackPort, MacAddress, Mmu, NetBackend, PAGE_SIZE,
};

const SCHEMA_VERSION: u32 = 1;
const CODE: u64 = 0x1000;
const DECODE_OPERATIONS: u64 = 200_000;
const TRANSLATION_BLOCKS: u64 = 20_000;
const TRANSLATION_BLOCK_INSTRUCTIONS: usize = 16;
const MEMORY_SIZE: usize = 4 * 1024 * 1024;
const MEMORY_PASSES: u64 = 4;
const INTERRUPT_OPERATIONS: u64 = 200_000;
const STORAGE_SECTORS: u64 = 256;
const NETWORK_PACKETS: u64 = 100_000;
const NETWORK_FRAME_BYTES: usize = 128;
const TERMINAL_INPUT_BYTES: usize = 4096;
const TERMINAL_PASSES: u64 = 10_000;

struct BenchmarkResult {
    name: &'static str,
    unit: &'static str,
    work_units: u64,
    elapsed: Duration,
    checksum: u64,
}

impl BenchmarkResult {
    fn print(&self) {
        let elapsed_ns = self.elapsed.as_nanos().max(1);
        let rate = u128::from(self.work_units)
            .saturating_mul(1_000_000_000)
            / elapsed_ns;
        println!(
            "{{\"schema\":{SCHEMA_VERSION},\"record\":\"result\",\"benchmark\":\"{}\",\"bounded\":true,\"unit\":\"{}\",\"work_units\":{},\"elapsed_ns\":{},\"rate_per_second\":{},\"checksum\":{}}}",
            self.name,
            self.unit,
            self.work_units,
            elapsed_ns,
            rate,
            self.checksum,
        )
    }
}

struct TemporaryFile {
    path: PathBuf,
}

impl TemporaryFile {
    fn create_raw(bytes: u64) -> Result<Self, String> {
        for sequence in 0..1024 {
            let path = std::env::temp_dir().join(format!(
                "synos-vm-bounded-bench-{}-{sequence}.raw",
                std::process::id(),
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    file.set_len(bytes)
                        .map_err(|error| format!("size benchmark disk: {error}"))?;
                    return Ok(Self { path })
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("create benchmark disk: {error}")),
            }
        }
        Err("cannot allocate a unique benchmark disk path".to_string())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("synos-vm bounded benchmark failed: {error}");
        std::process::exit(1)
    }
}

fn run() -> Result<(), String> {
    print_metadata();
    for result in [
        benchmark_decode()?,
        benchmark_translation()?,
        benchmark_memory()?,
        benchmark_interrupts()?,
        benchmark_storage()?,
        benchmark_network()?,
        benchmark_terminal()?,
    ] {
        result.print()
    }
    Ok(())
}

fn print_metadata() {
    let revision = std::env::var("SYNOS_BENCH_REVISION").unwrap_or_else(|_| "unknown".to_string());
    let logical_cpus = std::thread::available_parallelism().map_or(1, |value| value.get());
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!(
        "{{\"schema\":{SCHEMA_VERSION},\"record\":\"metadata\",\"suite\":\"synos-vm-bounded\",\"workload_version\":1,\"command\":\"cargo bench -p synos-vm --bench bounded\",\"crate_version\":\"{}\",\"revision\":\"{}\",\"target_os\":\"{}\",\"target_arch\":\"{}\",\"logical_cpus\":{},\"profile\":\"{}\"}}",
        env!("CARGO_PKG_VERSION"),
        json_escape(&revision),
        std::env::consts::OS,
        std::env::consts::ARCH,
        logical_cpus,
        profile,
    )
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn benchmark_decode() -> Result<BenchmarkResult, String> {
    let instruction = [0x48, 0x8B, 0x84, 0x8D, 0x78, 0x56, 0x34, 0x12];
    let mut mmu = Mmu::new(2 * 1024 * 1024);
    mmu.write_phys(CODE, &instruction)
        .map_err(|error| format!("prepare decode benchmark: {error:?}"))?;
    let cpu = Cpu::new();
    let mut checksum = 0u64;

    let started = Instant::now();
    for _ in 0..DECODE_OPERATIONS {
        let decoded = cpu
            .decode_instruction(black_box(CODE), black_box(&mmu))
            .map_err(|error| format!("decode benchmark instruction: {error:?}"))?;
        checksum = checksum.wrapping_add(
            decoded.next_ip
                ^ u64::from(decoded.opcode)
                ^ decoded.operands.len() as u64,
        );
        black_box(&decoded);
    }
    let elapsed = started.elapsed();

    Ok(BenchmarkResult {
        name: "decode",
        unit: "instructions",
        work_units: DECODE_OPERATIONS,
        elapsed,
        checksum,
    })
}

fn benchmark_translation() -> Result<BenchmarkResult, String> {
    let program = [0x90u8; TRANSLATION_BLOCK_INSTRUCTIONS];
    let mut mmu = Mmu::new(2 * 1024 * 1024);
    mmu.write_phys(CODE, &program)
        .map_err(|error| format!("prepare translation benchmark: {error:?}"))?;
    let mut cpu = Cpu::new();
    let mut engine = ExecutionEngine::with_config(ExecutionEngineConfig {
        max_block_instructions: TRANSLATION_BLOCK_INSTRUCTIONS,
        hot_threshold: u64::MAX,
        cache_capacity: 1,
        enable_jit: false,
        enable_profiling: false,
    });
    let mut interrupts = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();
    let mut retired = 0u64;
    let mut checksum = 0u64;

    let started = Instant::now();
    for _ in 0..TRANSLATION_BLOCKS {
        engine.clear_cache();
        cpu.set_rip(CODE);
        let executed = engine
            .execute(
                &mut cpu,
                &mut mmu,
                &mut interrupts,
                &mut ports,
                &mut bios,
                TRANSLATION_BLOCK_INSTRUCTIONS,
            )
            .map_err(|error| format!("translate benchmark block: {error:?}"))?;
        if executed != TRANSLATION_BLOCK_INSTRUCTIONS {
            return Err(format!(
                "translation benchmark retired {executed} instructions, expected {TRANSLATION_BLOCK_INSTRUCTIONS}"
            ))
        }
        retired += executed as u64;
        checksum = checksum.wrapping_add(cpu.state.rip);
        black_box(cpu.state.rip);
    }
    let elapsed = started.elapsed();
    checksum ^= engine.stats().translated_blocks;

    Ok(BenchmarkResult {
        name: "translation",
        unit: "instructions",
        work_units: retired,
        elapsed,
        checksum,
    })
}

fn benchmark_memory() -> Result<BenchmarkResult, String> {
    let mut mmu = Mmu::new(MEMORY_SIZE);
    let mut page = [0u8; PAGE_SIZE];
    for (index, byte) in page.iter_mut().enumerate() {
        *byte = (index % 251) as u8
    }
    let pages = MEMORY_SIZE / PAGE_SIZE;
    let mut checksum = 0u64;

    let started = Instant::now();
    for _ in 0..MEMORY_PASSES {
        for page_index in 0..pages {
            let address = (page_index * PAGE_SIZE) as u64;
            mmu.write_phys(address, black_box(&page))
                .map_err(|error| format!("memory benchmark write: {error:?}"))?;
        }
        for page_index in 0..pages {
            let address = (page_index * PAGE_SIZE) as u64;
            let bytes = mmu
                .read_phys(address, PAGE_SIZE)
                .map_err(|error| format!("memory benchmark read: {error:?}"))?;
            checksum = checksum
                .wrapping_add(u64::from(bytes[0]))
                .wrapping_add(u64::from(bytes[PAGE_SIZE - 1]));
            black_box(bytes);
        }
    }
    let elapsed = started.elapsed();
    let work_units = (MEMORY_SIZE as u64)
        .saturating_mul(MEMORY_PASSES)
        .saturating_mul(2);

    Ok(BenchmarkResult {
        name: "memory",
        unit: "bytes",
        work_units,
        elapsed,
        checksum,
    })
}

fn benchmark_interrupts() -> Result<BenchmarkResult, String> {
    let mut apic = LocalApic::new(0);
    let mut checksum = 0u64;

    let started = Instant::now();
    for index in 0..INTERRUPT_OPERATIONS {
        let vector = 0x40 + (index & 0x0F) as u8;
        apic.signal(vector, ApicTrigger::Edge);
        let pending = apic
            .pending_vector()
            .ok_or_else(|| "interrupt benchmark lost a pending vector".to_string())?;
        apic.accept_pending(pending);
        apic.eoi();
        checksum = checksum.wrapping_add(u64::from(pending));
        black_box(pending);
    }
    let elapsed = started.elapsed();

    Ok(BenchmarkResult {
        name: "interrupt",
        unit: "interrupts",
        work_units: INTERRUPT_OPERATIONS,
        elapsed,
        checksum,
    })
}

fn benchmark_storage() -> Result<BenchmarkResult, String> {
    let temporary = TemporaryFile::create_raw(STORAGE_SECTORS * 512)?;
    let mut image = DiskImage::open(temporary.path())
        .map_err(|error| format!("open benchmark disk: {error}"))?;
    let mut checksum = 0u64;

    let started = Instant::now();
    for lba in 0..STORAGE_SECTORS {
        let mut sector = [0u8; 512];
        sector.fill((lba % 251) as u8);
        image
            .write_sector(lba, black_box(&sector))
            .map_err(|error| format!("storage benchmark write: {error}"))?;
    }
    for lba in 0..STORAGE_SECTORS {
        let mut sector = [0u8; 512];
        image
            .read_sector(lba, &mut sector)
            .map_err(|error| format!("storage benchmark read: {error}"))?;
        let expected = (lba % 251) as u8;
        if sector.iter().any(|byte| *byte != expected) {
            return Err(format!("storage benchmark data mismatch at LBA {lba}"))
        }
        checksum = checksum.wrapping_add(u64::from(sector[0]));
        black_box(sector);
    }
    let elapsed = started.elapsed();

    Ok(BenchmarkResult {
        name: "storage",
        unit: "bytes",
        work_units: STORAGE_SECTORS * 512 * 2,
        elapsed,
        checksum,
    })
}

fn benchmark_network() -> Result<BenchmarkResult, String> {
    let hub = Rc::new(RefCell::new(LoopbackHub::new()));
    let left_mac = MacAddress::synos_default(0x10);
    let right_mac = MacAddress::synos_default(0x11);
    let mut left = LoopbackPort::new(hub.clone(), 0, left_mac);
    let mut right = LoopbackPort::new(hub, 1, right_mac);
    let mut frame = vec![0x5A; NETWORK_FRAME_BYTES];
    frame[..6].copy_from_slice(&right_mac.to_bytes());
    frame[6..12].copy_from_slice(&left_mac.to_bytes());
    frame[12..14].copy_from_slice(&0x88B5u16.to_be_bytes());
    let mut checksum = 0u64;

    let started = Instant::now();
    for _ in 0..NETWORK_PACKETS {
        left
            .transmit(black_box(&frame))
            .map_err(|error| format!("network benchmark transmit: {error}"))?;
        let packet = right
            .receive()
            .map_err(|error| format!("network benchmark receive: {error}"))?
            .ok_or_else(|| "network benchmark packet was not delivered".to_string())?;
        checksum = checksum
            .wrapping_add(packet.len() as u64)
            .wrapping_add(u64::from(packet[NETWORK_FRAME_BYTES - 1]));
        black_box(packet);
    }
    let elapsed = started.elapsed();

    Ok(BenchmarkResult {
        name: "network",
        unit: "packets",
        work_units: NETWORK_PACKETS,
        elapsed,
        checksum,
    })
}

fn benchmark_terminal() -> Result<BenchmarkResult, String> {
    let mut input = vec![0u8; TERMINAL_INPUT_BYTES];
    for (index, byte) in input.iter_mut().enumerate() {
        *byte = if index % 31 == 0 {
            0x7F
        } else {
            0x20 + (index % 95) as u8
        }
    }
    let mut checksum = 0u64;

    let started = Instant::now();
    for _ in 0..TERMINAL_PASSES {
        let translated = translate_input_bytes(black_box(&input));
        checksum = checksum
            .wrapping_add(translated.len() as u64)
            .wrapping_add(u64::from(translated[0]))
            .wrapping_add(u64::from(translated[TERMINAL_INPUT_BYTES - 1]));
        black_box(translated);
    }
    let elapsed = started.elapsed();

    Ok(BenchmarkResult {
        name: "terminal",
        unit: "bytes",
        work_units: TERMINAL_INPUT_BYTES as u64 * TERMINAL_PASSES,
        elapsed,
        checksum,
    })
}
