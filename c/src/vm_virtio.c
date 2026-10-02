#include "ghostos/vm_virtio.h"
#include "ghostos/vm_serial.h"

#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(ghostos_vm_virtio_transport) == 20, "Virtio transport ABI");
_Static_assert(offsetof(ghostos_vm_virtio_transport, status) == 14, "Virtio status offset");

struct ghostos_vm_virtio_console {
    uint8_t *output;
    size_t length, capacity;
    bool previous_cr;
};

void ghostos_vm_virtio_transport_init(ghostos_vm_virtio_transport *transport) {
    *transport = (ghostos_vm_virtio_transport){0};
}

bool ghostos_vm_virtio_read_common(ghostos_vm_virtio_transport *transport,
    uint16_t offset, uint32_t features, uint64_t *value) {
    switch (offset) {
        case 0: *value = features; break;
        case 4: *value = transport->guest_features; break;
        case 8: *value = transport->pfn; break;
        case 0x0c: *value = GHOSTOS_VM_VIRTIO_QUEUE_SIZE; break;
        case 0x0e: *value = transport->queue_sel; break;
        case 0x12: *value = transport->status; break;
        case 0x13: *value = transport->interrupt_status; transport->interrupt_status = 0; break;
        default: return false;
    }
    return true;
}

bool ghostos_vm_virtio_write_common(ghostos_vm_virtio_transport *transport,
    uint16_t offset, uint32_t value) {
    switch (offset) {
        case 4: transport->guest_features = value; break;
        case 8:
            transport->pfn = value;
            transport->avail_last = transport->used_idx = 0;
            break;
        case 0x0e: transport->queue_sel = (uint16_t)value; break;
        case 0x10: transport->pending = true; break;
        case 0x12:
            transport->status = (uint8_t)value;
            if (!transport->status) {
                transport->pfn = 0;
                transport->avail_last = transport->used_idx = 0;
                transport->interrupt_status = 0;
                transport->pending = false;
            }
            break;
        default: break;
    }
    return offset == 0x10;
}

bool ghostos_vm_virtio_take_pending(ghostos_vm_virtio_transport *transport) {
    bool pending = transport->pending;
    transport->pending = false;
    return pending;
}

uint8_t ghostos_vm_virtio_device_read(ghostos_vm_virtio_transport *transport,
    uint8_t kind, uint16_t port, uint8_t size, uint64_t sectors, uint64_t *value) {
    uint16_t offset = port & 0xff;
    if (ghostos_vm_virtio_read_common(transport, offset,
        kind == GHOSTOS_VM_VIRTIO_BLOCK ? 1u << 6 : 0, value)) return 0;
    if (kind == GHOSTOS_VM_VIRTIO_BLOCK && offset >= 0x14 && offset < 0x1c) {
        unsigned start = offset - 0x14;
        if ((size != 1 && size != 2 && size != 4 && size != 8) || start + size > 8) return 1;
        *value = sectors >> (8 * start);
        if (size < 8) *value &= (UINT64_C(1) << (8 * size)) - 1;
        return 0;
    }
    if (kind == GHOSTOS_VM_VIRTIO_CONSOLE && offset >= 0x14 && offset < 0x18) {
        if (size != 1 && size != 2 && size != 4) return 1;
        *value = 1;
        return 0;
    }
    return 2;
}

uint8_t ghostos_vm_virtio_device_write(ghostos_vm_virtio_transport *transport,
    uint16_t port, uint64_t value, uint8_t size) {
    if (size != 1 && size != 2 && size != 4) return 1;
    (void)ghostos_vm_virtio_write_common(transport, port & 0xff, (uint32_t)value);
    return 0;
}

ghostos_vm_virtio_console *ghostos_vm_virtio_console_new(void) {
    return calloc(1, sizeof(ghostos_vm_virtio_console));
}

