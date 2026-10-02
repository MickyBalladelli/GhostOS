#include "ghostos/vm_serial.h"

#include <stdlib.h>
#include <string.h>

static const uint8_t panic_marker[] = "KERNEL PANIC";

void ghostos_vm_serial_observe_panic_marker(uint8_t byte, size_t *progress,
    bool *detected) {
    if (!progress || !detected || *detected) return;
    if (*progress >= sizeof(panic_marker) - 1) {
        *progress = 0;
        return;
    }
    if (byte == panic_marker[*progress]) {
        ++*progress;
        if (*progress == sizeof(panic_marker) - 1) *detected = true;
    } else {
        *progress = byte == panic_marker[0] ? 1 : 0;
    }
}

bool ghostos_vm_serial_translate_newlines(const uint8_t *input, size_t input_length,
    bool previous_was_cr, uint8_t *output, size_t output_capacity,
    size_t *output_length, bool *output_previous_was_cr) {
    if ((!input && input_length) || !output_length || !output_previous_was_cr)
        return false;
    size_t required = input_length;
    bool last_was_cr = previous_was_cr;
    for (size_t i = 0; i < input_length; ++i) {
        if (input[i] == '\n' && !last_was_cr) {
            if (required == SIZE_MAX) return false;
            ++required;
        }
        last_was_cr = input[i] == '\r';
    }
    *output_length = required;
    *output_previous_was_cr = last_was_cr;
    if (!output) return output_capacity == 0;
    if (output_capacity < required) return false;
    size_t position = 0;
    last_was_cr = previous_was_cr;
    for (size_t i = 0; i < input_length; ++i) {
        uint8_t byte = input[i];
        if (byte == '\n' && !last_was_cr) output[position++] = '\r';
        output[position++] = byte;
        last_was_cr = byte == '\r';
    }
    return position == required;
}

typedef struct {
    uint8_t *data;
    size_t length, capacity;
} byte_vector;

struct ghostos_vm_serial {
    uint16_t base;
    bool dlab;
    uint8_t divisor_low, divisor_high, ier, fcr, lcr, mcr, lsr, msr, scratch;
    uint8_t tx_buffer[GHOSTOS_VM_SERIAL_FIFO_SIZE];
    uint8_t rx_buffer[GHOSTOS_VM_SERIAL_FIFO_SIZE];
    size_t tx_count, rx_head, rx_count;
    byte_vector pending, output, host_output, banner;
    size_t pending_head, pending_count, panic_marker_progress;
    bool host_last_was_cr, panic_detected, auth_waiting, prompt_shown;
    bool spinner_drawn;
    uint8_t spinner_frame;
    uint64_t last_spinner_ns;
};

static bool reserve(byte_vector *vector, size_t length) {
    if (length <= vector->capacity) return true;
    size_t capacity = vector->capacity ? vector->capacity : 64;
    while (capacity < length) {
        if (capacity > SIZE_MAX / 2) {
            capacity = length;
            break;
        }
        capacity *= 2;
    }
    uint8_t *replacement = realloc(vector->data, capacity);
    if (!replacement) return false;
    vector->data = replacement;
    vector->capacity = capacity;
    return true;
}

static bool append(byte_vector *vector, const uint8_t *bytes, size_t length) {
    if (!length) return true;
    if (!bytes || length > SIZE_MAX - vector->length ||
        !reserve(vector, vector->length + length)) return false;
    memcpy(vector->data + vector->length, bytes, length);
    vector->length += length;
    return true;
}

static void remove_range(byte_vector *vector, size_t start, size_t end) {
    if (start == end) return;
    memmove(vector->data + start, vector->data + end, vector->length - end);
    vector->length -= end - start;
}

static void release_buffers(ghostos_vm_serial *serial) {
    free(serial->pending.data);
    free(serial->output.data);
    free(serial->host_output.data);
    free(serial->banner.data);
}

static void initialize(ghostos_vm_serial *serial, uint16_t base, uint64_t now_ns) {
    *serial = (ghostos_vm_serial){
        .base = base, .divisor_low = 0x0c, .lcr = 3, .lsr = 0x60,
        .last_spinner_ns = now_ns
    };
}

