#ifndef GHOSTOS_VM_TERMINAL_PLATFORM_H
#define GHOSTOS_VM_TERMINAL_PLATFORM_H
#include <stdbool.h>
#include <stdint.h>

typedef struct ghostos_vm_terminal_mode ghostos_vm_terminal_mode;
enum ghostos_vm_terminal_platform_result {
    GHOSTOS_VM_TERMINAL_PLATFORM_OK, GHOSTOS_VM_TERMINAL_PLATFORM_IO,
    GHOSTOS_VM_TERMINAL_PLATFORM_ACTIVE, GHOSTOS_VM_TERMINAL_PLATFORM_ALLOCATION
};
/* Error outputs contain only native numeric error codes, never terminal data.
 * Explicit restore disarms signal ownership once; free retries restoration and
 * releases the native handle. The Unix guard permits one active raw session. */
uint32_t ghostos_vm_terminal_mode_enter(ghostos_vm_terminal_mode **, int32_t *);
uint32_t ghostos_vm_terminal_mode_restore(ghostos_vm_terminal_mode *, int32_t *);
void ghostos_vm_terminal_mode_free(ghostos_vm_terminal_mode *);
bool ghostos_vm_terminal_size(uint16_t *, uint16_t *);
#endif
