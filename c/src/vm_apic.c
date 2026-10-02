#include "ghostos/vm_apic.h"

#define MASK UINT32_C(0x10000)
#define PERIODIC UINT32_C(0x20000)

static int highest(const uint32_t table[8]) {
    for (int i = 7; i >= 0; --i) {
        for (int bit = 31; bit >= 0; --bit) {
            if (table[i] & (UINT32_C(1) << bit)) return i * 32 + bit;
        }
    }
    return -1;
}

static void update_ppr(ghostos_vm_apic *a) {
    int vector = highest(a->isr);
    uint32_t priority = vector < 0 ? 0 : (uint32_t)vector >> 4;
    if (priority < (a->tpr >> 4)) priority = a->tpr >> 4;
    a->ppr = priority << 4;
}

void ghostos_vm_apic_init(ghostos_vm_apic *a, uint8_t id) {
    *a = (ghostos_vm_apic){0};
    a->base = UINT64_C(0xfee00000);
    a->enabled = true;
    a->id = (uint32_t)id << 24;
    a->version = (6u << 16) | 0x14;
    a->svr = 0x100;
    a->lvt_timer = a->lvt_thermal = a->lvt_perfmon = MASK;
    a->lvt_lint0 = a->lvt_lint1 = a->lvt_error = MASK;
}

size_t ghostos_vm_apic_size(void) { return sizeof(ghostos_vm_apic); }

uint64_t ghostos_vm_apic_base_msr(const ghostos_vm_apic *a) {
    return (a->base & UINT64_C(0xfffff000)) | 0x100 | (a->enabled ? 0x800 : 0);
}

void ghostos_vm_apic_set_base_msr(ghostos_vm_apic *a, uint64_t value) {
    a->base = value & UINT64_C(0xfffff000);
    a->enabled = (value & 0x800) != 0;
}

void ghostos_vm_apic_signal(ghostos_vm_apic *a, uint8_t vector, bool level) {
    unsigned index = vector / 32;
    uint32_t bit = UINT32_C(1) << (vector % 32);
    a->irr[index] |= bit;
    if (level) a->level_pending[index] |= bit;
    else a->level_pending[index] &= ~bit;
}

int ghostos_vm_apic_pending(ghostos_vm_apic *a) {
    if (!a->enabled || !(a->svr & 0x100)) return -1;
    if (a->timer_fired && !(a->lvt_timer & MASK)) {
        ghostos_vm_apic_signal(a, (uint8_t)a->lvt_timer, false);
        a->timer_fired = false;
    }
    int vector = highest(a->irr);
    if (vector < 0) return -1;
    if (vector != 2 && ((uint32_t)vector >> 4) <= (a->tpr >> 4)) return -1;
    int service = highest(a->isr);
    if (service >= 0 && (service >> 4) >= (vector >> 4)) return -1;
    return vector;
}

int ghostos_vm_apic_in_service(const ghostos_vm_apic *a) { return highest(a->isr); }

void ghostos_vm_apic_accept(ghostos_vm_apic *a, uint8_t vector) {
    unsigned index = vector / 32;
    uint32_t bit = UINT32_C(1) << (vector % 32);
    a->irr[index] &= ~bit;
    a->isr[index] |= bit;
    if (a->level_pending[index] & bit) a->tmr[index] |= bit;
    else a->tmr[index] &= ~bit;
    a->level_pending[index] &= ~bit;
    if (!(a->lvt_timer & PERIODIC) && (a->lvt_timer & 0xff) == vector) a->timer_fired = false;
    update_ppr(a);
}

void ghostos_vm_apic_eoi(ghostos_vm_apic *a) {
    int vector = highest(a->isr);
    if (vector >= 0) {
        uint32_t bit = UINT32_C(1) << (vector % 32);
        a->isr[vector / 32] &= ~bit;
        a->tmr[vector / 32] &= ~bit;
    }
    update_ppr(a);
}

void ghostos_vm_apic_advance(ghostos_vm_apic *a, uint64_t now) {
    if (!a->timer_running || !a->timer_initial_count) return;
    if (!a->timer_seeded) {
        a->timer_last_ns = now;
        a->timer_seeded = true;
        return;
    }
    if (now <= a->timer_last_ns) return;
    static const uint32_t divisors[16] = {2, 4, 8, 16, 2, 4, 8, 16, 32, 64, 128, 1, 32, 64, 128, 1};
    uint64_t effective = (now - a->timer_last_ns) / 10 / divisors[a->timer_divide & 0xb];
    uint64_t remaining = a->timer_current_count ? a->timer_current_count : a->timer_initial_count;
    if (effective >= remaining) {
        a->timer_fired = true;
        if (a->lvt_timer & PERIODIC) {
            a->timer_current_count = a->timer_initial_count - (uint32_t)(effective % remaining);
        } else {
            a->timer_current_count = 0;
            a->timer_running = false;
        }
    } else if (effective) a->timer_current_count = (uint32_t)(remaining - effective);
    a->timer_last_ns = now;
}