ghostos_vm_serial *ghostos_vm_serial_new(uint16_t base, uint64_t now_ns) {
    ghostos_vm_serial *serial = malloc(sizeof(*serial));
    if (serial) initialize(serial, base, now_ns);
    return serial;
}

void ghostos_vm_serial_free(ghostos_vm_serial *serial) {
    if (!serial) return;
    release_buffers(serial);
    free(serial);
}

void ghostos_vm_serial_reset(ghostos_vm_serial *serial, uint64_t now_ns) {
    uint16_t base = serial->base;
    release_buffers(serial);
    initialize(serial, base, now_ns);
}

uint16_t ghostos_vm_serial_base(const ghostos_vm_serial *serial) { return serial->base; }
size_t ghostos_vm_serial_tx_count(const ghostos_vm_serial *serial) { return serial->tx_count; }
size_t ghostos_vm_serial_rx_count(const ghostos_vm_serial *serial) { return serial->rx_count; }
bool ghostos_vm_serial_guest_panicked(const ghostos_vm_serial *serial) { return serial->panic_detected; }
bool ghostos_vm_serial_auth_waiting(const ghostos_vm_serial *serial) { return serial->auth_waiting; }

static void rx_push(ghostos_vm_serial *serial, uint8_t byte) {
    serial->rx_buffer[(serial->rx_head + serial->rx_count) % GHOSTOS_VM_SERIAL_FIFO_SIZE] = byte;
    ++serial->rx_count;
}

static void refill_rx(ghostos_vm_serial *serial) {
    while (serial->rx_count < GHOSTOS_VM_SERIAL_FIFO_SIZE && serial->pending_count) {
        rx_push(serial, serial->pending.data[serial->pending_head]);
        serial->pending_head = (serial->pending_head + 1) % serial->pending.capacity;
        --serial->pending_count;
    }
    if (!serial->pending_count) serial->pending_head = 0;
}

bool ghostos_vm_serial_push_input(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length) {
    refill_rx(serial);
    bool was_empty = serial->rx_count == 0;
    for (size_t i = 0; i < length; ++i) {
        if (serial->rx_count >= GHOSTOS_VM_SERIAL_FIFO_SIZE) {
            serial->lsr |= 2;
            break;
        }
        rx_push(serial, bytes[i]);
    }
    return was_empty && serial->rx_count && (serial->ier & 1);
}

bool ghostos_vm_serial_push_input_lossless(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length, bool *interrupt) {
    *interrupt = false;
    if (!length) return true;
    if (!bytes || length > SIZE_MAX - serial->pending_count) return false;
    size_t needed = serial->pending_count + length;
    if (needed > serial->pending.capacity) {
        byte_vector replacement = {0};
        if (!reserve(&replacement, needed)) return false;
        if (serial->pending_count) {
            size_t first = serial->pending.capacity - serial->pending_head;
            if (first > serial->pending_count) first = serial->pending_count;
            memcpy(replacement.data, serial->pending.data + serial->pending_head, first);
            memcpy(replacement.data + first, serial->pending.data, serial->pending_count - first);
        }
        free(serial->pending.data);
        serial->pending = replacement;
        serial->pending_head = 0;
    }
    size_t space = serial->pending.capacity - serial->pending_head;
    size_t tail = serial->pending_count >= space ? serial->pending_count - space :
        serial->pending_head + serial->pending_count;
    size_t first = serial->pending.capacity - tail;
    if (first > length) first = length;
    memcpy(serial->pending.data + tail, bytes, first);
    memcpy(serial->pending.data, bytes + first, length - first);
    bool was_empty = serial->rx_count == 0;
    serial->pending_count = needed;
    refill_rx(serial);
    *interrupt = was_empty && serial->rx_count && (serial->ier & 1);
    return true;
}

bool ghostos_vm_serial_input_pending(const ghostos_vm_serial *serial) {
    return serial->rx_count || serial->pending_count;
}

