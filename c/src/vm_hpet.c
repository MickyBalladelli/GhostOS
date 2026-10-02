#include "ghostos/vm_hpet.h"

size_t ghostos_vm_hpet_size(void) { return sizeof(ghostos_vm_hpet); }

void ghostos_vm_hpet_init(ghostos_vm_hpet *h) {
    *h = (ghostos_vm_hpet){0};
    h->cap = 0x10000 | (31u << 8) | 0x20 | 0x8000;
}

void ghostos_vm_hpet_reset(ghostos_vm_hpet *h) {
    uint8_t vector = h->legacy_vector;
    ghostos_vm_hpet_init(h);
    h->legacy_vector = vector;
}

void ghostos_vm_hpet_advance(ghostos_vm_hpet *h, uint64_t now,
                            ghostos_vm_hpet_irq irq, void *context) {
    if (!(h->config & 1) || !h->seeded) {
        h->last_ns = now;
        h->seeded = true;
        return;
    }
    if (now <= h->last_ns) return;
    uint64_t ticks = (now - h->last_ns) / 100;
    h->last_ns = now;
    if (!ticks) return;
    h->counter += ticks;
    for (unsigned i = 0; i < 32; ++i) {
        ghostos_vm_hpet_timer *t = &h->timers[i];
        if (!t->running || !(t->config & 2) || h->counter < t->comparator) continue;
        if (t->config & 4) {
            uint64_t period = t->periodic_reload ? t->periodic_reload : 1;
            t->comparator += ((h->counter - t->comparator) / period + 1) * period;
            t->periodic_reload = period;
        } else t->running = false;
        uint8_t vector = (h->config & 2) && i < 2 ? h->legacy_vector : (uint8_t)(0x30 + i % 24);
        if (vector && irq) irq(context, vector, (t->config & 8) != 0);
    }
}

uint32_t ghostos_vm_hpet_read(const ghostos_vm_hpet *h, uint64_t address) {
    uint32_t off = (uint32_t)address & 0xfff;
    switch (off) {
    case 0: return h->cap;
    case 0x10: return h->config;
    case 0x30: {
        uint32_t status = 0;
        for (unsigned i = 0; i < 32; ++i) {
            if (h->timers[i].running && h->counter >= h->timers[i].comparator)
                status |= UINT32_C(1) << i;
        }
        return status;
    }
    default: break;
    }
    /* Preserve the existing VM register aperture and comparator aliases. */
    if (off < 0x100 || off > 0x400) return 0;
    const ghostos_vm_hpet_timer *t = &h->timers[(off - 0x100) / 0x20];
    switch ((off - 0x100) % 0x20) {
    case 0: return t->config;
    case 4: return (uint32_t)t->comparator;
    case 8: return (t->config & 8) ? 0 : (uint32_t)(t->comparator >> 32);
    case 0x10: return (t->config & 8) ? (uint32_t)t->periodic_reload : 0;
    case 0x14: return (t->config & 8) ? (uint32_t)(t->periodic_reload >> 32) : 0;
    default: return 0;
    }
}

static uint64_t low_word(uint64_t old, uint32_t value) {
    return (old & UINT64_C(0xffffffff00000000)) | value;
}

static uint64_t high_word(uint64_t old, uint32_t value) {
    return (old & UINT64_C(0xffffffff)) | ((uint64_t)value << 32);
}

void ghostos_vm_hpet_write(ghostos_vm_hpet *h, uint64_t address, uint32_t value) {
    uint32_t off = (uint32_t)address & 0xfff;
    if (off == 0x10) {
        h->config = value & 3;
        if (h->config & 1) h->seeded = false;
        return;
    }
    if (off < 0x100 || off > 0x400) return;
    ghostos_vm_hpet_timer *t = &h->timers[(off - 0x100) / 0x20];
    switch ((off - 0x100) % 0x20) {
    case 0:
        t->config = value & 0x7e;
        t->running = (t->config & 2) != 0;
        break;
    case 4:
        t->comparator = low_word(t->comparator, value);
        t->running = (t->config & 2) != 0;
        break;
    case 8:
        if (!(t->config & 8)) t->comparator = high_word(t->comparator, value);
        break;
    case 0xc: t->comparator = low_word(t->comparator, value); break;
    case 0x10:
        if (t->config & 4) t->periodic_reload = low_word(t->periodic_reload, value);
        break;
    case 0x14:
        if (t->config & 4) t->periodic_reload = high_word(t->periodic_reload, value);
        break;
    default: break;
    }
}
