#include "ghostos/vm_replay.h"
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(ghostos_vm_replay_event) == 56, "replay event ABI");
_Static_assert(offsetof(ghostos_vm_replay_event, kind) == 48, "replay kind ABI");
_Static_assert(sizeof(ghostos_vm_replay_error) == 24, "replay error ABI");

static uint64_t read64(const uint8_t *p) {
    uint64_t value = 0;
    for (size_t i = 0; i < 8; ++i) value |= (uint64_t)p[i] << (i * 8);
    return value;
}
static void write64(uint8_t *p, uint64_t value) {
    for (size_t i = 0; i < 8; ++i) p[i] = (uint8_t)(value >> (i * 8));
}
uint32_t ghostos_vm_replay_file_decode(const uint8_t *bytes, size_t length,
    uint64_t *count) {
    if (!bytes || !count || length < GHOSTOS_VM_REPLAY_FILE_HEADER ||
        length > GHOSTOS_VM_REPLAY_MAX_FILE || memcmp(bytes, "SYNVMRP1", 8) ||
        bytes[8] != 1 || bytes[9] != 0) return GHOSTOS_VM_REPLAY_CORRUPT;
    *count = read64(bytes + 12);
    return *count > GHOSTOS_VM_REPLAY_MAX_EVENTS ?
        GHOSTOS_VM_REPLAY_CAPACITY : GHOSTOS_VM_REPLAY_OK;
}
void ghostos_vm_replay_file_encode(uint8_t *bytes, uint64_t count) {
    if (!bytes) return;
    memset(bytes, 0, GHOSTOS_VM_REPLAY_FILE_HEADER);
    memcpy(bytes, "SYNVMRP1", 8);
    bytes[8] = 1;
    write64(bytes + 12, count);
}
uint32_t ghostos_vm_replay_event_decode(const uint8_t *bytes, size_t length,
    uint64_t sequence, ghostos_vm_replay_event *event) {
    if (!bytes || !event || length < GHOSTOS_VM_REPLAY_EVENT_HEADER ||
        bytes[0] < 1 || bytes[0] > 6 || read64(bytes + 8) != sequence)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    uint64_t payload = read64(bytes + 48);
    if (payload > GHOSTOS_VM_REPLAY_MAX_PAYLOAD)
        return GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE;
    if (payload > length - GHOSTOS_VM_REPLAY_EVENT_HEADER)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    event->kind = bytes[0];
    event->sequence = sequence;
    event->a = read64(bytes + 16);
    event->b = read64(bytes + 24);
    event->c = read64(bytes + 32);
    event->d = read64(bytes + 40);
    event->payload_length = payload;
    return GHOSTOS_VM_REPLAY_OK;
}
uint32_t ghostos_vm_replay_event_encode(uint8_t *bytes,
    const ghostos_vm_replay_event *event) {
    if (!bytes || !event || event->kind < 1 || event->kind > 6)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    if (event->payload_length > GHOSTOS_VM_REPLAY_MAX_PAYLOAD)
        return GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE;
    memset(bytes, 0, GHOSTOS_VM_REPLAY_EVENT_HEADER);
    bytes[0] = event->kind;
    write64(bytes + 8, event->sequence);
    write64(bytes + 16, event->a);
    write64(bytes + 24, event->b);
    write64(bytes + 32, event->c);
    write64(bytes + 40, event->d);
    write64(bytes + 48, event->payload_length);
    return GHOSTOS_VM_REPLAY_OK;
}

uint32_t ghostos_vm_replay_validate(const ghostos_vm_replay_view *events, size_t count) {
    if (count && !events) return GHOSTOS_VM_REPLAY_CORRUPT;
    for (size_t i = 0; i < count; ++i) {
        if (events[i].event.sequence != i ||
            events[i].event.payload_length > GHOSTOS_VM_REPLAY_MAX_PAYLOAD ||
            (events[i].event.payload_length && !events[i].payload) ||
            events[i].event.kind < 1 || events[i].event.kind > 6)
            return GHOSTOS_VM_REPLAY_CORRUPT;
    }
    return GHOSTOS_VM_REPLAY_OK;
}

