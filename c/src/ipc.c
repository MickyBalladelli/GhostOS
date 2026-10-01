#include "ghostos/ipc.h"

static uint64_t sat_add(uint64_t a, uint64_t b) { return UINT64_MAX - a < b ? UINT64_MAX : a + b; }
bool ghostos_channel_id_new(uint32_t raw, ghostos_channel_id *out) {
    if (!raw) return false;
    if (out) out->raw = raw;
    return true;
}
bool ghostos_shared_region_id_new(uint32_t raw, ghostos_shared_region_id *out) {
    if (!raw) return false;
    if (out) out->raw = raw;
    return true;
}

bool ghostos_ipc_channel_init(ghostos_ipc_channel *c, ghostos_channel_id id,
    size_t capacity, ghostos_ipc_event_fn event, void *event_context) {
    if (!id.raw || capacity < 2 || capacity > GHOSTOS_IPC_MAX_CAPACITY) return false;
    c->id = id;
    c->capacity = capacity;
    atomic_init(&c->enqueue_position, 0);
    atomic_init(&c->dequeue_position, 0);
    atomic_init(&c->closed, false);
    atomic_init(&c->high_watermark, 0);
    atomic_init(&c->enqueued, 0); atomic_init(&c->dequeued, 0);
    atomic_init(&c->full_events, 0); atomic_init(&c->rate_limited_events, 0);
    atomic_init(&c->last_enqueue_us, 0); atomic_init(&c->last_dequeue_us, 0);
    atomic_init(&c->last_correlation_low, 0); atomic_init(&c->last_correlation_high, 0);
    atomic_init(&c->last_label, 0); atomic_init(&c->generated_correlation, 0);
    c->event = event; c->event_context = event_context;
    for (size_t i = 0; i < capacity; ++i) {
        ghostos_ipc_slot *s = &c->slots[i];
        atomic_init(&s->sequence, i);
        atomic_init(&s->correlation_low, 0); atomic_init(&s->correlation_high, 0);
        atomic_init(&s->label, 0); atomic_init(&s->buffer_present, false);
        atomic_init(&s->buffer_writable, false); atomic_init(&s->buffer_region, 0);
        atomic_init(&s->buffer_offset, 0); atomic_init(&s->buffer_length, 0);
        for (size_t w = 0; w < 4; ++w) atomic_init(&s->words[w], 0);
    }
    return true;
}

static void write_slot(ghostos_ipc_slot *s, ghostos_ipc_message m) {
    atomic_store_explicit(&s->correlation_low, m.correlation_low, memory_order_relaxed);
    atomic_store_explicit(&s->correlation_high, m.correlation_high, memory_order_relaxed);
    atomic_store_explicit(&s->label, m.label, memory_order_relaxed);
    atomic_store_explicit(&s->buffer_present, m.has_buffer, memory_order_relaxed);
    if (m.has_buffer) {
        atomic_store_explicit(&s->buffer_region, m.buffer.region.raw, memory_order_relaxed);
        atomic_store_explicit(&s->buffer_offset, m.buffer.offset, memory_order_relaxed);
        atomic_store_explicit(&s->buffer_length, m.buffer.length, memory_order_relaxed);
        atomic_store_explicit(&s->buffer_writable, m.buffer.writable, memory_order_relaxed);
    }
    for (size_t w = 0; w < 4; ++w) atomic_store_explicit(&s->words[w], m.words[w], memory_order_relaxed);
}

static ghostos_ipc_message read_slot(ghostos_ipc_slot *s) {
    ghostos_ipc_message m = {0};
    m.correlation_low = atomic_load_explicit(&s->correlation_low, memory_order_relaxed);
    m.correlation_high = atomic_load_explicit(&s->correlation_high, memory_order_relaxed);
    m.label = atomic_load_explicit(&s->label, memory_order_relaxed);
    m.has_buffer = atomic_load_explicit(&s->buffer_present, memory_order_relaxed);
    if (m.has_buffer) {
        m.buffer.region.raw = atomic_load_explicit(&s->buffer_region, memory_order_relaxed);
        m.buffer.offset = atomic_load_explicit(&s->buffer_offset, memory_order_relaxed);
        m.buffer.length = atomic_load_explicit(&s->buffer_length, memory_order_relaxed);
        m.buffer.writable = atomic_load_explicit(&s->buffer_writable, memory_order_relaxed);
    }
    for (size_t w = 0; w < 4; ++w) m.words[w] = atomic_load_explicit(&s->words[w], memory_order_relaxed);
    return m;
}

