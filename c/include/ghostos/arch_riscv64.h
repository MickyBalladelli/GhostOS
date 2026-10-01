#ifndef GHOSTOS_ARCH_RISCV64_H
#define GHOSTOS_ARCH_RISCV64_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_RISCV64_TABLE_FRAME_COUNT 6

typedef struct {
    uint64_t general[16];
    uint64_t instruction_pointer, stack_pointer, flags, fault_address;
} ghostos_riscv64_register_state;

typedef struct {
    uint64_t registers[32];
    uint64_t sepc, sstatus, scause, stval;
} ghostos_riscv64_trap_frame;

typedef struct {
    void *context;
    void (*syscall_dispatch)(void *context, uint64_t request, uint64_t response);
    void (*capture_exception)(void *context, ghostos_riscv64_register_state registers, uint64_t fault_address, uint32_t status, uint16_t cause);
    void (*log_exception)(void *context, uint64_t cause, bool interrupt);
    void (*halt)(void *context);
} ghostos_riscv64_trap_hooks;

typedef struct { uint64_t stack_pointer, instruction_pointer; } ghostos_riscv64_user_context;

bool ghostos_riscv64_install_root(const uint64_t tables[GHOSTOS_RISCV64_TABLE_FRAME_COUNT], uint64_t physical_offset);
void ghostos_riscv64_activate_root(uint64_t root);
void ghostos_riscv64_invalidate_page(uint64_t address);
void ghostos_riscv64_invalidate_all(void);
void ghostos_riscv64_invalidate_range(uint64_t start, uint64_t length);
void ghostos_riscv64_set_core_isolated(uint8_t cpu, bool isolated);
bool ghostos_riscv64_core_isolated(uint8_t cpu);
void ghostos_riscv64_interrupts_disable(void);
void ghostos_riscv64_interrupts_enable(void);
bool ghostos_riscv64_interrupts_init(void);
uint8_t ghostos_riscv64_current_cpu(void);
bool ghostos_riscv64_send_ipi(uint8_t target, uint8_t vector);
void ghostos_riscv64_end_of_interrupt(void);
void ghostos_riscv64_set_trap_hooks(ghostos_riscv64_trap_hooks hooks);
void ghostos_riscv64_trap_dispatch(ghostos_riscv64_trap_frame *frame);
ghostos_riscv64_register_state ghostos_riscv64_capture_registers(uint64_t fault_address);
_Noreturn void ghostos_riscv64_enter_user(ghostos_riscv64_user_context context, uint64_t root);
void ghostos_riscv64_idle(void);
_Noreturn void ghostos_riscv64_halt(void);

#endif