uint32_t ghostos_vm_replay_file_size(const ghostos_vm_replay_view *events,
    size_t count, uint64_t *size) {
    uint32_t result = ghostos_vm_replay_validate(events, count);
    if (result) return result;
    if (!size) return GHOSTOS_VM_REPLAY_CORRUPT;
    uint64_t total = GHOSTOS_VM_REPLAY_FILE_HEADER;
    for (size_t i = 0; i < count; ++i) {
        uint64_t entry = GHOSTOS_VM_REPLAY_EVENT_HEADER + events[i].event.payload_length;
        if (entry > GHOSTOS_VM_REPLAY_MAX_FILE - total) return GHOSTOS_VM_REPLAY_CAPACITY;
        total += entry;
    }
    *size = total;
    return GHOSTOS_VM_REPLAY_OK;
}

static uint32_t read32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | (uint32_t)bytes[1] << 8 |
        (uint32_t)bytes[2] << 16 | (uint32_t)bytes[3] << 24;
}
static void write32(uint8_t *bytes, uint32_t value) {
    for (size_t i = 0; i < 4; ++i) bytes[i] = (uint8_t)(value >> (i * 8));
}

uint32_t ghostos_vm_replay_dma_encode(const ghostos_vm_replay_dma_write *writes,
    size_t count, uint8_t **output, size_t *length, uint64_t *total_bytes) {
    if ((count && !writes) || !output || !length || !total_bytes)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    *output = NULL;
    *length = 0;
    size_t required = 4;
    uint64_t total = 0;
    for (size_t i = 0; i < count; ++i) {
        if (writes[i].length && !writes[i].bytes) return GHOSTOS_VM_REPLAY_CORRUPT;
        if (writes[i].length > UINT32_MAX ||
            writes[i].length > GHOSTOS_VM_REPLAY_MAX_PAYLOAD - 12 ||
            required > GHOSTOS_VM_REPLAY_MAX_PAYLOAD - 12 - writes[i].length)
            return GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE;
        required += 12 + writes[i].length;
        total += writes[i].length;
    }
    uint8_t *bytes = malloc(required);
    if (!bytes) return GHOSTOS_VM_REPLAY_CAPACITY;
    write32(bytes, (uint32_t)count);
    size_t offset = 4;
    for (size_t i = 0; i < count; ++i) {
        write64(bytes + offset, writes[i].address);
        write32(bytes + offset + 8, (uint32_t)writes[i].length);
        if (writes[i].length) memcpy(bytes + offset + 12, writes[i].bytes, writes[i].length);
        offset += 12 + writes[i].length;
    }
    *output = bytes;
    *length = required;
    *total_bytes = total;
    return GHOSTOS_VM_REPLAY_OK;
}

uint32_t ghostos_vm_replay_dma_decode(const uint8_t *bytes, size_t length,
    ghostos_vm_replay_dma_sink sink, void *context) {
    if (!bytes || length < 4) return GHOSTOS_VM_REPLAY_CORRUPT;
    uint32_t count = read32(bytes);
    size_t offset = 4;
    for (uint32_t i = 0; i < count; ++i) {
        if (length - offset < 12) return GHOSTOS_VM_REPLAY_CORRUPT;
        uint64_t address = read64(bytes + offset);
        size_t payload = read32(bytes + offset + 8);
        offset += 12;
        if (payload > length - offset) return GHOSTOS_VM_REPLAY_CORRUPT;
        if (sink) sink(context, address, bytes + offset, payload);
        offset += payload;
    }
    return offset == length ? GHOSTOS_VM_REPLAY_OK : GHOSTOS_VM_REPLAY_CORRUPT;
}

void ghostos_vm_replay_bytes_free(uint8_t *bytes) { free(bytes); }

struct ghostos_vm_replay_session {
    ghostos_vm_replay_view *events;
    size_t count, capacity, cursor, max_events;
    uint32_t mode;
    ghostos_vm_replay_error error;
};

static void clear_events(ghostos_vm_replay_session *session) {
    for (size_t i = 0; i < session->count; ++i) free((void *)session->events[i].payload);
    session->count = 0;
}

ghostos_vm_replay_session *ghostos_vm_replay_session_new(size_t max_events) {
    ghostos_vm_replay_session *session = calloc(1, sizeof(*session));
    if (session) session->max_events = max_events ? max_events : 1;
    return session;
}

void ghostos_vm_replay_session_free(ghostos_vm_replay_session *session) {
    if (!session) return;
    clear_events(session);
    free(session->events);
    free(session);
}