static bool ring_send(ghostos_ipc_channel *c, ghostos_ipc_message m) {
    size_t pos = atomic_load_explicit(&c->enqueue_position, memory_order_relaxed);
    ghostos_ipc_slot *slot;
    for (;;) {
        slot = &c->slots[pos % c->capacity];
        size_t seq = atomic_load_explicit(&slot->sequence, memory_order_acquire);
        intptr_t diff = (intptr_t)seq - (intptr_t)pos;
        if (diff == 0) {
            if (atomic_compare_exchange_weak_explicit(&c->enqueue_position, &pos, pos + 1,
                    memory_order_relaxed, memory_order_relaxed)) break;
        } else if (diff < 0) return false;
        else pos = atomic_load_explicit(&c->enqueue_position, memory_order_relaxed);
    }
    write_slot(slot, m);
    atomic_store_explicit(&slot->sequence, pos + 1, memory_order_release);
    return true;
}

static bool ring_receive(ghostos_ipc_channel *c, ghostos_ipc_message *m) {
    size_t pos = atomic_load_explicit(&c->dequeue_position, memory_order_relaxed);
    ghostos_ipc_slot *slot;
    for (;;) {
        slot = &c->slots[pos % c->capacity];
        size_t seq = atomic_load_explicit(&slot->sequence, memory_order_acquire);
        intptr_t diff = (intptr_t)seq - (intptr_t)(pos + 1);
        if (diff == 0) {
            if (atomic_compare_exchange_weak_explicit(&c->dequeue_position, &pos, pos + 1,
                    memory_order_relaxed, memory_order_relaxed)) break;
        } else if (diff < 0) return false;
        else pos = atomic_load_explicit(&c->dequeue_position, memory_order_relaxed);
    }
    *m = read_slot(slot);
    atomic_store_explicit(&slot->sequence, pos + c->capacity, memory_order_release);
    return true;
}

size_t ghostos_ipc_pending(const ghostos_ipc_channel *c) {
    size_t enqueued = atomic_load_explicit(&c->enqueue_position, memory_order_acquire);
    size_t dequeued = atomic_load_explicit(&c->dequeue_position, memory_order_acquire);
    return enqueued - dequeued;
}

static ghostos_ipc_error rate_limited(ghostos_ipc_channel *c, uint64_t delay, uint64_t *retry) {
    atomic_fetch_add_explicit(&c->rate_limited_events, 1, memory_order_relaxed);
    if (retry) *retry = delay;
    if (c->event) c->event(c->event_context, GHOSTOS_IPC_EVENT_RATE_LIMITED, c->id.raw, delay, (ghostos_ipc_message){0});
    return GHOSTOS_IPC_RATE_LIMITED;
}

static ghostos_ipc_error charge(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, uint64_t now, uint64_t *retry) {
    ghostos_quota_result result;
    if (ghostos_capability_consume_quota(caps, caller, endpoint, GHOSTOS_QUOTA_IPC_MESSAGES,
            now, 1, &result) != GHOSTOS_CAP_OK) return GHOSTOS_IPC_ACCESS_DENIED;
    if (result.decision == GHOSTOS_CAP_QUOTA_THROTTLED) return rate_limited(c, result.retry_after_us, retry);
    if (result.decision == GHOSTOS_CAP_QUOTA_REJECTED) return rate_limited(c, UINT64_MAX, retry);
    return GHOSTOS_IPC_OK;
}

static bool authorize_endpoint(const ghostos_ipc_channel *c, const ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, uint16_t rights) {
    return ghostos_capability_authorize(caps, caller, endpoint,
        ghostos_capability_object_make(GHOSTOS_OBJECT_IPC_CHANNEL, 0, c->id.raw, 0, 0), rights) == GHOSTOS_CAP_OK;
}

