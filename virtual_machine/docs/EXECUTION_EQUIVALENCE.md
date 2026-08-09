# Interpreter and translated execution equivalence

The interpreter and translated engine are equivalent when they start from the
same guest state and consume the same deterministic external inputs.
Equivalence is checked at the same retired-instruction count or at the first
architectural stopping event.

The required observable state is:

- Every CpuState field is identical.
- Every guest RAM byte is identical.
- An exception or VM execution error has the same CpuError, faulting RIP, CPU
  state, and memory state. Translation may decode ahead, but a later decode
  fault must not prevent earlier valid instructions from retiring.
- Port and MMIO reads and writes occur in the same order with the same address,
  width, and value. I/O instructions end translated blocks.
- Interrupts, timers, DMA completions, and host input use the same replay
  events when whole-VM execution is compared.

Translation cache contents, cache hit counts, profiles, hotness counters, and
host elapsed time are implementation details. They are not guest-visible and
are excluded from equivalence.

The current compiled form is portable decoded IR and uses the same instruction
executor as the interpreter. The contract still applies if a host-native JIT
is added later.

## Differential evidence

The bounded differential harness in tests/cpu_differential.rs runs identical
fixtures through Cpu::step and ExecutionEngine::execute. It compares:

- straight-line register, flag, stack, and full-RAM effects;
- a promoted hot loop, including final CPU and memory state;
- delayed decode faults, divide errors, and unsupported instructions;
- ordered port and MMIO reads and writes plus their resulting CPU state.

The harness uses fixed programs, fixed device responses, a 2 MiB RAM image,
and explicit instruction budgets. It has no host-time or random input.
