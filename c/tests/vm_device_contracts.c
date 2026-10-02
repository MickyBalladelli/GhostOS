#include "ghostos/vm_apic.h"
#include "ghostos/vm_hpet.h"
#include "ghostos/vm_clock.h"

#include <assert.h>
#include <stdio.h>

static void timer_irq(void *context, uint8_t vector, bool level) {
    ghostos_vm_apic_signal(context, vector, level);
}

static void priority_and_trigger_contract(void) {
    ghostos_vm_apic a;
    ghostos_vm_apic_init(&a, 0);
    assert(ghostos_vm_apic_base_msr(&a) == UINT64_C(0xfee00900));
    assert(ghostos_vm_apic_read(&a, 0x30) == ((6u << 16) | 0x14));
    ghostos_vm_apic_signal(&a, 0x30, false);
    ghostos_vm_apic_signal(&a, 0x41, true);
    assert(ghostos_vm_apic_pending(&a) == 0x41);
    ghostos_vm_apic_accept(&a, 0x41);
    assert(ghostos_vm_apic_in_service(&a) == 0x41);
    assert(a.tmr[2] & (1u << 1));
    assert(ghostos_vm_apic_pending(&a) == -1);
    ghostos_vm_apic_eoi(&a);
    assert(a.tmr[2] == 0);
    assert(ghostos_vm_apic_pending(&a) == 0x30);
    ghostos_vm_apic_write(&a, 0x80, 0xf0);
    assert(ghostos_vm_apic_pending(&a) == -1);
    ghostos_vm_apic_write(&a, 0x310, 0x40000 | 0x400);
    // NMI bypasses TPR.
    ghostos_vm_apic_accept(&a, 0x30);
    ghostos_vm_apic_eoi(&a);
    assert(ghostos_vm_apic_pending(&a) == 2);
    ghostos_vm_apic_set_base_msr(&a, 0xfee00000);
    assert(ghostos_vm_apic_pending(&a) == -1);
    ghostos_vm_apic_set_base_msr(&a, 0xfee00800);
    ghostos_vm_apic_write(&a, 0xf0, 0);
    assert(ghostos_vm_apic_pending(&a) == -1);
}

static void timer_and_clock_contract(void) {
    ghostos_vm_apic a;
    ghostos_vm_apic_init(&a, 0);
    uint64_t clock = 0;
    ghostos_vm_apic_write(&a, 0x320, 0x32);
    ghostos_vm_apic_write(&a, 0x3e0, 0); // divide by two
    ghostos_vm_apic_write(&a, 0x380, 100);
    ghostos_vm_apic_advance(&a, clock);
    ghostos_vm_manual_clock_advance(&clock, 1900);
    ghostos_vm_apic_advance(&a, clock);
    assert(ghostos_vm_apic_pending(&a) == -1);
    assert(a.timer_current_count == 5);
    ghostos_vm_manual_clock_advance(&clock, 100);
    ghostos_vm_apic_advance(&a, clock);
    assert(ghostos_vm_apic_pending(&a) == 0x32);
    assert(!a.timer_running);
    ghostos_vm_apic_accept(&a, 0x32);
    ghostos_vm_apic_eoi(&a);
    ghostos_vm_apic_advance(&a, UINT64_MAX);
    assert(ghostos_vm_apic_pending(&a) == -1);
    assert(!ghostos_vm_manual_clock_set(&clock, 0));
    assert(clock == 2000);
    ghostos_vm_apic_write(&a, 0x320, 0x20000 | 0x32);
    ghostos_vm_apic_write(&a, 0x3e0, 0xb);
    ghostos_vm_apic_write(&a, 0x380, 100);
    ghostos_vm_apic_advance(&a, 0);
    ghostos_vm_apic_advance(&a, 1000);
    assert(a.timer_running && a.timer_current_count == 100);
    assert(ghostos_vm_apic_pending(&a) == 0x32);
    ghostos_vm_apic_write(&a, 0x320, 0x10000 | 0x32);
    assert(ghostos_vm_apic_pending(&a) == -1);
}

static void hpet_irq_contract(void) {
    ghostos_vm_apic a;
    ghostos_vm_hpet h;
    ghostos_vm_apic_init(&a, 0);
    ghostos_vm_hpet_init(&h);
    assert((ghostos_vm_hpet_read(&h, 0) & 0x1f00) == 0x1f00);
    h.legacy_vector = 0x52;
    ghostos_vm_hpet_write(&h, 0x10, 3);
    ghostos_vm_hpet_write(&h, 0x100, 6);
    ghostos_vm_hpet_write(&h, 0x110, 100);
    ghostos_vm_hpet_write(&h, 0x104, 100);
    ghostos_vm_hpet_advance(&h, 0, timer_irq, &a);
    ghostos_vm_hpet_advance(&h, 9900, timer_irq, &a);
    assert(ghostos_vm_apic_pending(&a) == -1);
    ghostos_vm_hpet_advance(&h, 10000, timer_irq, &a);
    assert(ghostos_vm_apic_pending(&a) == 0x52);
    assert(h.counter == 100 && h.timers[0].comparator == 200);
    ghostos_vm_apic_accept(&a, 0x52);
    ghostos_vm_apic_eoi(&a);
    ghostos_vm_hpet_advance(&h, 20000, timer_irq, &a);
    assert(ghostos_vm_apic_pending(&a) == 0x52);
    ghostos_vm_hpet_reset(&h);
    assert(h.legacy_vector == 0x52 && h.counter == 0 && h.config == 0);
    ghostos_vm_apic_init(&a, 0);
    // Timer 2 uses non-legacy vector 0x32 and level-triggered delivery.
    ghostos_vm_hpet_write(&h, 0x10, 1);
    ghostos_vm_hpet_write(&h, 0x140, 0xa);
    ghostos_vm_hpet_write(&h, 0x144, 10);
    ghostos_vm_hpet_advance(&h, 0, timer_irq, &a);
    ghostos_vm_hpet_advance(&h, 1000, timer_irq, &a);
    assert(ghostos_vm_apic_pending(&a) == 0x32);
    ghostos_vm_apic_accept(&a, 0x32);
    assert(a.tmr[1] & (1u << 18));
    ghostos_vm_apic_eoi(&a);
    ghostos_vm_hpet_advance(&h, 10000, timer_irq, &a);
    assert(ghostos_vm_apic_pending(&a) == -1);
    ghostos_vm_hpet_write(&h, 0x10, 0);
    ghostos_vm_hpet_advance(&h, 20000, timer_irq, &a);
    assert(h.counter == 100);
}

int main(void) {
    priority_and_trigger_contract();
    timer_and_clock_contract();
    hpet_irq_contract();
    puts("C VM device contracts passed");
    return 0;
}