bool ghostos_vm_serial_set_banner(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length) {
    if (serial->banner.length == length &&
        (!length || (bytes && memcmp(serial->banner.data, bytes, length) == 0))) return true;
    if ((length && !bytes) || !reserve(&serial->banner, length)) return false;
    if (length) memcpy(serial->banner.data, bytes, length);
    serial->banner.length = length;
    return true;
}

const uint8_t *ghostos_vm_serial_output(const ghostos_vm_serial *serial, size_t *length) {
    *length = serial->output.length;
    return serial->output.data;
}

void ghostos_vm_serial_clear_output(ghostos_vm_serial *serial) {
    free(serial->output.data);
    serial->output = (byte_vector){0};
}

const uint8_t *ghostos_vm_serial_host_output(const ghostos_vm_serial *serial, size_t *length) {
    *length = serial->host_output.length;
    return serial->host_output.data;
}

bool ghostos_vm_serial_append_host_output(ghostos_vm_serial *serial,
    const uint8_t *bytes, size_t length) {
    return append(&serial->host_output, bytes, length);
}

void ghostos_vm_serial_consume_host_output(ghostos_vm_serial *serial, size_t length) {
    if (length > serial->host_output.length) length = serial->host_output.length;
    remove_range(&serial->host_output, 0, length);
}

static bool flush_tx(ghostos_vm_serial *serial) {
    if (!serial->tx_count) return true;
    uint8_t translated[GHOSTOS_VM_SERIAL_FIFO_SIZE * 2];
    size_t length = 0;
    bool previous_cr = serial->host_last_was_cr;
    if (!ghostos_vm_serial_translate_newlines(serial->tx_buffer, serial->tx_count,
        previous_cr, translated, sizeof(translated), &length, &previous_cr) ||
        !append(&serial->host_output, translated, length)) return false;
    serial->host_last_was_cr = previous_cr;
    const size_t output_limit = 1024u * 1024u;
    if (serial->output.length > output_limit * 2)
        remove_range(&serial->output, 0, serial->output.length - output_limit);
    serial->tx_count = 0;
    return true;
}

uint8_t ghostos_vm_serial_read(ghostos_vm_serial *serial, uint16_t port,
    uint8_t size, uint64_t *value) {
    if (size != 1) return 1;
    unsigned offset = (uint16_t)(port - serial->base) & 7;
    switch (offset) {
        case 0:
            if (serial->dlab) *value = serial->divisor_low;
            else {
                *value = serial->rx_count ? serial->rx_buffer[serial->rx_head] : 0;
                if (serial->rx_count) {
                    serial->rx_head = (serial->rx_head + 1) % GHOSTOS_VM_SERIAL_FIFO_SIZE;
                    --serial->rx_count;
                }
                refill_rx(serial);
            }
            break;
        case 1: *value = serial->dlab ? serial->divisor_high : serial->ier; break;
        case 2: *value = serial->rx_count && (serial->ier & 1) ? 4 : 1; break;
        case 3: *value = serial->lcr; break;
        case 4: *value = serial->mcr; break;
        case 5: *value = serial->rx_count ? serial->lsr | 1u : serial->lsr & ~1u; break;
        case 6: *value = serial->msr; break;
        case 7: *value = serial->scratch; break;
    }
    return 0;
}

uint8_t ghostos_vm_serial_write(ghostos_vm_serial *serial, uint16_t port,
    uint64_t raw, uint8_t size, bool *interrupt) {
    *interrupt = false;
    if (size != 1) return 1;
    uint8_t value = (uint8_t)raw;
    unsigned offset = (uint16_t)(port - serial->base) & 7;
    switch (offset) {
        case 0: if (serial->dlab) serial->divisor_low = value; break;
        case 1:
            if (serial->dlab) serial->divisor_high = value;
            else {
                serial->ier = value;
                *interrupt = (value & 1) && serial->rx_count;
            }
            break;
        case 2:
            serial->fcr = value;
            if (value & 2) serial->rx_count = serial->rx_head = 0;
            if (value & 4) serial->tx_count = 0;
            break;
        case 3: serial->lcr = value; serial->dlab = (value & 0x80) != 0; break;
        case 4: serial->mcr = value; break;
        case 7: serial->scratch = value; break;
        default: break;
    }
    if (offset == 0 && !serial->dlab) {
        if (serial->tx_count >= GHOSTOS_VM_SERIAL_FIFO_SIZE && !flush_tx(serial)) return 2;
        if (!append(&serial->output, &value, 1)) return 2;
        serial->tx_buffer[serial->tx_count++] = value;
        ghostos_vm_serial_observe_panic_marker(value, &serial->panic_marker_progress,
            &serial->panic_detected);
        if ((value == '\n' || value == '\r') && !flush_tx(serial)) return 2;
    }
    return 0;
}

