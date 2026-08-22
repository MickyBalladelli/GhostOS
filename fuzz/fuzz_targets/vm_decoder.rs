#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_vm::{Cpu, Mmu};

fuzz_target!(|data: &[u8]| {
    let mut mmu = Mmu::new(64 * 1024);
    let bytes = &data[..data.len().min(4096)];
    if mmu.write_bytes(0, bytes).is_ok() {
        let mut cpu = Cpu::new();
        cpu.state.rip = 0;
        let _ = cpu.decode_instruction(0, &mmu);
    }
});