uint32_t ghostos_vm_replay_mode_get(const ghostos_vm_replay_session *session) {
    return session ? session->mode : GHOSTOS_VM_REPLAY_DISABLED;
}
size_t ghostos_vm_replay_position(const ghostos_vm_replay_session *session) {
    return session ? session->cursor : 0;
}
size_t ghostos_vm_replay_pending(const ghostos_vm_replay_session *session) {
    return session ? session->count - session->cursor : 0;
}
size_t ghostos_vm_replay_length(const ghostos_vm_replay_session *session) {
    return session ? session->count : 0;
}

void ghostos_vm_replay_begin_recording(ghostos_vm_replay_session *session) {
    if (!session) return;
    clear_events(session);
    session->cursor = 0;
    session->error = (ghostos_vm_replay_error){0};
    session->mode = GHOSTOS_VM_REPLAY_RECORDING;
}

static bool reserve_events(ghostos_vm_replay_session *session, size_t count) {
    if (count <= session->capacity) return true;
    if (count > SIZE_MAX / sizeof(*session->events)) return false;
    size_t capacity = session->capacity ? session->capacity : 16;
    while (capacity < count) {
        if (capacity > SIZE_MAX / 2) { capacity = count; break; }
        capacity *= 2;
    }
    if (capacity > session->max_events) capacity = session->max_events;
    if (capacity > SIZE_MAX / sizeof(*session->events)) capacity = count;
    ghostos_vm_replay_view *events = realloc(session->events, capacity * sizeof(*events));
    if (!events) return false;
    session->events = events;
    session->capacity = capacity;
    return true;
}

/* Build replacement separately so rejected traces cannot alter the session. */
uint32_t ghostos_vm_replay_begin(ghostos_vm_replay_session *session,
    const ghostos_vm_replay_view *events, size_t count) {
    if (!session) return GHOSTOS_VM_REPLAY_CORRUPT;
    uint32_t result = ghostos_vm_replay_validate(events, count);
    if (result) return result;
    if (count > session->max_events) return GHOSTOS_VM_REPLAY_CAPACITY;
    ghostos_vm_replay_session replacement = { .max_events = session->max_events };
    if (!reserve_events(&replacement, count)) return GHOSTOS_VM_REPLAY_CAPACITY;
    for (size_t i = 0; i < count; ++i) {
        size_t length = (size_t)events[i].event.payload_length;
        uint8_t *copy = length ? malloc(length) : NULL;
        if (length && !copy) {
            clear_events(&replacement);
            free(replacement.events);
            return GHOSTOS_VM_REPLAY_CAPACITY;
        }
        if (length) memcpy(copy, events[i].payload, length);
        replacement.events[i] = events[i];
        replacement.events[i].payload = copy;
        ++replacement.count;
    }
    clear_events(session);
    free(session->events);
    *session = replacement;
    session->mode = GHOSTOS_VM_REPLAY_REPLAYING;
    return GHOSTOS_VM_REPLAY_OK;
}

void ghostos_vm_replay_stop(ghostos_vm_replay_session *session) {
    if (!session) return;
    session->mode = GHOSTOS_VM_REPLAY_DISABLED;
    session->cursor = 0;
    session->error = (ghostos_vm_replay_error){0};
}

bool ghostos_vm_replay_take_error(ghostos_vm_replay_session *session,
    ghostos_vm_replay_error *error) {
    if (!session || !error || !session->error.code) return false;
    *error = session->error;
    session->error = (ghostos_vm_replay_error){0};
    return true;
}
void ghostos_vm_replay_error_get(const ghostos_vm_replay_session *session,
    ghostos_vm_replay_error *error) {
    if (session && error) *error = session->error;
}
uint8_t ghostos_vm_replay_peek_kind(const ghostos_vm_replay_session *session) {
    return session && session->mode == GHOSTOS_VM_REPLAY_REPLAYING &&
        session->cursor < session->count ? session->events[session->cursor].event.kind : 0;
}
bool ghostos_vm_replay_event_get(const ghostos_vm_replay_session *session,
    size_t index, ghostos_vm_replay_view *view) {
    if (!session || !view || index >= session->count) return false;
    *view = session->events[index];
    return true;
}

static uint32_t fail(ghostos_vm_replay_session *session, uint32_t code,
    uint8_t expected, uint8_t actual, uint8_t kind, uint64_t sequence) {
    session->error = (ghostos_vm_replay_error){sequence, code, session->mode,
        expected, actual, kind};
    return code;
}