static const uint8_t enrollment_marker[] = "\x1b]GhostOSEnroll\x07";
static const uint8_t login_marker[] = "\x1b]GhostOSLogin\x07";
static const uint8_t enrollment_prompt[] = "Administrator username: ";
static const uint8_t login_prompt[] = "Username: ";
static const uint8_t auth_progress_prefix[] = "GhostOS authentication:";

typedef struct {
    const uint8_t *marker, *fallback;
    size_t marker_length, fallback_length;
} authentication_marker;

static const authentication_marker authentication_markers[] = {
    {enrollment_marker, enrollment_prompt, sizeof(enrollment_marker) - 1, sizeof(enrollment_prompt) - 1},
    {login_marker, login_prompt, sizeof(login_marker) - 1, sizeof(login_prompt) - 1}
};

static size_t find_bytes(const uint8_t *bytes, size_t length,
    const uint8_t *pattern, size_t pattern_length) {
    if (pattern_length > length) return SIZE_MAX;
    for (size_t i = 0; i <= length - pattern_length; ++i)
        if (memcmp(bytes + i, pattern, pattern_length) == 0) return i;
    return SIZE_MAX;
}

static size_t pending_marker_bytes(const byte_vector *output) {
    size_t pending = 0;
    for (unsigned i = 0; i < 2; ++i) {
        const authentication_marker *marker = &authentication_markers[i];
        for (size_t length = 1; length < marker->marker_length && length <= output->length; ++length)
            if (length > pending && memcmp(output->data + output->length - length,
                marker->marker, length) == 0) pending = length;
    }
    return pending;
}

static size_t authorized_prompt_offset(const uint8_t *bytes, size_t length, bool allow_bare) {
    if (allow_bare && length >= 2 && bytes[0] == '$' && bytes[1] == ' ') return 0;
    for (size_t i = 0; length >= 3 && i <= length - 3; ++i)
        if ((bytes[i] == '\n' || bytes[i] == '\r') && bytes[i + 1] == '$' && bytes[i + 2] == ' ')
            return i + 1;
    return SIZE_MAX;
}

static bool only_authorized_prompt(const uint8_t *bytes, size_t length) {
    size_t index = 0;
    while (index < length && (bytes[index] == '\r' || bytes[index] == '\n')) ++index;
    return length - index == 2 && bytes[index] == '$' && bytes[index + 1] == ' ';
}

static void drop_auth_progress_frames(byte_vector *output) {
    for (;;) {
        size_t start = find_bytes(output->data, output->length,
            auth_progress_prefix, sizeof(auth_progress_prefix) - 1);
        if (start == SIZE_MAX) return;
        if (start > 0 && output->data[start - 1] == '\r') --start;
        size_t end = start + sizeof(auth_progress_prefix) - 1;
        while (end < output->length && output->data[end] != '\n') ++end;
        if (end < output->length) ++end;
        remove_range(output, start, end);
    }
}