void ghostos_vm_virtio_console_free(ghostos_vm_virtio_console *console) {
    if (!console) return;
    free(console->output);
    free(console);
}

void ghostos_vm_virtio_console_reset(ghostos_vm_virtio_console *console) {
    console->length = 0;
    console->previous_cr = false;
}

const uint8_t *ghostos_vm_virtio_console_output(const ghostos_vm_virtio_console *console,
    size_t *length) {
    *length = console->length;
    return console->output;
}

void ghostos_vm_virtio_console_clear_output(ghostos_vm_virtio_console *console) {
    free(console->output);
    console->output = NULL;
    console->length = console->capacity = 0;
}

bool ghostos_vm_virtio_console_append(ghostos_vm_virtio_console *console,
    const uint8_t *bytes, size_t length) {
    if (!length) return true;
    if (!bytes || length > SIZE_MAX - console->length) return false;
    size_t needed = console->length + length;
    if (needed > console->capacity) {
        size_t capacity = console->capacity ? console->capacity : 64;
        while (capacity < needed) {
            if (capacity > SIZE_MAX / 2) { capacity = needed; break; }
            capacity *= 2;
        }
        uint8_t *replacement = realloc(console->output, capacity);
        if (!replacement) return false;
        console->output = replacement;
        console->capacity = capacity;
    }
    memcpy(console->output + console->length, bytes, length);
    console->length = needed;
    return true;
}

void ghostos_vm_virtio_fill_random(uint64_t *fallback_state, uint8_t *bytes,
    size_t length, const ghostos_vm_virtio_io *io) {
    if (io->entropy && io->entropy(io->context, bytes, length)) return;
    for (size_t i = 0; i < length; ++i) {
        *fallback_state ^= *fallback_state << 7;
        *fallback_state ^= *fallback_state >> 9;
        *fallback_state ^= *fallback_state << 8;
        bytes[i] = (uint8_t)*fallback_state;
    }
}

static bool begin_poll(ghostos_vm_virtio_transport *transport) {
    return ghostos_vm_virtio_take_pending(transport) && (transport->status & 4);
}

static bool next_head(ghostos_vm_virtio_transport *transport,
    const ghostos_vm_virtio_io *io, uint16_t *head) {
    return ghostos_vm_virtio_next_available(transport->pfn, &transport->avail_last,
        io->read_memory, io->context, head);
}

static bool chain(ghostos_vm_virtio_transport *transport, const ghostos_vm_virtio_io *io,
    uint16_t head, ghostos_vm_virtio_descriptor *descriptors, size_t *count) {
    return ghostos_vm_virtio_descriptor_chain(transport->pfn, head,
        io->read_memory, io->context, descriptors, count);
}

static void complete(ghostos_vm_virtio_transport *transport, const ghostos_vm_virtio_io *io,
    uint16_t head, uint32_t length) {
    if (!ghostos_vm_virtio_complete(transport->pfn, &transport->used_idx,
        head, length, io->write_memory, io->context)) return;
    transport->interrupt_status |= 1;
    if (io->interrupt) io->interrupt(io->context);
}

static uint64_t little_endian(const uint8_t *bytes, unsigned length) {
    uint64_t value = 0;
    for (unsigned i = 0; i < length; ++i) value |= (uint64_t)bytes[i] << (8 * i);
    return value;
}

