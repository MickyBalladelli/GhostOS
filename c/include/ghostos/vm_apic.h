#ifndef GHOSTOS_VM_APIC_H
#define GHOSTOS_VM_APIC_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Native in-memory ABI only. Snapshot serialization remains field by field. */
typedef struct {
    uint64_t base;
    bool enabled;
    uint32_t id, version, tpr, ppr, ldr, dfr, svr;
    uint32_t irr[8], isr[8], tmr[8], level_pending[8];
    uint32_t esr, icr_hi, icr_lo;
    uint32_t lvt_timer, lvt_thermal, lvt_perfmon, lvt_lint0, lvt_lint1, lvt_error;
    uint32_t timer_initial_count, timer_current_count, timer_divide;
    bool timer_running;
    uint64_t timer_last_ns;
    bool timer_seeded, timer_fired;
} ghostos_vm_apic;

void ghostos_vm_apic_init(ghostos_vm_apic *apic, uint8_t id);
size_t ghostos_vm_apic_size(void);
uint64_t ghostos_vm_apic_base_msr(const ghostos_vm_apic *apic);
void ghostos_vm_apic_set_base_msr(ghostos_vm_apic *apic, uint64_t value);
void ghostos_vm_apic_signal(ghostos_vm_apic *apic, uint8_t vector, bool level);
int ghostos_vm_apic_pending(ghostos_vm_apic *apic);
int ghostos_vm_apic_in_service(const ghostos_vm_apic *apic);
void ghostos_vm_apic_accept(ghostos_vm_apic *apic, uint8_t vector);
void ghostos_vm_apic_eoi(ghostos_vm_apic *apic);
void ghostos_vm_apic_advance(ghostos_vm_apic *apic, uint64_t now_ns);
uint32_t ghostos_vm_apic_read(const ghostos_vm_apic *apic, uint64_t address);
void ghostos_vm_apic_write(ghostos_vm_apic *apic, uint64_t address, uint32_t value);

#endif