uint32_t ghostos_vm_replay_next(ghostos_vm_replay_session *session,
    uint8_t expected, ghostos_vm_replay_view *view) {
    if (!session || !view) return GHOSTOS_VM_REPLAY_CORRUPT;
    if (session->mode != GHOSTOS_VM_REPLAY_REPLAYING)
        return fail(session, GHOSTOS_VM_REPLAY_WRONG_MODE, 0, 0, 0, 0);
    if (session->cursor >= session->count)
        return fail(session, GHOSTOS_VM_REPLAY_END, expected, 0, 0, session->cursor);
    *view = session->events[session->cursor];
    if (view->event.kind != expected)
        return fail(session, GHOSTOS_VM_REPLAY_UNEXPECTED_KIND,
            expected, view->event.kind, 0, view->event.sequence);
    ++session->cursor;
    return GHOSTOS_VM_REPLAY_OK;
}

static uint32_t record_event(ghostos_vm_replay_session *session, uint8_t kind,
    uint64_t a, uint64_t b, uint64_t c, uint64_t d, const uint8_t *payload, size_t length) {
    if (session->mode != GHOSTOS_VM_REPLAY_RECORDING)
        return fail(session, GHOSTOS_VM_REPLAY_WRONG_MODE, 0, 0, 0, 0);
    if (session->count >= session->max_events)
        return fail(session, GHOSTOS_VM_REPLAY_CAPACITY, 0, 0, 0, 0);
    if (length > GHOSTOS_VM_REPLAY_MAX_PAYLOAD)
        return fail(session, GHOSTOS_VM_REPLAY_PAYLOAD_TOO_LARGE, 0, 0, 0, 0);
    uint8_t *copy = length ? malloc(length) : NULL;
    if ((length && !copy) || !reserve_events(session, session->count + 1)) {
        free(copy);
        return fail(session, GHOSTOS_VM_REPLAY_CAPACITY, 0, 0, 0, 0);
    }
    if (length) memcpy(copy, payload, length);
    session->events[session->count] = (ghostos_vm_replay_view){
        {session->count, a, b, c, d, length, kind}, copy};
    ++session->count;
    return GHOSTOS_VM_REPLAY_OK;
}

static uint32_t mismatch(ghostos_vm_replay_session *session,
    const ghostos_vm_replay_view *view) {
    return fail(session, GHOSTOS_VM_REPLAY_INPUT_MISMATCH, 0, 0,
        view->event.kind, view->event.sequence);
}

uint32_t ghostos_vm_replay_instruction_input(ghostos_vm_replay_session *session,
    uint64_t ip, uint64_t address, uint8_t size, uint64_t value, uint64_t *output) {
    if (!session || !output) return GHOSTOS_VM_REPLAY_CORRUPT;
    *output = value;
    if (session->mode == GHOSTOS_VM_REPLAY_DISABLED) return GHOSTOS_VM_REPLAY_OK;
    if (session->mode == GHOSTOS_VM_REPLAY_RECORDING)
        return record_event(session, 1, ip, address, size, value, NULL, 0);
    ghostos_vm_replay_view view;
    uint32_t result = ghostos_vm_replay_next(session, 1, &view);
    if (result) return result;
    if (view.event.a != ip || view.event.b != address || view.event.c != size)
        return mismatch(session, &view);
    *output = view.event.d;
    return GHOSTOS_VM_REPLAY_OK;
}

uint32_t ghostos_vm_replay_clock(ghostos_vm_replay_session *session,
    uint64_t now, uint64_t *output) {
    if (!session || !output) return GHOSTOS_VM_REPLAY_CORRUPT;
    *output = now;
    if (session->mode == GHOSTOS_VM_REPLAY_DISABLED) return GHOSTOS_VM_REPLAY_OK;
    if (session->mode == GHOSTOS_VM_REPLAY_RECORDING)
        return record_event(session, 4, now, 0, 0, 0, NULL, 0);
    ghostos_vm_replay_view view;
    uint32_t result = ghostos_vm_replay_next(session, 4, &view);
    if (!result) *output = view.event.a;
    return result;
}

uint32_t ghostos_vm_replay_timer(ghostos_vm_replay_session *session,
    uint64_t now, uint64_t vector) {
    if (!session) return GHOSTOS_VM_REPLAY_CORRUPT;
    if (session->mode == GHOSTOS_VM_REPLAY_DISABLED) return GHOSTOS_VM_REPLAY_OK;
    if (session->mode == GHOSTOS_VM_REPLAY_RECORDING)
        return record_event(session, 5, now, vector, 0, 0, NULL, 0);
    ghostos_vm_replay_view view;
    uint32_t result = ghostos_vm_replay_next(session, 5, &view);
    if (result) return result;
    return view.event.a != now || view.event.b != vector ? mismatch(session, &view) : 0;
}