/* Returns false only on host allocation failure; guest failures leave IOERR. */
static bool block_request(ghostos_vm_virtio_transport *transport,
    const ghostos_vm_virtio_io *io, uint16_t head, uint8_t *status, uint32_t *used_length) {
    *status = 1;
    *used_length = 1;
    ghostos_vm_virtio_descriptor descriptors[GHOSTOS_VM_VIRTIO_MAX_CHAIN];
    size_t count = 0;
    if (!chain(transport, io, head, descriptors, &count) || count < 2) return true;
    const ghostos_vm_virtio_descriptor *header = &descriptors[0];
    if ((header->flags & GHOSTOS_VM_VIRTIO_DESC_WRITE) || header->len < 16) return true;
    uint8_t header_bytes[16];
    if (!io->read_memory(io->context, header->addr, header_bytes, sizeof(header_bytes))) return true;
    uint32_t type = (uint32_t)little_endian(header_bytes, 4);
    uint64_t sector = little_endian(header_bytes + 8, 8);
    if (!(descriptors[count - 1].flags & GHOSTOS_VM_VIRTIO_DESC_WRITE) ||
        descriptors[count - 1].len < 1) return true;
    size_t length = 0;
    for (size_t i = 1; i + 1 < count; ++i) {
        if (descriptors[i].len > SIZE_MAX - length) return true;
        length += descriptors[i].len;
    }
    if (length > GHOSTOS_VM_VIRTIO_MAX_DMA_BYTES) return true;
    if (type == 4) {
        if (io->flush_disk && io->flush_disk(io->context)) *status = 0;
        return true;
    }
    if (type != 0 && type != 1) { *status = 2; return true; }
    if (!length || length % 512) return true;
    uint8_t *bytes = calloc(1, length);
    if (!bytes) return false;
    bool valid = true;
    if (type == 0) {
        for (size_t offset = 0; offset < length; offset += 512) {
            if (!io->read_sector || !io->read_sector(io->context, sector + offset / 512, bytes + offset)) {
                valid = false;
                break;
            }
        }
        size_t offset = 0;
        for (size_t i = 1; valid && i + 1 < count; ++i) {
            const ghostos_vm_virtio_descriptor *descriptor = &descriptors[i];
            if (!(descriptor->flags & GHOSTOS_VM_VIRTIO_DESC_WRITE) ||
                descriptor->len > GHOSTOS_VM_VIRTIO_MAX_DMA_BYTES ||
                descriptor->len > length - offset ||
                !io->write_memory(io->context, descriptor->addr, bytes + offset, descriptor->len)) {
                valid = false;
                break;
            }
            offset += descriptor->len;
        }
        if (valid) valid = offset == length;
    } else {
        size_t offset = 0;
        for (size_t i = 1; i + 1 < count; ++i) {
            const ghostos_vm_virtio_descriptor *descriptor = &descriptors[i];
            if ((descriptor->flags & GHOSTOS_VM_VIRTIO_DESC_WRITE) ||
                !io->read_memory(io->context, descriptor->addr, bytes + offset, descriptor->len)) {
                valid = false;
                break;
            }
            offset += descriptor->len;
        }
        for (size_t offset = 0; valid && offset < length; offset += 512) {
            if (!io->write_sector || !io->write_sector(io->context, sector + offset / 512, bytes + offset))
                valid = false;
        }
    }
    free(bytes);
    if (valid) { *status = 0; *used_length = type == 0 ? (uint32_t)length : 1; }
    return true;
}

bool ghostos_vm_virtio_block_poll(ghostos_vm_virtio_transport *transport,
    const ghostos_vm_virtio_io *io) {
    if (!begin_poll(transport)) return true;
    uint16_t head = 0;
    while (next_head(transport, io, &head)) {
        uint8_t status;
        uint32_t used_length;
        if (!block_request(transport, io, head, &status, &used_length)) return false;
        /* Re-read the chain after DMA, just as the existing VM does. */
        ghostos_vm_virtio_descriptor descriptors[GHOSTOS_VM_VIRTIO_MAX_CHAIN];
        size_t count = 0;
        if (chain(transport, io, head, descriptors, &count) && count)
            (void)io->write_memory(io->context, descriptors[count - 1].addr, &status, 1);
        complete(transport, io, head, used_length);
    }
    return true;
}

static uint32_t saturating_length(uint32_t completed, uint32_t length) {
    return length > UINT32_MAX - completed ? UINT32_MAX : completed + length;
}

