#include "ghostos/vm_apic.h"
#include "ghostos/vm_hpet.h"
#include "ghostos/vm_clock.h"
#include "ghostos/vm_pit.h"
#include "ghostos/vm_ps2.h"
#include "ghostos/vm_driver_capabilities.h"
#include "ghostos/vm_display.h"

#include <assert.h>
#include <stdio.h>
#include <string.h>

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

static uint64_t ps2_read(ghostos_vm_ps2 *ps2, uint16_t port) {
    uint64_t value = 0;
    assert(ghostos_vm_ps2_read(ps2, port, 1, &value, NULL, NULL) == 0);
    return value;
}

static void ps2_write(ghostos_vm_ps2 *ps2, uint16_t port, uint8_t value) {
    assert(ghostos_vm_ps2_write(ps2, port, value, 1, NULL, NULL) == 0);
}

/* The three existing Rust PS/2 cases, using the native C port API. */
static void ps2_contract(void) {
    ghostos_vm_ps2 *ps2 = ghostos_vm_ps2_new();
    assert(ps2);
    uint8_t byte = 0x1e;
    ghostos_vm_ps2_keyboard(ps2, &byte, 1, NULL, NULL);
    assert((ps2_read(ps2, 0x64) & 1) == 1);
    assert(ps2_read(ps2, 0x60) == 0x1e);
    assert((ps2_read(ps2, 0x64) & 1) == 0);

    ghostos_vm_ps2_reset(ps2);
    ps2_write(ps2, 0x64, 0xa8);
    ps2_write(ps2, 0x64, 0xd4);
    ps2_write(ps2, 0x60, 0xf4);
    while (ghostos_vm_ps2_input_pending(ps2)) (void)ps2_read(ps2, 0x60);
    uint8_t packet[3] = {8, 1, 0};
    ghostos_vm_ps2_mouse_packet(ps2, packet, NULL, NULL);
    assert(ps2_read(ps2, 0x60) == 8);

    ghostos_vm_ps2_reset(ps2);
    ps2_write(ps2, 0x64, 0xad);
    uint8_t bytes[] = {0x1c, 0xf0, 0x1c};
    ghostos_vm_ps2_keyboard(ps2, bytes, sizeof(bytes), NULL, NULL);
    assert(!ghostos_vm_ps2_input_pending(ps2));
    ps2_write(ps2, 0x64, 0xae);
    for (unsigned i = 0; i < 256; ++i) ghostos_vm_ps2_keyboard(ps2, &byte, 1, NULL, NULL);
    unsigned count = 0;
    while (ghostos_vm_ps2_input_pending(ps2)) {
        (void)ps2_read(ps2, 0x60);
        ++count;
    }
    assert(count <= 64);
    assert((ps2_read(ps2, 0x64) & 1) == 0);
    ghostos_vm_ps2_free(ps2);
}

/* The three existing Rust driver-report cases, including report text. */
static void driver_report_contract(void) {
    ghostos_vm_driver_capability entries[GHOSTOS_VM_DRIVER_CAPABILITY_COUNT];
    assert(ghostos_vm_driver_discover(false, 0, true, false, entries));
    assert(!entries[0].available);
    assert(strcmp(entries[0].selected, "portable-cpu-execution") == 0);
    assert(strcmp(entries[0].fallback, "portable-cpu-execution") == 0);
    assert(strstr(entries[0].semantics, "equivalent"));
    assert(ghostos_vm_driver_discover(false, 0, false, false, entries));
    const unsigned optional[] = {2, 3, 4, 6};
    for (size_t i = 0; i < sizeof(optional) / sizeof(optional[0]); ++i) {
        const ghostos_vm_driver_capability *entry = &entries[optional[i]];
        assert(!entry->available);
        assert(strcmp(entry->selected, entry->fallback) == 0);
        assert(entry->semantics[0]);
    }
    assert(ghostos_vm_driver_discover(true, 0, false, false, entries));
    assert(entries[8].available);
    assert(strcmp(entries[8].selected, "uefi-runtime-services") == 0);
    assert(strcmp(entries[9].selected, "shared-monotonic-clock") == 0);
}

/* The two retained Rust display cases, using the native C state and ports. */
static void display_contract(void) {
    ghostos_vm_display *display = ghostos_vm_display_new();
    assert(display);
    uint64_t value;
    assert(ghostos_vm_display_write(display, false, GHOSTOS_VM_VGA_TEXT_BASE, 2, 0x0741) == 0);
    assert(ghostos_vm_display_read(display, false, GHOSTOS_VM_VGA_TEXT_BASE, 2, &value) == 0);
    assert(value == 0x0741);
    uint8_t character, attribute;
    assert(ghostos_vm_display_cell(display, 0, 0, &character, &attribute));
    assert(character == 'A' && attribute == 7);
    assert(ghostos_vm_display_write(display, true, GHOSTOS_VM_VESA_LFB_BASE + 3, 4, 0xaabbccdd) == 0);
    assert(ghostos_vm_display_read(display, true, GHOSTOS_VM_VESA_LFB_BASE + 3, 4, &value) == 0);
    assert(value == 0xaabbccdd);
    assert(ghostos_vm_display_read(display, true,
        GHOSTOS_VM_VESA_LFB_BASE + GHOSTOS_VM_VESA_FB_SIZE - 1, 2, &value) == GHOSTOS_VM_DISPLAY_ADDRESS);
    assert(ghostos_vm_display_write(display, false, GHOSTOS_VM_VGA_TEXT_BASE, 3, 0) == GHOSTOS_VM_DISPLAY_SIZE);

    ghostos_vm_display_reset(display);
    ghostos_vm_display_port_write(display, 0x3d4, 15);
    ghostos_vm_display_port_write(display, 0x3d5, 81);
    ghostos_vm_display_port_write(display, 0x3d4, 14);
    ghostos_vm_display_port_write(display, 0x3d5, 0);
    ghostos_vm_display_info info;
    ghostos_vm_display_info_get(display, &info);
    assert(info.cursor_x == 1 && info.cursor_y == 1);
    ghostos_vm_display_port_write(display, 0x3c8, 2);
    ghostos_vm_display_port_write(display, 0x3c9, 1);
    ghostos_vm_display_port_write(display, 0x3c9, 2);
    ghostos_vm_display_port_write(display, 0x3c9, 3);
    ghostos_vm_display_port_write(display, 0x3c7, 2);
    assert(ghostos_vm_display_port_read(display, 0x3c9) == 1);
    assert(ghostos_vm_display_port_read(display, 0x3c9) == 2);
    assert(ghostos_vm_display_port_read(display, 0x3c9) == 3);
    ghostos_vm_display_reset(display);
    ghostos_vm_display_info_get(display, &info);
    assert(info.mode == 0 && info.cursor_x == 0 && info.cursor_y == 0);
    ghostos_vm_display_free(display);
}

int main(void) {
    priority_and_trigger_contract();
    timer_and_clock_contract();
    hpet_irq_contract();
    pit_contract();
    ps2_contract();
    driver_report_contract();
    display_contract();
    puts("C VM device contracts passed");
    return 0;
}
