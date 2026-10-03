#include "ghostos/time_sync.h"
#include "ghostos/memory.h"

#define SECOND UINT64_C(1000000000)
/* Portable signed magnitude arithmetic covers Rust's i128 intermediates.
 * Products fit 128 bits; clock division occurs before additions. */
typedef struct { uint64_t high, low; bool negative; } wide;
static wide unsigned_wide(uint64_t value) { return (wide){0, value, false}; }
static wide signed_wide(int64_t value) {
    return (wide){0, value < 0 ? 0 - (uint64_t)value : (uint64_t)value, value < 0};
}
static wide negate(wide value) { if (value.high || value.low) value.negative = !value.negative; return value; }
static wide add(wide a, wide b) {
    if (a.negative == b.negative) {
        uint64_t low = a.low + b.low;
        return (wide){a.high + b.high + (low < a.low), low, a.negative};
    }
    if (a.high < b.high || (a.high == b.high && a.low < b.low)) { wide swap = a; a = b; b = swap; }
    wide out = {a.high - b.high - (a.low < b.low), a.low - b.low, a.negative};
    if (!out.high && !out.low) out.negative = false;
    return out;
}
static wide multiply(wide value, uint64_t factor) {
    uint64_t a0 = (uint32_t)value.low, a1 = value.low >> 32;
    uint64_t b0 = (uint32_t)factor, b1 = factor >> 32;
    uint64_t p0 = a0 * b0, middle = a1 * b0 + (p0 >> 32);
    uint64_t carry = middle >> 32;
    middle = (uint32_t)middle + a0 * b1;
    return (wide){value.high * factor + a1 * b1 + carry + (middle >> 32),
        (middle << 32) | (uint32_t)p0, value.negative};
}
static wide divide(wide value, uint64_t divisor) {
    wide quotient = {0, 0, value.negative};
    uint64_t remainder = 0;
    for (unsigned bit = 128; bit-- > 0;) {
        uint64_t incoming = bit >= 64 ? (value.high >> (bit - 64)) & 1 : (value.low >> bit) & 1;
        bool overflow = (remainder >> 63) != 0;
        remainder = (remainder << 1) | incoming;
        if (overflow || remainder >= divisor) {
            remainder -= divisor;
            if (bit >= 64) quotient.high |= UINT64_C(1) << (bit - 64);
            else quotient.low |= UINT64_C(1) << bit;
        }
    }
    if (!quotient.high && !quotient.low) quotient.negative = false;
    return quotient;
}
static int64_t clamp(wide value) {
    if (value.negative) {
        if (value.high || value.low >= (UINT64_C(1) << 63)) return INT64_MIN;
        return -(int64_t)value.low;
    }
    return value.high || value.low > INT64_MAX ? INT64_MAX : (int64_t)value.low;
}
static int64_t sat_add(int64_t a, int64_t b) { return clamp(add(signed_wide(a), signed_wide(b))); }
static int64_t sat_sub(int64_t a, int64_t b) { return clamp(add(signed_wide(a), negate(signed_wide(b)))); }
static uint64_t sat_increment(uint64_t value) { return value == UINT64_MAX ? value : value + 1; }

