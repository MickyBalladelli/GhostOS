#ifndef GHOSTOS_ARCH_H
#define GHOSTOS_ARCH_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_ARCH_TABLE_FRAME_COUNT 6
#define GHOSTOS_RESCHEDULE_IPI_VECTOR UINT8_C(0xf0)
#define GHOSTOS_TLB_SHOOTDOWN_IPI_VECTOR UINT8_C(0xf1)

typedef struct {
    const char *name;
    bool smp, interrupts, user_mode, isolation;
} ghostos_architecture_evidence;

typedef uintptr_t (*ghostos_arch_operation)(void *context);
typedef struct {
    void *context;
    void (*idle)(void *context, uint8_t state);
    void (*disable_interrupts)(void *context);
    void (*enable_interrupts)(void *context);
    void (*invalidate_range)(void *context, uint64_t start, uint64_t length);
    bool (*install_root)(void *context, const uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT], uint64_t physical_offset);
    void (*enable_supervisor_protections)(void *context);
    void (*init_interrupts)(void *context);
    uintptr_t (*with_user_access)(void *context, ghostos_arch_operation operation, void *operation_context);
    bool (*ring3_supported)(void *context);
    void (*enter_user)(void *context, uintptr_t stack_pointer, uintptr_t instruction_pointer, uint64_t root);
    void (*halt)(void *context);
    bool x86_supervisor_protections;
} ghostos_arch_backend;

typedef bool (*ghostos_arch_validate_tables)(void *context, const uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT], uint64_t physical_offset);

ghostos_architecture_evidence ghostos_arch_evidence(void);
bool ghostos_arch_initialize(const ghostos_arch_backend *backend, ghostos_arch_validate_tables validate, void *validate_context, const uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT], uint64_t physical_offset);
void ghostos_arch_idle(const ghostos_arch_backend *backend, uint8_t state);
void ghostos_arch_disable_interrupts(const ghostos_arch_backend *backend);
void ghostos_arch_enable_interrupts(const ghostos_arch_backend *backend);
void ghostos_arch_invalidate_tlb_range(const ghostos_arch_backend *backend, uint64_t start, uint64_t length);
uintptr_t ghostos_arch_with_user_access(const ghostos_arch_backend *backend, ghostos_arch_operation operation, void *context);
bool ghostos_arch_ring3_supported(const ghostos_arch_backend *backend);
bool ghostos_arch_read_user(const ghostos_arch_backend *backend, const void *user_pointer, void *output, size_t size, size_t alignment);
bool ghostos_arch_write_user(const ghostos_arch_backend *backend, void *user_pointer, const void *input, size_t size, size_t alignment);
_Noreturn void ghostos_arch_enter_user(const ghostos_arch_backend *backend, uintptr_t stack_pointer, uintptr_t instruction_pointer, uint64_t root);
_Noreturn void ghostos_arch_halt(const ghostos_arch_backend *backend);

#endif
