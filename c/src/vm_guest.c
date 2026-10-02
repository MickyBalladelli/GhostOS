#include "ghostos/vm_guest.h"

#include <stdlib.h>
#include <string.h>

typedef struct {
    uint8_t *data;
    size_t head, length, capacity;
} byte_queue;

struct ghostos_vm_guest_agent {
    byte_queue host_to_guest, guest_to_host, events;
    ghostos_vm_guest_event active_event;
    bool has_active_event;
};

static bool append(byte_queue *queue, const void *bytes, size_t length) {
    if (!length) return true;
    if (!bytes || length > SIZE_MAX - queue->length) return false;
    size_t needed = queue->length + length;
    if (needed > queue->capacity) {
        size_t capacity = queue->capacity ? queue->capacity : 64;
        while (capacity < needed) {
            if (capacity > SIZE_MAX / 2) {
                capacity = needed;
                break;
            }
            capacity *= 2;
        }
        uint8_t *replacement = malloc(capacity);
        if (!replacement) return false;
        if (queue->length) {
            size_t first = queue->capacity - queue->head;
            if (first > queue->length) first = queue->length;
            memcpy(replacement, queue->data + queue->head, first);
            memcpy(replacement + first, queue->data, queue->length - first);
        }
        free(queue->data);
        queue->data = replacement;
        queue->capacity = capacity;
        queue->head = 0;
    }
    size_t tail_space = queue->capacity - queue->head;
    size_t tail = queue->length >= tail_space ? queue->length - tail_space :
        queue->head + queue->length;
    size_t first = queue->capacity - tail;
    if (first > length) first = length;
    memcpy(queue->data + tail, bytes, first);
    memcpy(queue->data, (const uint8_t *)bytes + first, length - first);
    queue->length = needed;
    return true;
}

static size_t take(byte_queue *queue, void *output, size_t capacity) {
    size_t length = capacity < queue->length ? capacity : queue->length;
    if (!length) return 0;
    size_t first = queue->capacity - queue->head;
    if (first > length) first = length;
    memcpy(output, queue->data + queue->head, first);
    memcpy((uint8_t *)output + first, queue->data, length - first);
    size_t head_space = queue->capacity - queue->head;
    queue->head = length >= head_space ? length - head_space : queue->head + length;
    queue->length -= length;
    if (!queue->length) queue->head = 0;
    return length;
}

ghostos_vm_guest_agent *ghostos_vm_guest_agent_new(void) {
    return calloc(1, sizeof(ghostos_vm_guest_agent));
}

void ghostos_vm_guest_agent_free(ghostos_vm_guest_agent *agent) {
    if (!agent) return;
    free(agent->host_to_guest.data);
    free(agent->guest_to_host.data);
    free(agent->events.data);
    free(agent);
}

void ghostos_vm_guest_agent_clear(ghostos_vm_guest_agent *agent) {
    agent->host_to_guest.head = agent->host_to_guest.length = 0;
    agent->guest_to_host.head = agent->guest_to_host.length = 0;
    agent->events.head = agent->events.length = 0;
    agent->has_active_event = false;
}

bool ghostos_vm_guest_agent_send(ghostos_vm_guest_agent *agent,
    const uint8_t *bytes, size_t length) {
    return append(&agent->host_to_guest, bytes, length);
}

size_t ghostos_vm_guest_agent_output_length(const ghostos_vm_guest_agent *agent) {
    return agent->guest_to_host.length;
}

size_t ghostos_vm_guest_agent_take_output(ghostos_vm_guest_agent *agent,
    uint8_t *output, size_t capacity) {
    return take(&agent->guest_to_host, output, capacity);
}

bool ghostos_vm_guest_agent_notify(ghostos_vm_guest_agent *agent,
    const ghostos_vm_guest_event *event) {
    return append(&agent->events, event, sizeof(*event));
}

size_t ghostos_vm_guest_agent_pending_events(const ghostos_vm_guest_agent *agent) {
    return agent->events.length / sizeof(ghostos_vm_guest_event);
}

