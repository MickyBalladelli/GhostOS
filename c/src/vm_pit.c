#include "ghostos/vm_pit.h"
#include <stddef.h>

_Static_assert(sizeof(ghostos_vm_pit_channel) == 18, "PIT channel ABI");
_Static_assert(_Alignof(ghostos_vm_pit_channel) == 2, "PIT channel alignment");
_Static_assert(sizeof(ghostos_vm_pit) == 72, "PIT state ABI");
_Static_assert(_Alignof(ghostos_vm_pit) == 8, "PIT state alignment");
_Static_assert(offsetof(ghostos_vm_pit, last_ns) == 56, "PIT timestamp offset");

void ghostos_vm_pit_init(ghostos_vm_pit *pit) {
    *pit = (ghostos_vm_pit){0};
    for (unsigned i = 0; i < 3; ++i) {
        pit->channels[i] = (ghostos_vm_pit_channel){
            .reload = UINT16_MAX, .count = UINT16_MAX,
            .mode = 3, .access = 3, .gate = true, .null_count = true
        };
    }
}

static void latch_count(ghostos_vm_pit_channel *ch) {
    if (!ch->has_latched_count) {
        ch->latched_count = ch->count;
        ch->has_latched_count = true;
    }
}

static uint8_t status_byte(const ghostos_vm_pit_channel *ch) {
    return (uint8_t)((ch->output ? 0x80 : 0) |
        (ch->null_count ? 0x40 : 0) | (ch->access << 4) |
        ((ch->mode & 7) << 1) | ch->bcd);
}

static void write_control(ghostos_vm_pit *pit, uint8_t value) {
    unsigned channel = value >> 6;
    unsigned access = (value >> 4) & 3;
    if (channel == 3) {
        unsigned selected = (value >> 1) & 7;
        for (unsigned i = 0; i < 3; ++i) {
            ghostos_vm_pit_channel *ch = &pit->channels[i];
            if (!(selected & (1u << i))) continue;
            if (!(value & 0x20)) latch_count(ch);
            if (!(value & 0x10)) {
                ch->latched_status = status_byte(ch);
                ch->has_latched_status = true;
            }
        }
        return;
    }
    ghostos_vm_pit_channel *ch = &pit->channels[channel];
    if (access == 0) {
        latch_count(ch);
        return;
    }
    ch->mode = (value & 0x0e) >> 1;
    if (ch->mode > 5) ch->mode &= 3;
    ch->bcd = (value & 1) != 0;
    ch->access = (uint8_t)access;
    ch->null_count = true;
    ch->wr_bytes = ch->rd_bytes = 0;
    ch->has_latched_count = ch->has_latched_status = false;
}

static void write_reload(ghostos_vm_pit_channel *ch, uint8_t byte) {
    if (ch->access == 1 || (ch->access == 3 && ch->wr_bytes == 0)) {
        ch->reload = (uint16_t)((ch->reload & 0xff00) | byte);
        if (ch->access == 3) {
            ch->wr_bytes = 1;
            return;
        }
    } else {
        ch->reload = (uint16_t)((ch->reload & 0xff) | ((uint16_t)byte << 8));
        ch->wr_bytes = 0;
    }
    ch->count = ch->reload;
    ch->running = ch->reload != 0;
    ch->output = false;
    ch->null_count = false;
    ch->rd_bytes = 0;
}

static uint8_t read_byte(ghostos_vm_pit_channel *ch) {
    if (ch->has_latched_status) {
        ch->has_latched_status = false;
        return ch->latched_status;
    }
    /* Preserve the existing VM's consume-on-first-byte latch behavior. */
    uint16_t count = ch->has_latched_count ? ch->latched_count : ch->count;
    ch->has_latched_count = false;
    if (ch->access == 1) return (uint8_t)count;
    if (ch->access == 2) return (uint8_t)(count >> 8);
    if (ch->rd_bytes == 0) {
        ch->rd_bytes = 1;
        return (uint8_t)count;
    }
    ch->rd_bytes = 0;
    return (uint8_t)(count >> 8);
}

uint8_t ghostos_vm_pit_read(ghostos_vm_pit *pit, uint16_t port,
    uint8_t size, uint64_t *value) {
    if (size != 1) return 1;
    if (port < 0x40 || port > 0x42) return 2;
    *value = read_byte(&pit->channels[port - 0x40]);
    return 0;
}

uint8_t ghostos_vm_pit_write(ghostos_vm_pit *pit, uint16_t port,
    uint64_t value, uint8_t size) {
    if (size != 1) return 1;
    if (port == 0x43) write_control(pit, (uint8_t)value);
    else if (port >= 0x40 && port <= 0x42)
        write_reload(&pit->channels[port - 0x40], (uint8_t)value);
    else return 2;
    return 0;
}

static bool tick_channel(ghostos_vm_pit_channel *ch, uint64_t ticks) {
    if (!ch->gate || !ch->running || ticks == 0) return false;
    bool fired = false;
    while (ticks > 0) {
        uint64_t count = ch->count;
        if (count == 0) {
            ch->output = true;
            ch->running = false;
            return true;
        }
        if (count > ticks) {
            ch->count = (uint16_t)(count - ticks);
            break;
        }
        ticks -= count;
        fired = true;
        ch->output = true;
        if (ch->mode == 2 || ch->mode == 3) ch->count = ch->reload;
        else {
            ch->running = false;
            break;
        }
    }
    return fired;
}

bool ghostos_vm_pit_advance(ghostos_vm_pit *pit, uint64_t now_ns) {
    if (!pit->has_last_ns) {
        pit->last_ns = now_ns;
        pit->has_last_ns = true;
        return false;
    }
    if (now_ns <= pit->last_ns) return false;
    uint64_t elapsed = now_ns - pit->last_ns;
    pit->last_ns = now_ns;
    /* Exact u128-equivalent arithmetic without a compiler extension. */
    uint64_t ticks = (elapsed / UINT64_C(1000000000)) * GHOSTOS_VM_PIT_FREQUENCY_HZ +
        ((elapsed % UINT64_C(1000000000)) * GHOSTOS_VM_PIT_FREQUENCY_HZ) /
        UINT64_C(1000000000);
    bool fired = false;
    for (unsigned i = 0; i < 3; ++i) {
        bool pulse = tick_channel(&pit->channels[i], ticks);
        if (i == 0) fired = pulse;
    }
    return fired;
}