static bool authorize_buffer(const ghostos_capability_space *caps, uint32_t caller,
    bool has_authority, ghostos_capability_handle authority, ghostos_ipc_message message) {
    return !message.has_buffer || (has_authority && ghostos_capability_authorize_mapping(caps, caller,
        authority, message.buffer.region.raw, message.buffer.writable, false) == GHOSTOS_CAP_OK);
}

static ghostos_ipc_error enqueue(ghostos_ipc_channel *c, ghostos_ipc_message m, uint64_t now) {
    if (atomic_load_explicit(&c->closed, memory_order_acquire)) return GHOSTOS_IPC_CLOSED;
    if (m.correlation_low == 0 && m.correlation_high == 0) {
        m.correlation_low = atomic_fetch_add_explicit(&c->generated_correlation, 1, memory_order_relaxed) + 1;
        m.correlation_high = 0;
    }
    if (!ring_send(c, m)) {
        atomic_fetch_add_explicit(&c->full_events, 1, memory_order_relaxed);
        if (c->event) c->event(c->event_context, GHOSTOS_IPC_EVENT_FULL, c->id.raw, 3, m);
        return GHOSTOS_IPC_FULL;
    }
    atomic_fetch_add_explicit(&c->enqueued, 1, memory_order_relaxed);
    atomic_store_explicit(&c->last_enqueue_us, now, memory_order_release);
    atomic_store_explicit(&c->last_correlation_low, m.correlation_low, memory_order_relaxed);
    atomic_store_explicit(&c->last_correlation_high, m.correlation_high, memory_order_relaxed);
    atomic_store_explicit(&c->last_label, m.label, memory_order_release);
    size_t pending = ghostos_ipc_pending(c), high = atomic_load_explicit(&c->high_watermark, memory_order_relaxed);
    while (pending > high && !atomic_compare_exchange_weak_explicit(&c->high_watermark, &high, pending,
            memory_order_relaxed, memory_order_relaxed)) {}
    if (c->event) c->event(c->event_context, GHOSTOS_IPC_EVENT_SEND, c->id.raw, m.correlation_low, m);
    return GHOSTOS_IPC_OK;
}

static ghostos_ipc_error dequeue(ghostos_ipc_channel *c, uint64_t now, ghostos_ipc_message *out) {
    if (atomic_load_explicit(&c->closed, memory_order_acquire)) return GHOSTOS_IPC_CLOSED;
    if (!ring_receive(c, out)) return GHOSTOS_IPC_EMPTY;
    atomic_fetch_add_explicit(&c->dequeued, 1, memory_order_relaxed);
    atomic_store_explicit(&c->last_dequeue_us, now, memory_order_release);
    if (c->event) c->event(c->event_context, GHOSTOS_IPC_EVENT_RECEIVE, c->id.raw, out->correlation_low, *out);
    return GHOSTOS_IPC_OK;
}

ghostos_ipc_error ghostos_ipc_try_send(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, bool has_buffer_authority,
    ghostos_capability_handle buffer_authority, uint64_t now, ghostos_ipc_message message, uint64_t *retry) {
    if (!authorize_endpoint(c, caps, caller, endpoint, GHOSTOS_RIGHT_SEND) ||
            !authorize_buffer(caps, caller, has_buffer_authority, buffer_authority, message)) return GHOSTOS_IPC_ACCESS_DENIED;
    ghostos_ipc_error e = charge(c, caps, caller, endpoint, now, retry);
    if (e != GHOSTOS_IPC_OK) return e;
    e = enqueue(c, message, now);
    if (e != GHOSTOS_IPC_OK) (void)ghostos_capability_refund_quota(caps, caller, endpoint, GHOSTOS_QUOTA_IPC_MESSAGES, 1);
    return e;
}

ghostos_ipc_error ghostos_ipc_try_receive(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, uint64_t now,
    ghostos_ipc_message *message, uint64_t *retry) {
    if (!authorize_endpoint(c, caps, caller, endpoint, GHOSTOS_RIGHT_RECEIVE)) return GHOSTOS_IPC_ACCESS_DENIED;
    ghostos_ipc_error e = charge(c, caps, caller, endpoint, now, retry);
    if (e != GHOSTOS_IPC_OK) return e;
    e = dequeue(c, now, message);
    if (e != GHOSTOS_IPC_OK) (void)ghostos_capability_refund_quota(caps, caller, endpoint, GHOSTOS_QUOTA_IPC_MESSAGES, 1);
    return e;
}

