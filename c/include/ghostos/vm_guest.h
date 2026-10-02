#ifndef GHOSTOS_VM_GUEST_H
#define GHOSTOS_VM_GUEST_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_GUEST_AGENT_BASE UINT64_C(0xfebf0000)
#define GHOSTOS_VM_MEMORY_HOTPLUG_BASE UINT64_C(0xfebe0000)
#define GHOSTOS_VM_KVM_SYSTEM_TIME_NEW UINT32_C(0x4b564d01)
#define GHOSTOS_VM_KVM_WALL_CLOCK_NEW UINT32_C(0x4b564d00)

typedef struct ghostos_vm_guest_agent ghostos_vm_guest_agent;
typedef struct {
    uint32_t kind;
    uint64_t base, size;
} ghostos_vm_guest_event;

ghostos_vm_guest_agent *ghostos_vm_guest_agent_new(void);
void ghostos_vm_guest_agent_free(ghostos_vm_guest_agent *agent);
void ghostos_vm_guest_agent_clear(ghostos_vm_guest_agent *agent);
bool ghostos_vm_guest_agent_send(ghostos_vm_guest_agent *agent,
    const uint8_t *bytes, size_t length);
size_t ghostos_vm_guest_agent_output_length(const ghostos_vm_guest_agent *agent);
size_t ghostos_vm_guest_agent_take_output(ghostos_vm_guest_agent *agent,
    uint8_t *output, size_t capacity);
bool ghostos_vm_guest_agent_notify(ghostos_vm_guest_agent *agent,
    const ghostos_vm_guest_event *event);
size_t ghostos_vm_guest_agent_pending_events(const ghostos_vm_guest_agent *agent);
/* Results: 0 success, 1 unsupported size, 2 invalid address,
 * 3 allocation failure, 4 access denied. Agent unknown registers retain
 * the existing unsupported-size error. */
uint8_t ghostos_vm_guest_agent_read(ghostos_vm_guest_agent *agent,
    uint64_t address, uint8_t size, uint64_t *value);
uint8_t ghostos_vm_guest_agent_write(ghostos_vm_guest_agent *agent,
    uint64_t address, uint64_t value, uint8_t size);

typedef struct {
    uint64_t current, maximum, pending_base, pending_size, request;
    bool has_pending, has_request;
} ghostos_vm_memory_hotplug;

void ghostos_vm_memory_hotplug_init(ghostos_vm_memory_hotplug *state,
    uint64_t current, uint64_t maximum);
bool ghostos_vm_memory_hotplug_add(ghostos_vm_memory_hotplug *state,
    uint64_t base, uint64_t size);
bool ghostos_vm_memory_hotplug_take_request(ghostos_vm_memory_hotplug *state,
    uint64_t *request);
void ghostos_vm_memory_hotplug_clear_pending(ghostos_vm_memory_hotplug *state);
void ghostos_vm_memory_hotplug_reset(ghostos_vm_memory_hotplug *state);
uint8_t ghostos_vm_memory_hotplug_read(const ghostos_vm_memory_hotplug *state,
    uint64_t address, uint8_t size, uint64_t *value);
uint8_t ghostos_vm_memory_hotplug_write(ghostos_vm_memory_hotplug *state,
    uint64_t address, uint64_t value, uint8_t size);

typedef struct {
    uint64_t system_time_page, wall_clock_page;
    uint32_t version;
    bool has_system_time_page, has_wall_clock_page;
} ghostos_vm_pvclock;

typedef void (*ghostos_vm_guest_memory_write)(void *context, uint64_t address,
    const uint8_t *bytes, size_t length);
typedef uint64_t (*ghostos_vm_guest_wall_time)(void *context);
void ghostos_vm_pvclock_reset(ghostos_vm_pvclock *state);
void ghostos_vm_pvclock_write_msr(ghostos_vm_pvclock *state, uint32_t msr, uint64_t value);
bool ghostos_vm_pvclock_configured(const ghostos_vm_pvclock *state);
/* Callback is synchronous, never retained. Host/replay supplies wall time.
 * Memory-write failures are ignored, matching the existing device model. */
void ghostos_vm_pvclock_update(ghostos_vm_pvclock *state, uint64_t monotonic_ns,
    ghostos_vm_guest_wall_time wall_time, ghostos_vm_guest_memory_write write_memory,
    void *context);

#endif
