#ifndef GHOSTOS_VM_EXECUTION_H
#define GHOSTOS_VM_EXECUTION_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Callbacks borrow their context only for the duration of a dispatch. The
 * host owns decoded instructions, CPU/MMU/device objects, and error values. */
typedef struct {
    uint64_t rip, code_version, translation_version;
    uint32_t mode;
    bool halted;
} ghostos_vm_execution_state;
typedef struct {
    uint64_t ip, next_ip;
    const uint8_t *mnemonic;
    size_t mnemonic_length;
} ghostos_vm_execution_instruction;
typedef struct {
    void (*state)(void *, ghostos_vm_execution_state *);
    void (*instruction)(void *, size_t, ghostos_vm_execution_instruction *);
    bool (*source_valid)(void *, size_t);
    bool (*step)(void *, size_t);
    void (*record)(void *, size_t);
} ghostos_vm_execution_host;
typedef struct {
    bool (*decode)(void *, uint64_t, ghostos_vm_execution_instruction *);
    bool (*finish)(void *, uint64_t, size_t);
} ghostos_vm_translation_host;

bool ghostos_vm_execution_boundary(const uint8_t *, size_t);
bool ghostos_vm_execution_loop(uint64_t start, const ghostos_vm_execution_instruction *, bool relative, uint64_t displacement);
bool ghostos_vm_execution_source_range(uint64_t start, size_t bytes, uint64_t ip, uint64_t next_ip, size_t *offset, size_t *length);
bool ghostos_vm_execution_observe_version(uint64_t *observed, uint64_t current);
bool ghostos_vm_execution_promote(bool enabled, bool loop, bool compiled, uint64_t *hot, uint64_t threshold);
bool ghostos_vm_execution_retune(uint64_t hits, uint64_t misses);
/* On failure, the callback context holds the original host error. */
bool ghostos_vm_execution_run(const ghostos_vm_execution_host *, void *, size_t count, size_t maximum, size_t *executed, bool *clear_cache);
bool ghostos_vm_execution_translate(const ghostos_vm_translation_host *, void *, uint64_t start, size_t maximum);
#endif