ghostos_ipc_error ghostos_ipc_try_send_delegated(ghostos_ipc_channel *c,
    ghostos_capability_space *caps, uint32_t caller, ghostos_capability_handle endpoint,
    bool has_buffer_authority, ghostos_capability_handle buffer_authority,
    ghostos_capability_handle source, uint32_t receiver, uint16_t rights, uint64_t now,
    ghostos_ipc_message message, ghostos_capability_handle *delegated, uint64_t *retry) {
    if (!authorize_endpoint(c, caps, caller, endpoint, GHOSTOS_RIGHT_SEND) ||
            !authorize_buffer(caps, caller, has_buffer_authority, buffer_authority, message)) return GHOSTOS_IPC_ACCESS_DENIED;
    ghostos_capability_handle child;
    if (ghostos_capability_delegate(caps, caller, source, receiver, rights, &child) != GHOSTOS_CAP_OK) return GHOSTOS_IPC_ACCESS_DENIED;
    message.words[3] = child;
    ghostos_ipc_error e = charge(c, caps, caller, endpoint, now, retry);
    if (e != GHOSTOS_IPC_OK) { (void)ghostos_capability_delete(caps, receiver, child, 0); return e; }
    e = enqueue(c, message, now);
    if (e != GHOSTOS_IPC_OK) {
        (void)ghostos_capability_delete(caps, receiver, child, 0);
        (void)ghostos_capability_refund_quota(caps, caller, endpoint, GHOSTOS_QUOTA_IPC_MESSAGES, 1);
        return e;
    }
    if (delegated) *delegated = child;
    return GHOSTOS_IPC_OK;
}

ghostos_ipc_error ghostos_ipc_send_on(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, bool has_buffer_authority,
    ghostos_capability_handle buffer_authority, uint8_t cpu, ghostos_ipc_accepts_cpu_fn accepts,
    void *ctx, uint64_t now, ghostos_ipc_message message, uint64_t *retry) {
    if (!accepts || !accepts(ctx, cpu)) return GHOSTOS_IPC_CORE_ISOLATED;
    return ghostos_ipc_try_send(c, caps, caller, endpoint, has_buffer_authority, buffer_authority, now, message, retry);
}
ghostos_ipc_error ghostos_ipc_receive_on(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, uint8_t cpu, ghostos_ipc_accepts_cpu_fn accepts,
    void *ctx, uint64_t now, ghostos_ipc_message *message, uint64_t *retry) {
    if (!accepts || !accepts(ctx, cpu)) return GHOSTOS_IPC_CORE_ISOLATED;
    return ghostos_ipc_try_receive(c, caps, caller, endpoint, now, message, retry);
}
ghostos_ipc_error ghostos_ipc_send_with_priority(ghostos_ipc_channel *c,
    ghostos_capability_space *caps, ghostos_ipc_scheduler scheduler, uint32_t owner_thread,
    uint32_t waiter_thread, uint32_t caller, ghostos_capability_handle endpoint,
    bool has_buffer_authority, ghostos_capability_handle buffer_authority, uint64_t now,
    ghostos_ipc_message message, uint64_t *retry) {
    ghostos_ipc_error e = ghostos_ipc_try_send(c, caps, caller, endpoint, has_buffer_authority, buffer_authority, now, message, retry);
    if (e == GHOSTOS_IPC_OK) { if (scheduler.complete) scheduler.complete(scheduler.context, c->id.raw, waiter_thread); }
    else if (e == GHOSTOS_IPC_FULL && scheduler.wait) {
        ghostos_ipc_sched_result r = scheduler.wait(scheduler.context, c->id.raw, owner_thread, waiter_thread);
        if (r == GHOSTOS_IPC_SCHED_DEADLOCK) return GHOSTOS_IPC_DEADLOCK;
    }
    return e;
}

