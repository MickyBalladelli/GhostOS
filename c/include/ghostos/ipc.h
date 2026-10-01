#ifndef GHOSTOS_IPC_H
#define GHOSTOS_IPC_H

#include "ghostos/capability.h"
#include <stdatomic.h>

#define GHOSTOS_IPC_MAX_CAPACITY 256u
#define GHOSTOS_IPC_MAX_CAPABILITIES GHOSTOS_MAX_CAPABILITIES

typedef struct { uint32_t raw; } ghostos_channel_id;
typedef struct { uint32_t raw; } ghostos_shared_region_id;
typedef struct { ghostos_shared_region_id region; uint32_t offset, length; bool writable; } ghostos_shared_buffer;
typedef struct { uint64_t correlation_low, correlation_high, label; bool has_buffer; ghostos_shared_buffer buffer; uint64_t words[4]; } ghostos_ipc_message;
typedef enum { GHOSTOS_IPC_OK, GHOSTOS_IPC_FULL, GHOSTOS_IPC_EMPTY, GHOSTOS_IPC_CLOSED,
    GHOSTOS_IPC_DEADLOCK, GHOSTOS_IPC_ACCESS_DENIED, GHOSTOS_IPC_CORE_ISOLATED,
    GHOSTOS_IPC_RATE_LIMITED } ghostos_ipc_error;
typedef enum { GHOSTOS_IPC_EVENT_SEND=1, GHOSTOS_IPC_EVENT_RECEIVE, GHOSTOS_IPC_EVENT_FULL,
    GHOSTOS_IPC_EVENT_RATE_LIMITED, GHOSTOS_IPC_EVENT_CLOSED } ghostos_ipc_event_kind;
typedef void (*ghostos_ipc_event_fn)(void *context, ghostos_ipc_event_kind kind,
    uint32_t channel, uint64_t value, ghostos_ipc_message message);
typedef struct {
    atomic_size_t sequence;
    atomic_uint_fast64_t correlation_low, correlation_high, label;
    atomic_bool buffer_present, buffer_writable;
    atomic_uint buffer_region, buffer_offset, buffer_length;
    atomic_uint_fast64_t words[4];
} ghostos_ipc_slot;
typedef struct {
    ghostos_channel_id id;
    size_t capacity;
    atomic_size_t enqueue_position, dequeue_position;
    atomic_bool closed;
    atomic_size_t high_watermark;
    atomic_uint_fast64_t enqueued, dequeued, full_events, rate_limited_events;
    atomic_uint_fast64_t last_enqueue_us, last_dequeue_us, last_correlation_low, last_correlation_high, last_label;
    atomic_uint_fast64_t generated_correlation;
    ghostos_ipc_slot slots[GHOSTOS_IPC_MAX_CAPACITY];
    ghostos_ipc_event_fn event;
    void *event_context;
} ghostos_ipc_channel;
typedef struct { uint64_t revoked_capabilities; size_t discarded_messages; bool channel_closed; } ghostos_ipc_close_report;
typedef struct { size_t endpoints; uint64_t revoked_capabilities; size_t discarded_messages; } ghostos_ipc_cleanup_report;
typedef struct {
    ghostos_channel_id channel; bool closed; size_t pending, high_watermark;
    uint64_t enqueued, dequeued, full_events, rate_limited_events, last_enqueue_us, last_dequeue_us;
} ghostos_ipc_diagnostics;
typedef struct {
    ghostos_channel_id channel; size_t pending; uint64_t stalled_for_us, full_events;
    uint64_t correlation_low, correlation_high, label;
} ghostos_ipc_stuck_report;
typedef bool (*ghostos_ipc_accepts_cpu_fn)(void *context, uint8_t cpu);
typedef enum { GHOSTOS_IPC_SCHED_OK, GHOSTOS_IPC_SCHED_DEADLOCK, GHOSTOS_IPC_SCHED_ERROR } ghostos_ipc_sched_result;
typedef ghostos_ipc_sched_result (*ghostos_ipc_wait_fn)(void *context, uint32_t channel, uint32_t owner_thread, uint32_t waiter_thread);
typedef void (*ghostos_ipc_complete_fn)(void *context, uint32_t channel, uint32_t waiter_thread);
typedef struct { void *context; ghostos_ipc_wait_fn wait; ghostos_ipc_complete_fn complete; } ghostos_ipc_scheduler;
typedef struct {
    ghostos_ipc_channel *channel; ghostos_capability_space *capabilities;
    uint32_t caller; ghostos_capability_handle endpoint; ghostos_shared_region_id region;
    bool sender;
} ghostos_ipc_mapped_endpoint;

