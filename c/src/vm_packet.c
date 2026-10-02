#include "ghostos/vm_packet.h"
#include <stdlib.h>
#include <string.h>

typedef struct {
    uint8_t *data;
    size_t length;
} packet_entry;

struct ghostos_vm_packet_queue {
    packet_entry *entries;
    size_t capacity;
    size_t max_bytes;
    size_t length;
    size_t bytes;
    size_t head;
};

ghostos_vm_packet_queue *ghostos_vm_packet_queue_new(size_t max_packets,
    size_t max_bytes) {
    ghostos_vm_packet_queue *queue = malloc(sizeof(*queue));
    if (!queue) return NULL;
    queue->entries = max_packets ? calloc(max_packets, sizeof(*queue->entries)) : NULL;
    if (max_packets && !queue->entries) {
        free(queue);
        return NULL;
    }
    queue->capacity = max_packets;
    queue->max_bytes = max_bytes;
    queue->length = 0;
    queue->bytes = 0;
    queue->head = 0;
    return queue;
}

void ghostos_vm_packet_queue_clear(ghostos_vm_packet_queue *queue) {
    if (!queue) return;
    for (size_t i = 0; i < queue->length; ++i) {
        size_t slot = (queue->head + i) % queue->capacity;
        free(queue->entries[slot].data);
        queue->entries[slot] = (packet_entry){0};
    }
    queue->length = 0;
    queue->bytes = 0;
    queue->head = 0;
}

void ghostos_vm_packet_queue_free(ghostos_vm_packet_queue *queue) {
    if (!queue) return;
    ghostos_vm_packet_queue_clear(queue);
    free(queue->entries);
    free(queue);
}

bool ghostos_vm_packet_queue_push(ghostos_vm_packet_queue *queue,
    const uint8_t *packet, size_t length) {
    if (!queue || (length && !packet) || queue->length >= queue->capacity ||
        length > queue->max_bytes || queue->bytes > queue->max_bytes - length)
        return false;
    uint8_t *copy = length ? malloc(length) : NULL;
    if (length && !copy) return false;
    if (length) memcpy(copy, packet, length);
    size_t slot = (queue->head + queue->length) % queue->capacity;
    queue->entries[slot] = (packet_entry){copy, length};
    ++queue->length;
    queue->bytes += length;
    return true;
}

bool ghostos_vm_packet_queue_pop(ghostos_vm_packet_queue *queue) {
    if (!queue || !queue->length) return false;
    packet_entry *entry = &queue->entries[queue->head];
    queue->bytes -= entry->length;
    free(entry->data);
    *entry = (packet_entry){0};
    queue->head = (queue->head + 1) % queue->capacity;
    --queue->length;
    if (!queue->length) queue->head = 0;
    return true;
}

const uint8_t *ghostos_vm_packet_queue_peek(const ghostos_vm_packet_queue *queue,
    size_t *length) {
    if (!queue || !length || !queue->length) return NULL;
    *length = queue->entries[queue->head].length;
    return queue->entries[queue->head].data;
}

size_t ghostos_vm_packet_queue_len(const ghostos_vm_packet_queue *queue) {
    return queue ? queue->length : 0;
}

size_t ghostos_vm_packet_queue_bytes(const ghostos_vm_packet_queue *queue) {
    return queue ? queue->bytes : 0;
}

bool ghostos_vm_packet_pad(const uint8_t *packet, size_t length,
    uint8_t *output, size_t output_capacity, size_t *output_length) {
    size_t padded_length = length < GHOSTOS_VM_ETHERNET_FRAME_MIN ?
        GHOSTOS_VM_ETHERNET_FRAME_MIN : length;
    if ((length && !packet) || !output || !output_length ||
        output_capacity < padded_length) return false;
    if (length) memcpy(output, packet, length);
    if (padded_length > length) memset(output + length, 0, padded_length - length);
    *output_length = padded_length;
    return true;
}

const char *ghostos_vm_net_error_message(uint32_t error) {
    static const char *const messages[] = {
        "packet exceeds maximum Ethernet frame size",
        "packet too short to contain an Ethernet header",
        "receive queue full, packet dropped",
        "network link is down",
        "network interface is administratively down",
        "network backend is unavailable"
    };
    return error < sizeof(messages) / sizeof(messages[0]) ? messages[error] : "unknown network error";
}

const char *ghostos_vm_net_error_code(uint32_t error) {
    static const char *const codes[] = {
        "NET_PACKET_TOO_LARGE", "NET_TRUNCATED", "NET_QUEUE_FULL",
        "NET_LINK_DOWN", "NET_ADMIN_DOWN", "NET_BACKEND_UNAVAILABLE"
    };
    return error < sizeof(codes) / sizeof(codes[0]) ? codes[error] : "NET_UNKNOWN";
}