static ghostos_ipc_error map_endpoint(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, ghostos_capability_handle memory,
    ghostos_shared_region_id region, bool sender, ghostos_ipc_mapped_endpoint *out) {
    uint16_t right = sender ? GHOSTOS_RIGHT_SEND : GHOSTOS_RIGHT_RECEIVE;
    if (!region.raw || !authorize_endpoint(c, caps, caller, endpoint, right) ||
        ghostos_capability_authorize_mapping(caps, caller, memory, region.raw, sender, false) != GHOSTOS_CAP_OK)
        return GHOSTOS_IPC_ACCESS_DENIED;
    *out = (ghostos_ipc_mapped_endpoint){c, caps, caller, endpoint, region, sender};
    return GHOSTOS_IPC_OK;
}
ghostos_ipc_error ghostos_ipc_map_sender(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, ghostos_capability_handle memory,
    ghostos_shared_region_id region, ghostos_ipc_mapped_endpoint *mapped) {
    return map_endpoint(c,caps,caller,endpoint,memory,region,true,mapped);
}
ghostos_ipc_error ghostos_ipc_map_receiver(ghostos_ipc_channel *c, ghostos_capability_space *caps,
    uint32_t caller, ghostos_capability_handle endpoint, ghostos_capability_handle memory,
    ghostos_shared_region_id region, ghostos_ipc_mapped_endpoint *mapped) {
    return map_endpoint(c,caps,caller,endpoint,memory,region,false,mapped);
}
ghostos_ipc_error ghostos_ipc_mapped_send(ghostos_ipc_mapped_endpoint *m, uint64_t now,
    ghostos_ipc_message message, uint64_t *retry) {
    if (!m || !m->sender || (message.has_buffer && message.buffer.region.raw != m->region.raw)) return GHOSTOS_IPC_ACCESS_DENIED;
    ghostos_ipc_error e = charge(m->channel,m->capabilities,m->caller,m->endpoint,now,retry);
    if (e != GHOSTOS_IPC_OK) return e;
    e = enqueue(m->channel,message,now);
    if (e != GHOSTOS_IPC_OK) (void)ghostos_capability_refund_quota(m->capabilities,m->caller,m->endpoint,GHOSTOS_QUOTA_IPC_MESSAGES,1);
    return e;
}
ghostos_ipc_error ghostos_ipc_mapped_receive(ghostos_ipc_mapped_endpoint *m, uint64_t now,
    ghostos_ipc_message *message, uint64_t *retry) {
    if (!m || m->sender) return GHOSTOS_IPC_ACCESS_DENIED;
    ghostos_ipc_error e = charge(m->channel,m->capabilities,m->caller,m->endpoint,now,retry);
    if (e != GHOSTOS_IPC_OK) return e;
    e = dequeue(m->channel,now,message);
    if (e != GHOSTOS_IPC_OK) (void)ghostos_capability_refund_quota(m->capabilities,m->caller,m->endpoint,GHOSTOS_QUOTA_IPC_MESSAGES,1);
    return e;
}

ghostos_ipc_error ghostos_ipc_close_endpoint(ghostos_ipc_channel *c,
    ghostos_capability_space *caps, uint32_t caller, ghostos_capability_handle endpoint,
    ghostos_ipc_close_report *report) {
    ghostos_capability_info info;
    if (ghostos_capability_inspect(caps,caller,endpoint,&info) != GHOSTOS_CAP_OK ||
        info.object.kind != GHOSTOS_OBJECT_IPC_CHANNEL || info.object.value32 != c->id.raw ||
        !(info.rights & (GHOSTOS_RIGHT_SEND | GHOSTOS_RIGHT_RECEIVE))) return GHOSTOS_IPC_ACCESS_DENIED;
    bool closes = (info.rights & GHOSTOS_RIGHT_RECEIVE) != 0;
    size_t revoked = 0;
    if (ghostos_capability_delete(caps,caller,endpoint,&revoked) != GHOSTOS_CAP_OK) return GHOSTOS_IPC_ACCESS_DENIED;
    size_t discarded = 0;
    if (closes && !atomic_exchange_explicit(&c->closed,true,memory_order_acq_rel)) {
        ghostos_ipc_message ignored;
        while (ring_receive(c,&ignored)) ++discarded;
        if (c->event) c->event(c->event_context,GHOSTOS_IPC_EVENT_CLOSED,c->id.raw,discarded,(ghostos_ipc_message){0});
    }
    if (report) *report = (ghostos_ipc_close_report){revoked,discarded,atomic_load_explicit(&c->closed,memory_order_acquire)};
    return GHOSTOS_IPC_OK;
}

