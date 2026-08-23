//! Differential compatibility checks for the five externally visible paths.
//!
//! The reference sides in this file are deliberately small and independent:
//! std collections model filesystem and packet state, a plain VT model checks
//! terminal bytes, published PC/UEFI facts check firmware, and a literal wire
//! encoder checks ABI serialization.

use std::collections::{BTreeMap, VecDeque};

use ghostos_abi::{RpcFrameHeader, RpcMethod, RpcStatus, RPC_FRAME_HEADER_BYTES};
use ghostos_netd::PacketQueue as DriverPacketQueue;
use ghostos_ghostfs::SynFs;
use ghostos_test_support::differential::assert_no_divergence;
use ghostos_webterm::{Cell, Terminal};
use ghostos_vm::firmware::bios::{Bios, BiosState};
use ghostos_vm::firmware::uefi::{UefiContext, UefiState, UEFI_TABLES_BASE};
use ghostos_vm::{CpuState, Mmu};

#[derive(Clone, Debug)]
enum FilesystemOp {
    Write { path: String, contents: Vec<u8> },
    Delete { path: String },
}

fn compare_filesystem(operations: &[FilesystemOp]) -> Option<String> {
    let mut filesystem = SynFs::<32>::new();
    let mut reference = BTreeMap::<String, Vec<u8>>::new();
    let paths = ["/alpha", "/beta", "/gamma"];

    for operation in operations {
        match operation {
            FilesystemOp::Write { path, contents } => {
                if filesystem.write(path, contents).is_err() {
                    return Some(format!("write rejected for {path}"));
                }
                reference.insert(path.clone(), contents.clone());
            }
            FilesystemOp::Delete { path } => {
                let expected = reference.remove(path).is_some();
                let actual = filesystem.delete(path).is_ok();
                if actual != expected {
                    return Some(format!("delete result for {path}: expected {expected}, got {actual}"));
                }
            }
        }

        for path in paths {
            let expected = reference.get(path);
            let actual = filesystem.lookup(path);
            match (expected, actual) {
                (Some(contents), Ok(file)) => {
                    let mut bytes = vec![0; file.size as usize];
                    let read = match filesystem.read(path, &mut bytes) {
                        Ok(read) => read,
                        Err(error) => return Some(format!("read rejected for {path}: {error:?}")),
                    };
                    bytes.truncate(read.bytes_read);
                    if bytes != *contents {
                        return Some(format!("contents differ for {path}"));
                    }
                }
                (None, Err(_)) => {}
                (Some(_), Err(error)) => return Some(format!("lookup rejected for {path}: {error:?}")),
                (None, Ok(_)) => return Some(format!("deleted path remains visible: {path}")),
            }
        }
    }
    None
}

#[test]
fn filesystem_matches_independent_reference() {
    let operations = vec![
        FilesystemOp::Write { path: "/alpha".into(), contents: b"one".to_vec() },
        FilesystemOp::Write { path: "/beta".into(), contents: b"two".to_vec() },
        FilesystemOp::Write { path: "/alpha".into(), contents: b"updated".to_vec() },
        FilesystemOp::Delete { path: "/beta".into() },
        FilesystemOp::Write { path: "/gamma".into(), contents: b"three".to_vec() },
    ];
    assert_no_divergence(
        "filesystem",
        &operations,
        "GhostFS is authoritative for versioning and path rules; the BTreeMap models only latest visible contents.",
        compare_filesystem,
    );
}

#[derive(Clone, Debug)]
enum NetworkOp {
    Push(Vec<u8>),
    Pop,
}

fn compare_network(operations: &[NetworkOp]) -> Option<String> {
    const CAPACITY: usize = 3;
    const MTU: usize = 8;
    let mut queue = DriverPacketQueue::<CAPACITY, MTU>::new();
    let mut reference = VecDeque::<Vec<u8>>::new();

    for operation in operations {
        match operation {
            NetworkOp::Push(packet) => {
                let actual = match queue.reserve() {
                    Ok(mut writer) if packet.len() <= MTU => {
                        writer.buffer()[..packet.len()].copy_from_slice(packet);
                        writer.commit(packet.len()).is_ok()
                    }
                    Ok(writer) => writer.commit(packet.len()).is_ok(),
                    Err(_) => false,
                };
                let expected = packet.len() <= MTU && reference.len() < CAPACITY;
                if actual != expected {
                    return Some(format!("push of {packet:?}: expected {expected}, got {actual}"));
                }
                if expected {
                    reference.push_back(packet.clone());
                }
            }
            NetworkOp::Pop => {
                let actual = queue.dequeue().ok().map(|reader| reader.frame().to_vec());
                let expected = reference.pop_front();
                if actual != expected {
                    return Some(format!("pop: expected {expected:?}, got {actual:?}"));
                }
            }
        }
        if queue.pending() != reference.len() || queue.available() != CAPACITY - reference.len() {
            return Some("queue depth differs from VecDeque reference".into());
        }
    }
    None
}

#[test]
fn network_queue_matches_independent_reference() {
    let operations = vec![
        NetworkOp::Push(vec![1, 2]),
        NetworkOp::Push(vec![3, 4, 5]),
        NetworkOp::Pop,
        NetworkOp::Push(vec![6, 7, 8]),
        NetworkOp::Push(vec![9]),
        NetworkOp::Push(vec![10]),
        NetworkOp::Push(vec![11]),
        NetworkOp::Pop,
        NetworkOp::Pop,
        NetworkOp::Pop,
    ];
    assert_no_divergence(
        "network",
        &operations,
        "The fixed packet queue owns bounded admission and FIFO order; VecDeque models the independent queue contract.",
        compare_network,
    );
}

