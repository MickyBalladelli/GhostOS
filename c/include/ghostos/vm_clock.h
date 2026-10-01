#ifndef GHOSTOS_VM_CLOCK_H
#define GHOSTOS_VM_CLOCK_H

#include <stdbool.h>
#include <stdint.h>

uint64_t ghostos_vm_clock_now_ns(void);
uint64_t ghostos_vm_clock_elapsed_ns(uint64_t start_ns);
bool ghostos_vm_manual_clock_set(uint64_t *clock, uint64_t now_ns);
uint64_t ghostos_vm_manual_clock_advance(uint64_t *clock, uint64_t elapsed_ns);

#endif