uint64_t ghostos_manual_clock_now(const _Atomic uint64_t *clock) {
    return atomic_load_explicit(clock, memory_order_acquire);
}
void ghostos_manual_clock_set(_Atomic uint64_t *clock, uint64_t now) {
    uint64_t current = atomic_load_explicit(clock, memory_order_acquire);
    for (;;) {
        uint64_t next = current > now ? current : now;
        if (atomic_compare_exchange_weak_explicit(clock, &current, next, memory_order_acq_rel, memory_order_acquire)) return;
    }
}
void ghostos_manual_clock_advance(_Atomic uint64_t *clock, uint64_t delta) {
    uint64_t current = atomic_load_explicit(clock, memory_order_acquire);
    for (;;) {
        uint64_t next = delta > UINT64_MAX - current ? UINT64_MAX : current + delta;
        if (atomic_compare_exchange_weak_explicit(clock, &current, next, memory_order_acq_rel, memory_order_acquire)) return;
    }
}
uint32_t ghostos_time_timestamp_init(uint64_t seconds, uint32_t nanoseconds, ghostos_ptp_timestamp *out) {
    if (seconds > UINT64_C(0x0000ffffffffffff) || nanoseconds >= SECOND) return 2;
    *out = (ghostos_ptp_timestamp){seconds, nanoseconds}; return 0;
}
void ghostos_time_from_nanos(uint64_t value, ghostos_ptp_timestamp *out) {
    *out = (ghostos_ptp_timestamp){value / SECOND, (uint32_t)(value % SECOND)};
}
uint32_t ghostos_time_to_nanos(const ghostos_ptp_timestamp *value, uint64_t *out) {
    if (value->seconds > UINT64_MAX / SECOND) return 2;
    uint64_t base = value->seconds * SECOND;
    if (value->nanoseconds > UINT64_MAX - base) return 2;
    *out = base + value->nanoseconds;
    return 0;
}
static void put(uint8_t *bytes, uint64_t value, size_t size) {
    for (size_t i = 0; i < size; ++i) bytes[i] = (uint8_t)(value >> (8 * (size - i - 1)));
}
static uint64_t get(const uint8_t *bytes, size_t size) {
    uint64_t value = 0;
    for (size_t i = 0; i < size; ++i) value = (value << 8) | bytes[i];
    return value;
}
static void timestamp_put(uint8_t *bytes, ghostos_ptp_timestamp value) {
    put(bytes, value.seconds, 6); put(bytes + 6, value.nanoseconds, 4);
}
static uint32_t timestamp_get(const uint8_t *bytes, ghostos_ptp_timestamp *out) {
    *out = (ghostos_ptp_timestamp){get(bytes, 6), (uint32_t)get(bytes + 6, 4)};
    return out->nanoseconds >= SECOND ? 2 : 0;
}
static bool valid_kind(uint8_t kind) { return kind == 0 || kind == 8 || kind == 1 || kind == 9; }
uint32_t ghostos_ptp_encode(const ghostos_ptp_message *message, uint8_t *output, size_t capacity) {
    if (capacity < 64) return 4;
    if (!message->source || !message->target) return 1;
    if (!valid_kind(message->kind)) return 3;
    ghostos_memory_zero(output, 64);
    ghostos_memory_copy(output, "SPTP", 4);
    output[4] = 2; output[5] = message->kind;
    put(output + 8, message->sequence, 2); put(output + 10, message->source, 4);
    put(output + 14, message->target, 4); put(output + 18, (uint64_t)message->correction, 8);
    /* Encoding historically truncates seconds and accepts directly constructed
     * timestamps; validation is performed only by constructors/decoding. */
    timestamp_put(output + 26, message->first);
    if (message->kind == 9) timestamp_put(output + 36, message->second);
    return 0;
}
uint32_t ghostos_ptp_decode(const uint8_t *bytes, size_t length, ghostos_ptp_message *out) {
    if (length < 64 || !ghostos_memory_equal(bytes, "SPTP", 4) || bytes[4] != 2 || bytes[6] || bytes[7] || !valid_kind(bytes[5])) return 3;
    ghostos_ptp_message message = {.kind = bytes[5], .sequence = (uint16_t)get(bytes + 8, 2),
        .source = (uint32_t)get(bytes + 10, 4), .target = (uint32_t)get(bytes + 14, 4)};
    if (!message.source || !message.target) return 3;
    uint64_t correction = get(bytes + 18, 8);
    message.correction = correction <= INT64_MAX ? (int64_t)correction : -1 - (int64_t)(UINT64_MAX - correction);
    uint32_t error = timestamp_get(bytes + 26, &message.first);
    if (error) return error;
    if (message.kind == 9 && (error = timestamp_get(bytes + 36, &message.second))) return error;
    *out = message;
    return 0;
}
void ghostos_clock_discipline(ghostos_cluster_clock *clock, const ghostos_sync_measurement *measurement,
    uint64_t now, ghostos_clock_adjustment *out) {
    int64_t previous = clock->state.offset_ns;
    bool first = clock->state.samples == 0;
    int64_t offset = first ? measurement->offset_ns : sat_add(previous, sat_sub(measurement->offset_ns, previous) / 4);
    int64_t frequency = 0;
    if (!first && now > clock->state.last_update_ns) {
        frequency = clamp(divide(multiply(signed_wide(sat_sub(offset, previous)), SECOND), now - clock->state.last_update_ns));
        if (frequency < -500000) frequency = -500000;
        if (frequency > 500000) frequency = 500000;
    }
    uint64_t magnitude = measurement->offset_ns < 0 ? 0 - (uint64_t)measurement->offset_ns : (uint64_t)measurement->offset_ns;
    clock->state = (ghostos_clock_state){offset, frequency, measurement->path_delay_ns, now,
        clock->state.samples == UINT32_MAX ? UINT32_MAX : clock->state.samples + 1, true};
    *out = (ghostos_clock_adjustment){offset, frequency, first || magnitude > 1000000};
}
uint64_t ghostos_clock_corrected_time(ghostos_cluster_clock *clock, uint64_t local) {
    uint64_t elapsed = local > clock->state.last_update_ns ? local - clock->state.last_update_ns : 0;
    wide correction = divide(multiply(signed_wide(clock->state.frequency_ppb), elapsed), SECOND);
    wide corrected = add(add(unsigned_wide(local), signed_wide(clock->state.offset_ns)), correction);
    uint64_t value = corrected.negative ? 0 : corrected.high ? UINT64_MAX : corrected.low;
    uint64_t floor = sat_increment(clock->last_global_ns);
    clock->last_global_ns = value > floor ? value : floor;
    return clock->last_global_ns;
}
uint32_t ghostos_epoch_init(ghostos_epoch_counter *counter, uint32_t node, uint64_t initial) {
    if (!node) return 1;
    *counter = (ghostos_epoch_counter){node, 1, initial}; return 0;
}
uint32_t ghostos_epoch_issue(ghostos_epoch_counter *counter, uint64_t hardware, ghostos_epoch_stamp *out) {
    uint64_t floor = counter->counter > hardware ? counter->counter : hardware;
    if (floor == UINT64_MAX) return 10;
    counter->counter = floor + 1;
    *out = (ghostos_epoch_stamp){counter->epoch, counter->counter, counter->node}; return 0;
}
uint32_t ghostos_epoch_observe(ghostos_epoch_counter *counter, const ghostos_epoch_stamp *remote) {
    if (!remote->node) return 1;
    if (remote->epoch > counter->epoch) { counter->epoch = remote->epoch; counter->counter = remote->counter; }
    else if (remote->epoch == counter->epoch && remote->counter > counter->counter) counter->counter = remote->counter;
    return 0;
}
uint32_t ghostos_ptp_daemon_init(ghostos_ptp_daemon *daemon, uint32_t node, uint8_t role) {
    if (!node) return 1;
    if (role > 1) return 5;
    ghostos_memory_zero(daemon, sizeof(*daemon));
    daemon->node = node; daemon->role = role; daemon->next_sequence = 1;
    return ghostos_epoch_init(&daemon->epoch, node, 0);
}
static uint16_t take_sequence(ghostos_ptp_daemon *daemon) {
    uint16_t current = daemon->next_sequence;
    daemon->next_sequence = (uint16_t)(current + 1);
    if (!daemon->next_sequence) daemon->next_sequence = 1;
    return current;
}
uint32_t ghostos_ptp_sync(ghostos_ptp_daemon *daemon, uint32_t target, uint64_t tx,
    bool two_step, ghostos_ptp_message *out) {
    if (!target || target == daemon->node) return 1;
    if (daemon->role != 0) return 5;
    *out = (ghostos_ptp_message){.kind = 0, .sequence = take_sequence(daemon), .source = daemon->node, .target = target};
    if (!two_step) ghostos_time_from_nanos(tx, &out->first);
    return 0;
}
uint32_t ghostos_ptp_follow_up(const ghostos_ptp_daemon *daemon,
    const ghostos_ptp_message *sync, uint64_t tx, ghostos_ptp_message *out) {
    if (sync->kind != 0 || sync->source != daemon->node || sync->first.seconds || sync->first.nanoseconds) return 5;
    *out = *sync; out->kind = 8; ghostos_time_from_nanos(tx, &out->first); return 0;
}
uint32_t ghostos_ptp_receive_sync(ghostos_ptp_daemon *daemon, const ghostos_ptp_message *message, uint64_t rx) {
    if (message->kind != 0 || message->target != daemon->node || daemon->role != 1 || message->source == daemon->node) return 5;
    if (daemon->has_pending && daemon->pending.has_t3) return 8;
    bool has_t1 = message->first.seconds || message->first.nanoseconds;
    uint64_t t1 = 0;
    if (has_t1) { uint32_t error = ghostos_time_to_nanos(&message->first, &t1); if (error) return error; }
    daemon->pending = (ghostos_pending_exchange){.sequence = message->sequence, .master = message->source,
        .t1 = t1, .t2 = rx, .correction = message->correction, .has_t1 = has_t1, .has_t2 = true};
    daemon->has_pending = true; return 0;
}
uint32_t ghostos_ptp_receive_follow_up(ghostos_ptp_daemon *daemon, const ghostos_ptp_message *message) {
    if (message->kind != 8 || message->target != daemon->node) return 5;
    if (!daemon->has_pending) return 7;
    if (daemon->pending.sequence != message->sequence || daemon->pending.master != message->source) return 6;
    uint64_t t1; uint32_t error = ghostos_time_to_nanos(&message->first, &t1);
    if (error) return error;
    daemon->pending.t1 = t1; daemon->pending.has_t1 = true; daemon->pending.correction = message->correction; return 0;
}
uint32_t ghostos_ptp_delay_request(ghostos_ptp_daemon *daemon, uint64_t tx, ghostos_ptp_message *out) {
    if (daemon->role != 1) return 5;
    if (!daemon->has_pending || !daemon->pending.has_t1) return 7;
    daemon->pending.t3 = tx; daemon->pending.has_t3 = true;
    *out = (ghostos_ptp_message){.kind = 1, .sequence = daemon->pending.sequence,
        .source = daemon->node, .target = daemon->pending.master};
    ghostos_time_from_nanos(tx, &out->first); return 0;
}
uint32_t ghostos_ptp_receive_delay_request(const ghostos_ptp_daemon *daemon,
    const ghostos_ptp_message *message, uint64_t rx, ghostos_ptp_message *out) {
    if (message->kind != 1 || message->target != daemon->node || daemon->role != 0 || message->source == daemon->node) return 5;
    *out = (ghostos_ptp_message){.kind = 9, .sequence = message->sequence, .source = daemon->node,
        .target = message->source, .first = message->first, .correction = message->correction};
    ghostos_time_from_nanos(rx, &out->second); return 0;
}
uint32_t ghostos_ptp_receive_delay_response(ghostos_ptp_daemon *daemon,
    const ghostos_ptp_message *message, uint64_t rx, ghostos_sync_measurement *out) {
    if (message->kind != 9 || message->target != daemon->node || daemon->role != 1 || message->source == daemon->node) return 5;
    if (!daemon->has_pending) return 7;
    ghostos_pending_exchange pending = daemon->pending;
    if (pending.sequence != message->sequence || pending.master != message->source) return 6;
    if (!pending.has_t1 || !pending.has_t2 || !pending.has_t3) return 7;
    uint64_t request, t4;
    uint32_t error = ghostos_time_to_nanos(&message->first, &request);
    if (error) return error;
    if (request != pending.t3) return 6;
    /* The original consumes pending state before validating the receive timestamp. */
    daemon->has_pending = false;
    error = ghostos_time_to_nanos(&message->second, &t4);
    if (error) return error;
    wide forward = add(unsigned_wide(pending.t2), negate(unsigned_wide(pending.t1)));
    wide backward = add(unsigned_wide(t4), negate(unsigned_wide(pending.t3)));
    wide correction = signed_wide(sat_add(pending.correction, message->correction));
    *out = (ghostos_sync_measurement){message->sequence, message->source,
        clamp(divide(add(add(forward, negate(backward)), negate(correction)), 2)),
        clamp(divide(add(add(forward, backward), negate(correction)), 2))};
    ghostos_clock_adjustment adjustment;
    ghostos_clock_discipline(&daemon->clock, out, rx, &adjustment); return 0;
}
uint32_t ghostos_ptp_synchronized_time(ghostos_ptp_daemon *daemon, uint64_t local, uint64_t *out) {
    if (daemon->role == 1 && !daemon->clock.state.synchronized) return 9;
    *out = ghostos_clock_corrected_time(&daemon->clock, local); return 0;
}

#if UINTPTR_MAX == UINT64_MAX
_Static_assert(sizeof(_Atomic uint64_t) == 8 && _Alignof(_Atomic uint64_t) == 8, "manual clock ABI");
_Static_assert(sizeof(ghostos_ptp_timestamp) == 16, "PTP timestamp ABI");
_Static_assert(sizeof(ghostos_ptp_message) == 56, "PTP message ABI");
_Static_assert(sizeof(ghostos_pending_exchange) == 48, "PTP pending ABI");
_Static_assert(sizeof(ghostos_cluster_clock) == 48, "cluster clock ABI");
_Static_assert(sizeof(ghostos_ptp_daemon) == 136, "PTP daemon ABI");
_Static_assert(offsetof(ghostos_ptp_daemon, clock) == 64, "PTP clock offset");
_Static_assert(sizeof(ghostos_epoch_counter) == 24, "epoch counter ABI");
#endif