typedef struct { uint32_t owner, channel; uint64_t handles[GHOSTOS_MAX_CAPABILITIES]; size_t count; } cleanup_context;
static bool collect_owner_endpoint(void *context, ghostos_capability_handle handle, ghostos_capability_info info) {
    cleanup_context *c = context;
    if (info.owner == c->owner && info.object.kind == GHOSTOS_OBJECT_IPC_CHANNEL &&
        info.object.value32 == c->channel && c->count < GHOSTOS_MAX_CAPABILITIES)
        c->handles[c->count++] = handle;
    return true;
}
ghostos_ipc_cleanup_report ghostos_ipc_cleanup_owner(ghostos_ipc_channel *c,
    ghostos_capability_space *caps, uint32_t owner) {
    cleanup_context found = {.owner=owner,.channel=c->id.raw};
    (void)ghostos_capability_entries(caps,collect_owner_endpoint,&found);
    ghostos_ipc_cleanup_report out = {0};
    for (size_t i = 0; i < found.count; ++i) {
        ghostos_ipc_close_report one;
        if (ghostos_ipc_close_endpoint(c,caps,owner,found.handles[i],&one) == GHOSTOS_IPC_OK) {
            ++out.endpoints; out.revoked_capabilities = sat_add(out.revoked_capabilities,one.revoked_capabilities);
            out.discarded_messages = sat_add(out.discarded_messages,one.discarded_messages);
        }
    }
    return out;
}

ghostos_ipc_diagnostics ghostos_ipc_get_diagnostics(const ghostos_ipc_channel *c) {
    return (ghostos_ipc_diagnostics){c->id, atomic_load_explicit(&c->closed,memory_order_acquire),
        ghostos_ipc_pending(c), atomic_load_explicit(&c->high_watermark,memory_order_relaxed),
        atomic_load_explicit(&c->enqueued,memory_order_relaxed), atomic_load_explicit(&c->dequeued,memory_order_relaxed),
        atomic_load_explicit(&c->full_events,memory_order_relaxed), atomic_load_explicit(&c->rate_limited_events,memory_order_relaxed),
        atomic_load_explicit(&c->last_enqueue_us,memory_order_acquire), atomic_load_explicit(&c->last_dequeue_us,memory_order_acquire)};
}
bool ghostos_ipc_get_stuck_report(const ghostos_ipc_channel *c, uint64_t now, uint64_t threshold,
    ghostos_ipc_stuck_report *report) {
    if (!threshold || atomic_load_explicit(&c->closed,memory_order_acquire)) return false;
    ghostos_ipc_diagnostics d = ghostos_ipc_get_diagnostics(c);
    if (!d.pending) return false;
    uint64_t progress = d.last_dequeue_us ? d.last_dequeue_us : d.last_enqueue_us;
    uint64_t stalled = now >= progress ? now-progress : 0;
    if (stalled < threshold) return false;
    if (report) *report = (ghostos_ipc_stuck_report){c->id,d.pending,stalled,d.full_events,
        atomic_load_explicit(&c->last_correlation_low,memory_order_relaxed),
        atomic_load_explicit(&c->last_correlation_high,memory_order_relaxed),
        atomic_load_explicit(&c->last_label,memory_order_acquire)};
    return true;
}
bool ghostos_ipc_check_invariants(const ghostos_ipc_channel *c,
    const ghostos_capability_space *caps, uint32_t caller,
    ghostos_capability_handle endpoint, uint16_t rights) {
    return c->id.raw && c->capacity >= 2 && c->capacity <= GHOSTOS_IPC_MAX_CAPACITY &&
        ghostos_ipc_pending(c) <= c->capacity && authorize_endpoint(c,caps,caller,endpoint,rights);
}