static uint64_t offset_from(uint64_t address, uint64_t base) {
    return address >= base ? address - base : 0;
}

uint8_t ghostos_vm_guest_agent_read(ghostos_vm_guest_agent *agent,
    uint64_t address, uint8_t size, uint64_t *value) {
    uint64_t offset = offset_from(address, GHOSTOS_VM_GUEST_AGENT_BASE);
    if (offset == 0 && size == 4) *value = UINT32_C(0x41474f53);
    else if (offset == 4 && size == 4) *value = 1;
    else if (offset == 8 && size == 8) *value = 15;
    else if (offset == 0x0c && size == 4) {
        *value = (agent->host_to_guest.length ? 1u : 0u) |
            (agent->guest_to_host.length ? 2u : 0u) |
            (agent->events.length || agent->has_active_event ? 4u : 0u);
    } else if (offset == 0x10 && size == 1) {
        uint8_t byte = 0;
        (void)take(&agent->host_to_guest, &byte, 1);
        *value = byte;
    } else if (offset == 0x18 && size == 4) {
        agent->has_active_event = take(&agent->events, &agent->active_event,
            sizeof(agent->active_event)) != 0;
        *value = agent->has_active_event ? agent->active_event.kind : 0;
    } else if (offset == 0x20 && size == 8) {
        *value = agent->has_active_event && agent->active_event.kind == 3 ? agent->active_event.base : 0;
    } else if (offset == 0x28 && size == 8) {
        *value = agent->has_active_event && agent->active_event.kind == 3 ? agent->active_event.size : 0;
    } else return 1;
    return 0;
}

uint8_t ghostos_vm_guest_agent_write(ghostos_vm_guest_agent *agent,
    uint64_t address, uint64_t value, uint8_t size) {
    uint64_t offset = offset_from(address, GHOSTOS_VM_GUEST_AGENT_BASE);
    if (offset == 0x14 && size == 1) {
        uint8_t byte = (uint8_t)value;
        if (!append(&agent->guest_to_host, &byte, 1)) return 3;
    } else if (offset == 0x1c && size == 4) {
        if ((uint32_t)value != 0) agent->has_active_event = false;
    } else if (offset == 0x2c && size == 4) {
        if (value & 1) ghostos_vm_guest_agent_clear(agent);
    } else return 1;
    return 0;
}

_Static_assert(sizeof(ghostos_vm_guest_event) == 24, "guest event ABI");
_Static_assert(offsetof(ghostos_vm_guest_event, base) == 8, "guest event base offset");
_Static_assert(sizeof(ghostos_vm_memory_hotplug) == 48, "hotplug ABI");
_Static_assert(_Alignof(ghostos_vm_memory_hotplug) == 8, "hotplug alignment");
_Static_assert(offsetof(ghostos_vm_memory_hotplug, has_pending) == 40, "hotplug pending offset");
_Static_assert(sizeof(ghostos_vm_pvclock) == 24, "pvclock ABI");
_Static_assert(_Alignof(ghostos_vm_pvclock) == 8, "pvclock alignment");
_Static_assert(offsetof(ghostos_vm_pvclock, version) == 16, "pvclock version offset");
_Static_assert(offsetof(ghostos_vm_pvclock, has_system_time_page) == 20, "pvclock flag offset");

void ghostos_vm_memory_hotplug_init(ghostos_vm_memory_hotplug *state,
    uint64_t current, uint64_t maximum) {
    *state = (ghostos_vm_memory_hotplug){
        .current = current, .maximum = maximum > current ? maximum : current
    };
}

static uint64_t saturating_add(uint64_t left, uint64_t right) {
    return right > UINT64_MAX - left ? UINT64_MAX : left + right;
}

bool ghostos_vm_memory_hotplug_add(ghostos_vm_memory_hotplug *state,
    uint64_t base, uint64_t size) {
    uint64_t total = saturating_add(state->current, size);
    if (!size || total > state->maximum) return false;
    state->current = total;
    state->pending_base = base;
    state->pending_size = size;
    state->has_pending = true;
    return true;
}

