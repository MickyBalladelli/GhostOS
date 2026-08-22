//! Repeated in-process checks for VM state that must be empty after teardown.

use std::io::Cursor;
use std::time::{Duration, Instant};

use ghostos_vm::firmware::bios::BiosContext;
use ghostos_vm::{
    Cpu, ExecutionEngine, ExecutionEngineConfig, InterruptController, Mmu, PortBus,
    TerminalSession,
};

const CODE: u64 = 0x1000;

#[test]
fn soak_translation_cache_and_terminal_state_release() {
    let repeats = std::env::var("GHOSTOS_VM_SOAK_INNER_RUNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(32);

    for _ in 0..repeats {
        let mut mmu = Mmu::new(2 * 1024 * 1024);
        mmu.write_phys(CODE, &[0xF4]).expect("write soak program");
        let mut cpu = Cpu::new();
        cpu.set_rip(CODE);
        let mut interrupts = InterruptController::new();
        let mut ports = PortBus::new();
        let mut bios = BiosContext::new();
        let mut engine = ExecutionEngine::with_config(ExecutionEngineConfig {
            cache_capacity: 8,
            ..ExecutionEngineConfig::default()
        });
        engine
            .execute(&mut cpu, &mut mmu, &mut interrupts, &mut ports, &mut bios, 1)
            .expect("execute soak program");
        assert!(engine.cache_len() <= engine.config().cache_capacity);
        engine.clear_cache();
        assert_eq!(engine.cache_len(), 0, "translation cache survived teardown");
        engine.reset();
        assert_eq!(engine.cache_len(), 0, "translation cache survived reset");

        let session = TerminalSession::new_with_io(Cursor::new(vec![b'x']), Vec::<u8>::new());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let input = session.poll().expect("poll soak terminal");
            if !input.bytes.is_empty() {
                break
            }
            assert!(Instant::now() < deadline, "terminal input did not arrive");
            std::thread::yield_now();
        }
        drop(session);
    }
}
