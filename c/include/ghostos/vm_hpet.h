#ifndef GHOSTOS_VM_HPET_H
#define GHOSTOS_VM_HPET_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint32_t config;
    uint64_t comparator, periodic_reload;
    bool running;
} ghostos_vm_hpet_timer;

typedef struct {
    uint32_t cap, config;
    uint64_t counter;
    ghostos_vm_hpet_timer timers[32];
    uint8_t legacy_vector;
    uint64_t last_ns;
    bool seeded;
} ghostos_vm_hpet;

typedef void (*ghostos_vm_hpet_irq)(void *context, uint8_t vector, bool level);
size_t ghostos_vm_hpet_size(void);
void ghostos_vm_hpet_init(ghostos_vm_hpet *hpet);
void ghostos_vm_hpet_reset(ghostos_vm_hpet *hpet);
void ghostos_vm_hpet_advance(ghostos_vm_hpet *hpet, uint64_t now_ns,
                            ghostos_vm_hpet_irq irq, void *context);
uint32_t ghostos_vm_hpet_read(const ghostos_vm_hpet *hpet, uint64_t address);
void ghostos_vm_hpet_write(ghostos_vm_hpet *hpet, uint64_t address, uint32_t value);

#endif