fn reference_terminal(bytes: &[u8]) -> ([[Cell; 8]; 3], (usize, usize)) {
    let mut cells = [[Cell::EMPTY; 8]; 3];
    let mut row: usize = 0;
    let mut column: usize = 0;
    let mut wrap_pending = false;
    for byte in bytes {
        match byte {
            b'\r' => {
                column = 0;
                wrap_pending = false;
            }
            b'\n' => {
                column = 0;
                row = (row + 1).min(2);
                wrap_pending = false;
            }
            0x08 => {
                column = column.saturating_sub(1);
                wrap_pending = false;
            }
            0x20..=0x7e => {
                if wrap_pending {
                    column = 0;
                    row = (row + 1).min(2);
                    wrap_pending = false;
                }
                cells[row][column].glyph = u32::from(*byte);
                if column + 1 == 8 {
                    wrap_pending = true;
                } else {
                    column += 1;
                }
            }
            _ => {}
        }
    }
    (cells, (row, column))
}

fn compare_terminal(bytes: &[u8]) -> Option<String> {
    let (expected_cells, expected_cursor) = reference_terminal(bytes);
    let mut terminal = Terminal::<8, 3>::new().ok()?;
    terminal.write(bytes);
    for row in 0..3 {
        if terminal.row(row)? != &expected_cells[row] {
            return Some(format!("row {row} differs"));
        }
    }
    let cursor = terminal.cursor();
    if (cursor.row as usize, cursor.column as usize) != expected_cursor {
        return Some(format!("cursor differs: expected {expected_cursor:?}, got {cursor:?}"));
    }
    None
}

#[test]
fn terminal_bytes_match_independent_vt_reference() {
    let input = b"abc\nxy\rZ\x08Q12345678A".to_vec();
    assert_no_divergence(
        "terminal",
        &input,
        "The web terminal owns VT parsing; the reference covers printable bytes, CR, LF, backspace, and bounded wrap.",
        |candidate| compare_terminal(candidate),
    );
}

#[test]
fn firmware_facts_match_independent_platform_reference() {
    const BIOS_SIGNATURE: [u8; 2] = [0x55, 0xaa];
    const BIOS_EQUIPMENT: u8 = 0x21;
    const UEFI_SYSTEM_SIGNATURE: u64 = 0x5459_5353_2049_4249;

    let mut bios = Bios::new();
    let mut bios_mmu = Mmu::new(8 * 1024 * 1024);
    let mut bios_cpu = CpuState::default();
    bios.post(&mut bios_mmu, &mut bios_cpu).expect("BIOS POST");
    assert_eq!(bios.context.state, BiosState::Initialized);
    assert_eq!(bios_mmu.read_phys(0xFFFFE, 2).expect("BIOS signature"), BIOS_SIGNATURE);
    assert_eq!(bios_mmu.read_phys(0x410, 1).expect("BDA equipment"), [BIOS_EQUIPMENT]);

    let mut uefi = UefiContext::new();
    uefi.set_memory_size(64 * 1024 * 1024);
    let mut uefi_mmu = Mmu::new(64 * 1024 * 1024);
    let mut uefi_cpu = CpuState::default();
    uefi.init(&mut uefi_mmu, &mut uefi_cpu).expect("UEFI init");
    assert_eq!(uefi.state, UefiState::Initialized);
    assert!(uefi_cpu.halted);
    assert_eq!(uefi_mmu.read_u64(UEFI_TABLES_BASE).expect("UEFI system table"), UEFI_SYSTEM_SIGNATURE);
}

#[derive(Clone, Copy, Debug)]
struct WireCase {
    method: RpcMethod,
    flags: u16,
    request_id: u64,
    payload_bytes: u32,
    status: RpcStatus,
}

fn reference_wire(case: WireCase) -> [u8; RPC_FRAME_HEADER_BYTES] {
    let mut output = [0; RPC_FRAME_HEADER_BYTES];
    output[0..4].copy_from_slice(b"SYRP");
    output[4] = 1;
    output[5] = case.method as u8;
    output[6..8].copy_from_slice(&case.flags.to_be_bytes());
    output[8..16].copy_from_slice(&case.request_id.to_be_bytes());
    output[16..20].copy_from_slice(&case.payload_bytes.to_be_bytes());
    output[20..22].copy_from_slice(&(case.status as u16).to_be_bytes());
    output
}

fn compare_serialization(cases: &[WireCase]) -> Option<String> {
    for case in cases {
        let header = RpcFrameHeader {
            method: case.method,
            flags: case.flags,
            request_id: case.request_id,
            payload_bytes: case.payload_bytes,
            status: case.status,
        };
        let mut actual = [0; RPC_FRAME_HEADER_BYTES];
        if header.encode(&mut actual).is_err() {
            return Some(format!("valid wire case rejected: {case:?}"));
        }
        let expected = reference_wire(*case);
        if actual != expected {
            return Some(format!("encoded bytes differ for {case:?}"));
        }
    }
    None
}

#[test]
fn serialization_matches_independent_wire_reference() {
    let cases = vec![
        WireCase { method: RpcMethod::ClusterState, flags: 0, request_id: 1, payload_bytes: 0, status: RpcStatus::Ok },
        WireCase { method: RpcMethod::SubmitJob, flags: 1, request_id: 0x0102_0304_0506_0708, payload_bytes: 128, status: RpcStatus::Busy },
        WireCase { method: RpcMethod::Poll, flags: 0, request_id: u64::MAX, payload_bytes: 4096 - RPC_FRAME_HEADER_BYTES as u32, status: RpcStatus::ProtocolMismatch },
    ];
    assert_no_divergence(
        "serialization",
        &cases,
        "The generated ABI owns validation; the literal encoder documents the stable big-endian wire contract.",
        compare_serialization,
    );
}
