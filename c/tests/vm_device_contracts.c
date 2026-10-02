#include "ghostos/vm_apic.h"
#include "ghostos/vm_hpet.h"
#include "ghostos/vm_clock.h"
#include "ghostos/vm_pit.h"

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

static uint64_t pit_ns_for_ticks(uint64_t ticks) {
    return ticks * UINT64_C(1000000000) / GHOSTOS_VM_PIT_FREQUENCY_HZ + 1;
}

static void pit_program(ghostos_vm_pit *pit, uint8_t control, uint16_t reload) {
    assert(ghostos_vm_pit_write(pit, 0x43, control, 1) == 0);
    assert(ghostos_vm_pit_write(pit, 0x40, reload & 0xff, 1) == 0);
    assert(ghostos_vm_pit_write(pit, 0x40, reload >> 8, 1) == 0);
}

/* Ported from the eight retained Rust PIT cases. The Rc dispatch case is
 * covered by the C port API here; Rust still checks its shared adapter. */
static void pit_contract(void) {
    ghostos_vm_pit pit;
    ghostos_vm_pit_init(&pit);
    pit_program(&pit, 0x36, 0x1234);
    assert(pit.channels[0].reload == 0x1234 && pit.channels[0].count == 0x1234);
    assert(pit.channels[0].running && !pit.channels[0].null_count);

    ghostos_vm_pit_init(&pit);
    pit_program(&pit, 0x34, 10);
    assert(!ghostos_vm_pit_advance(&pit, 0));
    assert(ghostos_vm_pit_advance(&pit, pit_ns_for_ticks(10)));
    assert(pit.channels[0].count == 10 && pit.channels[0].running);
    assert(!ghostos_vm_pit_advance(&pit, pit_ns_for_ticks(15)));
    assert(pit.channels[0].count == 5 && pit.channels[0].running);

    ghostos_vm_pit_init(&pit);
    pit_program(&pit, 0x30, 5);
    assert(!ghostos_vm_pit_advance(&pit, 0));
    assert(ghostos_vm_pit_advance(&pit, pit_ns_for_ticks(5)));
    assert(!pit.channels[0].running && pit.channels[0].output);

    ghostos_vm_pit_init(&pit);
    pit_program(&pit, 0x34, 0x1234);
    assert(ghostos_vm_pit_write(&pit, 0x43, 0, 1) == 0);
    uint64_t value = 0;
    assert(ghostos_vm_pit_read(&pit, 0x40, 1, &value) == 0 && value == 0x34);
    assert(ghostos_vm_pit_read(&pit, 0x40, 1, &value) == 0 && value == 0x12);

    ghostos_vm_pit_init(&pit);
    assert(ghostos_vm_pit_write(&pit, 0x43, 0x30, 1) == 0);
    assert(ghostos_vm_pit_write(&pit, 0x43, 0xc2, 1) == 0);
    assert(ghostos_vm_pit_read(&pit, 0x40, 1, &value) == 0);
    assert(((value >> 1) & 7) == 0 && ((value >> 4) & 3) == 3);
    assert(value & 0x40);

    ghostos_vm_apic apic;
    ghostos_vm_apic_init(&apic, 0);
    ghostos_vm_pit_init(&pit);
    pit_program(&pit, 0x34, 15);
    assert(!ghostos_vm_pit_advance(&pit, 0));
    if (ghostos_vm_pit_advance(&pit, pit_ns_for_ticks(15)))
        ghostos_vm_apic_signal(&apic, 0x20, false);
    assert(ghostos_vm_apic_pending(&apic) == 0x20);
    assert(pit.channels[0].running);

    assert(ghostos_vm_pit_write(&pit, 0x43, 0x10, 2) == 1);
    assert(ghostos_vm_pit_write(&pit, 0x43, 0x10, 4) == 1);
    ghostos_vm_pit_init(&pit);
    pit_program(&pit, 0x34, 0x2010);
    assert(pit.channels[0].reload == 0x2010);
}

int main(void) {
    priority_and_trigger_contract();
    timer_and_clock_contract();
    hpet_irq_contract();
    pit_contract();
    puts("C VM device contracts passed");
    return 0;
}
