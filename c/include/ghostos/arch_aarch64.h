#ifndef GHOSTOS_ARCH_AARCH64_H
#define GHOSTOS_ARCH_AARCH64_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_AARCH64_TABLE_FRAME_COUNT 6

typedef struct {
    uint64_t general[16];
    uint64_t instruction_pointer;
    uint64_t stack_pointer;
    uint64_t flags;
    uint64_t fault_address;
} ghostos_aarch64_register_state;

typedef struct {
    uint64_t registers[31];
    uint64_t elr, spsr, esr, far;
} ghostos_aarch64_exception_frame;

typedef struct {
    void *context;
    void (*syscall_dispatch)(void *context, uint64_t request, uint64_t response);
    void (*capture_exception)(void *context, ghostos_aarch64_register_state registers, uint64_t fault_address, uint32_t status, uint16_t exception_class);
    void (*log_exception)(void *context, uint8_t exception_class);
    void (*halt)(void *context);
} ghostos_aarch64_exception_hooks;

typedef struct { uint64_t stack_pointer, instruction_pointer; } ghostos_aarch64_user_context;

bool ghostos_aarch64_install_root(const uint64_t tables[GHOSTOS_AARCH64_TABLE_FRAME_COUNT], uint64_t physical_offset);
void ghostos_aarch64_activate_root(uint64_t root);
void ghostos_aarch64_invalidate_page(uint64_t address);
void ghostos_aarch64_invalidate_all(void);
void ghostos_aarch64_invalidate_range(uint64_t start, uint64_t length);
void ghostos_aarch64_set_core_isolated(uint8_t cpu, bool isolated);
bool ghostos_aarch64_core_isolated(uint8_t cpu);
void ghostos_aarch64_interrupts_disable(void);
void ghostos_aarch64_interrupts_enable(void);
bool ghostos_aarch64_interrupts_init(void);
uint8_t ghostos_aarch64_current_cpu(void);
bool ghostos_aarch64_send_ipi(uint8_t target, uint8_t vector);
void ghostos_aarch64_end_of_interrupt(void);
void ghostos_aarch64_set_exception_hooks(ghostos_aarch64_exception_hooks hooks);
void ghostos_aarch64_exception_dispatch(ghostos_aarch64_exception_frame *frame);
ghostos_aarch64_register_state ghostos_aarch64_capture_registers(uint64_t fault_address);
_Noreturn void ghostos_aarch64_enter_user(ghostos_aarch64_user_context context, uint64_t root);
void ghostos_aarch64_idle(void);
_Noreturn void ghostos_aarch64_halt(void);

#endif