uint32_t ghostos_vm_replay_interrupt(ghostos_vm_replay_session *session, uint8_t vector) {
    if (!session) return GHOSTOS_VM_REPLAY_CORRUPT;
    if (session->mode == GHOSTOS_VM_REPLAY_DISABLED) return GHOSTOS_VM_REPLAY_OK;
    if (session->mode == GHOSTOS_VM_REPLAY_RECORDING)
        return record_event(session, 6, vector, 0, 0, 0, NULL, 0);
    ghostos_vm_replay_view view;
    uint32_t result = ghostos_vm_replay_next(session, 6, &view);
    if (result) return result;
    return view.event.a != vector ? mismatch(session, &view) : 0;
}

uint32_t ghostos_vm_replay_host_input(ghostos_vm_replay_session *session,
    uint64_t channel, bool has_rows, uint16_t rows, bool has_columns, uint16_t columns,
    const uint8_t *bytes, size_t length) {
    if (!session || (length && !bytes)) return GHOSTOS_VM_REPLAY_CORRUPT;
    if (has_rows != has_columns)
        return fail(session, GHOSTOS_VM_REPLAY_INPUT_MISMATCH, 0, 0, 2, session->count);
    uint64_t resize = has_rows ? (uint64_t)rows << 32 | columns : 0;
    if (session->mode == GHOSTOS_VM_REPLAY_DISABLED) return GHOSTOS_VM_REPLAY_OK;
    if (session->mode == GHOSTOS_VM_REPLAY_RECORDING)
        return record_event(session, 2, channel, resize, 0, length, bytes, length);
    ghostos_vm_replay_view view;
    uint32_t result = ghostos_vm_replay_next(session, 2, &view);
    if (result) return result;
    if (view.event.a != channel || view.event.b != resize || view.event.d != length ||
        view.event.payload_length != length || (length && memcmp(view.payload, bytes, length)))
        return mismatch(session, &view);
    return GHOSTOS_VM_REPLAY_OK;
}

uint32_t ghostos_vm_replay_next_host_input(ghostos_vm_replay_session *session,
    bool *present, ghostos_vm_replay_view *view, bool *has_resize,
    uint16_t *rows, uint16_t *columns) {
    if (!session || !present || !view || !has_resize || !rows || !columns)
        return GHOSTOS_VM_REPLAY_CORRUPT;
    *present = false;
    if (ghostos_vm_replay_peek_kind(session) != 2) return GHOSTOS_VM_REPLAY_OK;
    uint32_t result = ghostos_vm_replay_next(session, 2, view);
    if (result) return result;
    *has_resize = view->event.b != 0;
    *rows = (uint16_t)(view->event.b >> 32);
    *columns = (uint16_t)view->event.b;
    if ((*has_resize && (!*rows || !*columns)) ||
        view->event.d != view->event.payload_length)
        return fail(session, GHOSTOS_VM_REPLAY_CORRUPT, 0, 0, 0, 0);
    *present = true;
    return GHOSTOS_VM_REPLAY_OK;
}

uint32_t ghostos_vm_replay_device_completion(ghostos_vm_replay_session *session,
    const ghostos_vm_replay_dma_write *writes, size_t count,
    ghostos_vm_replay_dma_sink sink, void *context) {
    if (!session) return GHOSTOS_VM_REPLAY_CORRUPT;
    if (session->mode == GHOSTOS_VM_REPLAY_DISABLED) return GHOSTOS_VM_REPLAY_OK;
    if (session->mode == GHOSTOS_VM_REPLAY_RECORDING) {
        uint8_t *bytes;
        size_t length;
        uint64_t total;
        uint32_t result = ghostos_vm_replay_dma_encode(writes, count, &bytes, &length, &total);
        if (result) return result;
        result = record_event(session, 3, count, total, 0, 0, bytes, length);
        free(bytes);
        return result;
    }
    ghostos_vm_replay_view view;
    uint32_t result = ghostos_vm_replay_next(session, 3, &view);
    if (result) return result;
    /* Recorded DMA is authoritative. Decode errors do not replace last_error. */
    return ghostos_vm_replay_dma_decode(view.payload, (size_t)view.event.payload_length, sink, context);
}