bool ghostos_vm_virtio_console_poll(ghostos_vm_virtio_transport *transport,
    ghostos_vm_virtio_console *console, const ghostos_vm_virtio_io *io) {
    if (!begin_poll(transport)) return true;
    uint16_t head = 0;
    while (next_head(transport, io, &head)) {
        ghostos_vm_virtio_descriptor descriptors[GHOSTOS_VM_VIRTIO_MAX_CHAIN];
        size_t count = 0;
        if (!chain(transport, io, head, descriptors, &count)) {
            complete(transport, io, head, 0);
            continue;
        }
        uint32_t completed = 0;
        bool valid = true;
        for (size_t i = 0; i < count; ++i) {
            const ghostos_vm_virtio_descriptor *descriptor = &descriptors[i];
            if ((descriptor->flags & GHOSTOS_VM_VIRTIO_DESC_WRITE) ||
                descriptor->len > GHOSTOS_VM_VIRTIO_MAX_DMA_BYTES) { valid = false; break; }
            uint8_t *bytes = malloc(descriptor->len ? descriptor->len : 1);
            if (!bytes) return false;
            if (!io->read_memory(io->context, descriptor->addr, bytes, descriptor->len)) {
                free(bytes);
                valid = false;
                break;
            }
            completed = saturating_length(completed, descriptor->len);
            if (!ghostos_vm_virtio_console_append(console, bytes, descriptor->len)) {
                free(bytes);
                return false;
            }
            size_t translated_length = 0;
            bool previous_cr = console->previous_cr;
            (void)ghostos_vm_serial_translate_newlines(bytes, descriptor->len, previous_cr,
                NULL, 0, &translated_length, &previous_cr);
            uint8_t *translated = malloc(translated_length ? translated_length : 1);
            if (!translated) { free(bytes); return false; }
            (void)ghostos_vm_serial_translate_newlines(bytes, descriptor->len, console->previous_cr,
                translated, translated_length, &translated_length, &previous_cr);
            console->previous_cr = previous_cr;
            if (io->console) io->console(io->context, translated, translated_length);
            free(translated);
            free(bytes);
        }
        if (!valid) completed = 0;
        const size_t limit = 1024u * 1024u;
        if (console->length > limit * 2) {
            memmove(console->output, console->output + console->length - limit, limit);
            console->length = limit;
        }
        complete(transport, io, head, completed);
    }
    return true;
}

bool ghostos_vm_virtio_rng_poll(ghostos_vm_virtio_transport *transport,
    uint64_t *fallback_state, const ghostos_vm_virtio_io *io) {
    if (!begin_poll(transport)) return true;
    uint16_t head = 0;
    while (next_head(transport, io, &head)) {
        ghostos_vm_virtio_descriptor descriptors[GHOSTOS_VM_VIRTIO_MAX_CHAIN];
        size_t count = 0;
        if (!chain(transport, io, head, descriptors, &count)) {
            complete(transport, io, head, 0);
            continue;
        }
        uint32_t completed = 0;
        bool valid = true;
        for (size_t i = 0; i < count; ++i) {
            const ghostos_vm_virtio_descriptor *descriptor = &descriptors[i];
            if (!(descriptor->flags & GHOSTOS_VM_VIRTIO_DESC_WRITE) ||
                descriptor->len > GHOSTOS_VM_VIRTIO_MAX_DMA_BYTES) { valid = false; break; }
            uint8_t *bytes = calloc(1, descriptor->len ? descriptor->len : 1);
            if (!bytes) return false;
            ghostos_vm_virtio_fill_random(fallback_state, bytes, descriptor->len, io);
            valid = io->write_memory(io->context, descriptor->addr, bytes, descriptor->len);
            free(bytes);
            if (!valid) break;
            completed = saturating_length(completed, descriptor->len);
        }
        if (!valid) completed = 0;
        complete(transport, io, head, completed);
    }
    return true;
}