bool ghostos_channel_id_new(uint32_t raw, ghostos_channel_id *out);
bool ghostos_shared_region_id_new(uint32_t raw, ghostos_shared_region_id *out);
bool ghostos_ipc_channel_init(ghostos_ipc_channel *channel, ghostos_channel_id id,
    size_t capacity, ghostos_ipc_event_fn event, void *event_context);
ghostos_ipc_error ghostos_ipc_try_send(ghostos_ipc_channel *channel, ghostos_capability_space *capabilities,
    uint32_t caller, ghostos_capability_handle endpoint, bool has_buffer_authority,
    ghostos_capability_handle buffer_authority, uint64_t now_us, ghostos_ipc_message message,
    uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_try_receive(ghostos_ipc_channel *channel, ghostos_capability_space *capabilities,
    uint32_t caller, ghostos_capability_handle endpoint, uint64_t now_us,
    ghostos_ipc_message *message, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_try_send_delegated(ghostos_ipc_channel *channel,
    ghostos_capability_space *capabilities, uint32_t caller, ghostos_capability_handle endpoint,
    bool has_buffer_authority, ghostos_capability_handle buffer_authority,
    ghostos_capability_handle source, uint32_t receiver, uint16_t rights, uint64_t now_us,
    ghostos_ipc_message message, ghostos_capability_handle *delegated, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_send_on(ghostos_ipc_channel *channel, ghostos_capability_space *capabilities,
    uint32_t caller, ghostos_capability_handle endpoint, bool has_buffer_authority,
    ghostos_capability_handle buffer_authority, uint8_t cpu, ghostos_ipc_accepts_cpu_fn accepts,
    void *partition_context, uint64_t now_us, ghostos_ipc_message message, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_receive_on(ghostos_ipc_channel *channel, ghostos_capability_space *capabilities,
    uint32_t caller, ghostos_capability_handle endpoint, uint8_t cpu, ghostos_ipc_accepts_cpu_fn accepts,
    void *partition_context, uint64_t now_us, ghostos_ipc_message *message, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_send_with_priority(ghostos_ipc_channel *channel,
    ghostos_capability_space *capabilities, ghostos_ipc_scheduler scheduler,
    uint32_t owner_thread, uint32_t waiter_thread, uint32_t caller,
    ghostos_capability_handle endpoint, bool has_buffer_authority,
    ghostos_capability_handle buffer_authority, uint64_t now_us,
    ghostos_ipc_message message, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_map_sender(ghostos_ipc_channel *channel,
    ghostos_capability_space *capabilities, uint32_t caller, ghostos_capability_handle endpoint,
    ghostos_capability_handle ring_memory, ghostos_shared_region_id region,
    ghostos_ipc_mapped_endpoint *mapped);
ghostos_ipc_error ghostos_ipc_map_receiver(ghostos_ipc_channel *channel,
    ghostos_capability_space *capabilities, uint32_t caller, ghostos_capability_handle endpoint,
    ghostos_capability_handle ring_memory, ghostos_shared_region_id region,
    ghostos_ipc_mapped_endpoint *mapped);
ghostos_ipc_error ghostos_ipc_mapped_send(ghostos_ipc_mapped_endpoint *mapped, uint64_t now_us,
    ghostos_ipc_message message, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_mapped_receive(ghostos_ipc_mapped_endpoint *mapped, uint64_t now_us,
    ghostos_ipc_message *message, uint64_t *retry_after_us);
ghostos_ipc_error ghostos_ipc_close_endpoint(ghostos_ipc_channel *channel,
    ghostos_capability_space *capabilities, uint32_t caller, ghostos_capability_handle endpoint,
    ghostos_ipc_close_report *report);
ghostos_ipc_cleanup_report ghostos_ipc_cleanup_owner(ghostos_ipc_channel *channel,
    ghostos_capability_space *capabilities, uint32_t owner);
size_t ghostos_ipc_pending(const ghostos_ipc_channel *channel);
ghostos_ipc_diagnostics ghostos_ipc_get_diagnostics(const ghostos_ipc_channel *channel);
bool ghostos_ipc_get_stuck_report(const ghostos_ipc_channel *channel, uint64_t now_us,
    uint64_t threshold_us, ghostos_ipc_stuck_report *report);
bool ghostos_ipc_check_invariants(const ghostos_ipc_channel *channel,
    const ghostos_capability_space *capabilities, uint32_t caller,
    ghostos_capability_handle endpoint, uint16_t required_rights);

#endif
