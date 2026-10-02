#ifndef GHOSTOS_VM_CLOCK_H
#define GHOSTOS_VM_CLOCK_H

#include <stdbool.h>
#include <stdint.h>

typedef struct ghostos_vm_host_clock ghostos_vm_host_clock;
typedef struct ghostos_vm_manual_clock ghostos_vm_manual_clock;

uint64_t ghostos_vm_clock_now_ns(void);
uint64_t ghostos_vm_clock_elapsed_ns(uint64_t start_ns);
bool ghostos_vm_manual_clock_set(uint64_t *clock, uint64_t now_ns);
uint64_t ghostos_vm_manual_clock_advance(uint64_t *clock, uint64_t elapsed_ns);
ghostos_vm_host_clock *ghostos_vm_host_clock_new(void);
void ghostos_vm_host_clock_free(ghostos_vm_host_clock *clock);
uint64_t ghostos_vm_host_clock_now(const ghostos_vm_host_clock *clock);
ghostos_vm_manual_clock *ghostos_vm_manual_clock_new(uint64_t now_ns);
void ghostos_vm_manual_clock_free(ghostos_vm_manual_clock *clock);
bool ghostos_vm_manual_clock_set_state(ghostos_vm_manual_clock *clock, uint64_t now_ns);
uint64_t ghostos_vm_manual_clock_advance_state(ghostos_vm_manual_clock *clock,
    uint64_t elapsed_ns);
uint64_t ghostos_vm_manual_clock_now(const ghostos_vm_manual_clock *clock);

#endif