bool ghostos_vm_serial_prepare_host_output(ghostos_vm_serial *serial, size_t *writable) {
    byte_vector *output = &serial->host_output;
    bool waiting = serial->auth_waiting;
    size_t waiting_from = 0;
    for (;;) {
        size_t marker_start = SIZE_MAX;
        unsigned selected = 0;
        for (unsigned i = 0; i < 2; ++i) {
            size_t start = find_bytes(output->data, output->length,
                authentication_markers[i].marker, authentication_markers[i].marker_length);
            if (start < marker_start) { marker_start = start; selected = i; }
        }
        if (marker_start == SIZE_MAX) break;
        const authentication_marker *marker = &authentication_markers[selected];
        size_t replacement_length = serial->banner.length ? 0 : marker->fallback_length;
        if (serial->banner.length && !waiting) {
            waiting_from = marker_start;
            waiting = true;
        }
        size_t marker_end = marker_start + marker->marker_length;
        size_t new_length = output->length - marker->marker_length;
        if (replacement_length > SIZE_MAX - new_length ||
            !reserve(output, new_length + replacement_length)) return false;
        memmove(output->data + marker_start + replacement_length,
            output->data + marker_end, output->length - marker_end);
        if (replacement_length) memcpy(output->data + marker_start, marker->fallback, replacement_length);
        output->length = new_length + replacement_length;
    }
    if (waiting) serial->auth_waiting = true;
    drop_auth_progress_frames(output);
    size_t pending = pending_marker_bytes(output);
    size_t complete_end = output->length - pending;
    if (!serial->auth_waiting) {
        if (serial->prompt_shown && only_authorized_prompt(output->data, complete_end)) {
            remove_range(output, 0, complete_end);
            *writable = 0;
            return true;
        }
        if (authorized_prompt_offset(output->data, complete_end, true) != SIZE_MAX)
            serial->prompt_shown = true;
        else {
            for (size_t i = 0; i < complete_end; ++i)
                if (output->data[i] != '\r' && output->data[i] != '\n') {
                    serial->prompt_shown = false;
                    break;
                }
        }
        *writable = complete_end;
        return true;
    }
    if (serial->panic_detected) {
        serial->auth_waiting = false;
        *writable = complete_end;
        return true;
    }
    size_t search_from = waiting_from < complete_end ? waiting_from : complete_end;
    bool allow_bare = !(serial->prompt_shown && search_from == 0);
    /* Empty vectors can have a NULL buffer. Avoid pointer arithmetic on NULL. */
    const uint8_t *search = output->length ? output->data + search_from : NULL;
    size_t relative = authorized_prompt_offset(search, complete_end - search_from, allow_bare);
    if (relative != SIZE_MAX) {
        remove_range(output, 0, search_from + relative);
        serial->auth_waiting = false;
        serial->prompt_shown = true;
        *writable = output->length - pending;
        return true;
    }
    if (search_from == 0 && output->length > 32 + pending)
        remove_range(output, 0, output->length - 32 - pending);
    *writable = search_from;
    return true;
}

bool ghostos_vm_serial_flush(ghostos_vm_serial *serial,
    ghostos_vm_serial_console_write write_console,
    ghostos_vm_serial_console_flush flush_console,
    ghostos_vm_serial_console_time now_ns, void *context) {
    if (!flush_tx(serial)) return false;
    size_t writable = 0;
    if (serial->host_output.length && !ghostos_vm_serial_prepare_host_output(serial, &writable))
        return false;
    if (!writable && !serial->auth_waiting) return true;
    if (writable) {
        if (serial->spinner_drawn) {
            static const uint8_t clear[] = "\r\x1b[K";
            write_console(context, clear, sizeof(clear) - 1);
            serial->spinner_drawn = false;
        }
        write_console(context, serial->host_output.data, writable);
        remove_range(&serial->host_output, 0, writable);
    }
    if (serial->auth_waiting) {
        uint64_t now = now_ns(context);
        uint64_t elapsed = now >= serial->last_spinner_ns ? now - serial->last_spinner_ns : 0;
        if (!serial->spinner_drawn || elapsed >= UINT64_C(80000000)) {
            static const uint8_t frames[] = "|/-\\";
            uint8_t frame[] = {'\r', frames[serial->spinner_frame], 0x1b, '[', 'K'};
            serial->spinner_frame = (serial->spinner_frame + 1) % 4;
            serial->last_spinner_ns = now;
            serial->spinner_drawn = true;
            write_console(context, frame, sizeof(frame));
        }
    }
    flush_console(context);
    return true;
}