bool ghostos_vm_memory_hotplug_take_request(ghostos_vm_memory_hotplug *state,
    uint64_t *request) {
    if (!state->has_request) return false;
    *request = state->request;
    state->has_request = false;
    return true;
}

void ghostos_vm_memory_hotplug_clear_pending(ghostos_vm_memory_hotplug *state) {
    state->has_pending = false;
}

void ghostos_vm_memory_hotplug_reset(ghostos_vm_memory_hotplug *state) {
    state->has_pending = state->has_request = false;
}

uint8_t ghostos_vm_memory_hotplug_read(const ghostos_vm_memory_hotplug *state,
    uint64_t address, uint8_t size, uint64_t *value) {
    if (size != 8) return 1;
    switch (offset_from(address, GHOSTOS_VM_MEMORY_HOTPLUG_BASE)) {
        case 0: *value = state->current; break;
        case 8: *value = state->maximum; break;
        case 0x10: *value = state->has_pending ? state->pending_base : 0; break;
        case 0x18: *value = state->has_pending ? state->pending_size : 0; break;
        case 0x20: *value = state->has_pending; break;
        default: return 2;
    }
    return 0;
}

uint8_t ghostos_vm_memory_hotplug_write(ghostos_vm_memory_hotplug *state,
    uint64_t address, uint64_t value, uint8_t size) {
    if (size != 8) return 1;
    switch (offset_from(address, GHOSTOS_VM_MEMORY_HOTPLUG_BASE)) {
        case 0x28: if (value & 1) state->has_pending = false; break;
        case 0x30:
            if (!value || value % 4096 || saturating_add(state->current, value) > state->maximum)
                return 4;
            state->request = value;
            state->has_request = true;
            break;
        default: return 2;
    }
    return 0;
}

void ghostos_vm_pvclock_reset(ghostos_vm_pvclock *state) {
    *state = (ghostos_vm_pvclock){0};
}

void ghostos_vm_pvclock_write_msr(ghostos_vm_pvclock *state, uint32_t msr, uint64_t value) {
    if (msr == GHOSTOS_VM_KVM_SYSTEM_TIME_NEW) {
        state->system_time_page = value & ~UINT64_C(0xfff);
        state->has_system_time_page = (value & 1) != 0;
    } else if (msr == GHOSTOS_VM_KVM_WALL_CLOCK_NEW) {
        state->wall_clock_page = value & ~UINT64_C(0xfff);
        state->has_wall_clock_page = (value & 1) != 0;
    }
}

bool ghostos_vm_pvclock_configured(const ghostos_vm_pvclock *state) {
    return state->has_system_time_page || state->has_wall_clock_page;
}

static void write_le(uint8_t *output, uint64_t value, unsigned length) {
    for (unsigned i = 0; i < length; ++i) output[i] = (uint8_t)(value >> (8 * i));
}

void ghostos_vm_pvclock_update(ghostos_vm_pvclock *state, uint64_t monotonic_ns,
    ghostos_vm_guest_wall_time wall_time, ghostos_vm_guest_memory_write write_memory,
    void *context) {
    if (state->has_system_time_page) {
        state->version = (state->version + 1u) | 1u;
        uint8_t info[32] = {0};
        write_le(info, state->version, 4);
        write_le(info + 16, monotonic_ns, 8);
        write_le(info + 24, 1, 4);
        info[29] = 1;
        write_memory(context, state->system_time_page, info, sizeof(info));
        state->version = (state->version + 1u) & ~1u;
        write_le(info, state->version, 4);
        write_memory(context, state->system_time_page, info, 4);
    }
    if (state->has_wall_clock_page) {
        uint64_t wall_clock_ns = wall_time(context);
        uint8_t info[12] = {0};
        write_le(info + 4, (uint32_t)(wall_clock_ns / UINT64_C(1000000000)), 4);
        write_le(info + 8, wall_clock_ns % UINT64_C(1000000000), 4);
        write_memory(context, state->wall_clock_page, info, sizeof(info));
    }
}