uint32_t ghostos_vm_apic_read(const ghostos_vm_apic *a, uint64_t address) {
    uint32_t off = (uint32_t)address & 0xff0;
    if (off >= 0x100 && off <= 0x11c) return a->isr[(off - 0x100) / 4];
    if (off >= 0x180 && off <= 0x19c) return a->tmr[(off - 0x180) / 4];
    if (off >= 0x200 && off <= 0x21c) return a->irr[(off - 0x200) / 4];
    switch (off) {
    case 0x020: return a->id;
    case 0x030: return a->version;
    case 0x080: return a->tpr;
    case 0x0a0: return a->ppr;
    case 0x0d0: return a->ldr;
    case 0x0e0: return a->dfr;
    case 0x0f0: return a->svr;
    case 0x280: return a->esr;
    case 0x300: return a->icr_hi;
    case 0x310: return a->icr_lo & ~UINT32_C(0x1000);
    case 0x320: return a->lvt_timer;
    case 0x330: return a->lvt_thermal;
    case 0x340: return a->lvt_perfmon;
    case 0x350: return a->lvt_lint0;
    case 0x360: return a->lvt_lint1;
    case 0x370: return a->lvt_error;
    case 0x380: return a->timer_initial_count;
    case 0x390: return a->timer_current_count;
    case 0x3e0: return a->timer_divide;
    default: return 0;
    }
}

static void icr_write(ghostos_vm_apic *a, uint32_t value) {
    a->icr_lo = value;
    uint32_t shorthand = value & 0xc0000;
    uint8_t destination = (uint8_t)(a->icr_hi >> 24);
    bool logical = (a->dfr >> 28) == 0xf
        ? ((a->ldr >> 24) & destination) != 0
        : (a->id >> 28) == (destination >> 4) && ((a->ldr >> 24) & destination & 0xf) != 0;
    bool self = shorthand == 0x40000 || shorthand == 0x80000 ||
        (shorthand == 0 && (destination == 0xff || destination == (uint8_t)(a->id >> 24) || logical));
    if (!self) return;
    switch (value & 0x700) {
    case 0: ghostos_vm_apic_signal(a, (uint8_t)value, false); break;
    case 0x400: ghostos_vm_apic_signal(a, 2, false); break;
    case 0x500: for (unsigned i = 0; i < 8; ++i) a->isr[i] = 0; break;
    default: a->esr |= 2; break;
    }
}

void ghostos_vm_apic_write(ghostos_vm_apic *a, uint64_t address, uint32_t value) {
    switch ((uint32_t)address & 0xff0) {
    case 0x020: a->id = value & 0x0f000000; break;
    case 0x080: a->tpr = value & 0xff; update_ppr(a); break;
    case 0x0b0: ghostos_vm_apic_eoi(a); break;
    case 0x0d0: a->ldr = value & 0xff000000; break;
    case 0x0e0: a->dfr = value & 0xf0000000; break;
    case 0x0f0: a->svr = value; break;
    case 0x280: a->esr = 0; break;
    case 0x300: a->icr_hi = value; break;
    case 0x310: icr_write(a, value); break;
    case 0x320:
        a->lvt_timer = value & 0x31fff;
        if (a->lvt_timer & MASK) {
            unsigned vector = a->lvt_timer & 0xff;
            uint32_t bit = UINT32_C(1) << (vector % 32);
            a->timer_fired = false;
            a->irr[vector / 32] &= ~bit;
            a->level_pending[vector / 32] &= ~bit;
            a->tmr[vector / 32] &= ~bit;
        }
        break;
    case 0x330: a->lvt_thermal = value & 0x31fff; break;
    case 0x340: a->lvt_perfmon = value & 0x31fff; break;
    case 0x350: a->lvt_lint0 = value & 0x31fff; break;
    case 0x360: a->lvt_lint1 = value & 0x31fff; break;
    case 0x370: a->lvt_error = value & 0x31fff; break;
    case 0x380:
        a->timer_initial_count = a->timer_current_count = value;
        a->timer_running = value != 0;
        a->timer_fired = a->timer_seeded = false;
        break;
    case 0x3e0: a->timer_divide = value & 0xb; break;
    default: break;
    }
}
