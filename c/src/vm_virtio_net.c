#include "ghostos/vm_virtio_net.h"
#include "ghostos/vm_packet.h"

#include <stdlib.h>
#include <string.h>

struct ghostos_vm_virtio_net {
    uint8_t mac[6], status, interrupt_status;
    uint32_t guest_features, queue_pfn;
    uint16_t queue_sel, avail_last[2], used_count[2];
    bool queue_enabled[2], poll_pending;
    ghostos_vm_packet_queue *pending_rx;
    int32_t last_error;
};

static const uint32_t features = (1u << 5) | (1u << 16);

ghostos_vm_virtio_net *ghostos_vm_virtio_net_new(const uint8_t mac[6]) {
    ghostos_vm_virtio_net *net = calloc(1, sizeof(*net));
    if (!net) return NULL;
    net->pending_rx = ghostos_vm_packet_queue_new(128, GHOSTOS_VM_ETHERNET_FRAME_MAX * 2);
    if (!net->pending_rx) { free(net); return NULL; }
    memcpy(net->mac, mac, 6);
    net->last_error = -1;
    return net;
}

void ghostos_vm_virtio_net_free(ghostos_vm_virtio_net *net) {
    if (!net) return;
    ghostos_vm_packet_queue_free(net->pending_rx);
    free(net);
}

void ghostos_vm_virtio_net_clear_rx(ghostos_vm_virtio_net *net) {
    ghostos_vm_packet_queue_clear(net->pending_rx);
}

void ghostos_vm_virtio_net_reset(ghostos_vm_virtio_net *net) {
    uint8_t mac[6];
    memcpy(mac, net->mac, 6);
    ghostos_vm_packet_queue *queue = net->pending_rx;
    ghostos_vm_packet_queue_clear(queue);
    *net = (ghostos_vm_virtio_net){.pending_rx = queue, .last_error = -1};
    memcpy(net->mac, mac, 6);
}

bool ghostos_vm_virtio_net_pending(const ghostos_vm_virtio_net *net) {
    return net->poll_pending || ghostos_vm_packet_queue_len(net->pending_rx);
}

void ghostos_vm_virtio_net_notify(ghostos_vm_virtio_net *net) { net->poll_pending = true; }

int32_t ghostos_vm_virtio_net_take_error(ghostos_vm_virtio_net *net) {
    int32_t error = net->last_error;
    net->last_error = -1;
    return error;
}

uint64_t ghostos_vm_virtio_net_read(ghostos_vm_virtio_net *net, uint16_t port, bool carrier) {
    uint16_t offset = port & 0xff;
    switch (offset) {
        case 0: return features;
        case 4: return net->guest_features;
        case 8: return net->queue_pfn;
        case 0x0c: return GHOSTOS_VM_VIRTIO_QUEUE_SIZE;
        case 0x0e: return net->queue_sel;
        case 0x12: return net->status;
        case 0x13: {
            uint8_t status = net->interrupt_status;
            net->interrupt_status = 0;
            return status;
        }
        case 0x1a: return carrier;
        default: return offset >= 0x14 && offset <= 0x19 ? net->mac[offset - 0x14] : 0;
    }
}

void ghostos_vm_virtio_net_write(ghostos_vm_virtio_net *net, uint16_t port, uint32_t value) {
    switch (port & 0xff) {
        case 4: net->guest_features = value; break;
        case 8:
            net->queue_pfn = value;
            net->queue_enabled[net->queue_sel] = value != 0;
            net->avail_last[net->queue_sel] = net->used_count[net->queue_sel] = 0;
            break;
        case 0x0e: net->queue_sel = value & 1; break;
        case 0x10: net->poll_pending = true; break;
        case 0x12:
            net->status = (uint8_t)value;
            if (!net->status) {
                net->queue_pfn = 0;
                net->queue_enabled[0] = net->queue_enabled[1] = false;
                net->avail_last[0] = net->avail_last[1] = 0;
                net->used_count[0] = net->used_count[1] = 0;
                net->interrupt_status = 0;
                net->poll_pending = false;
            }
            break;
        default: break;
    }
}

static uint64_t decode_le(const uint8_t *bytes, unsigned count) {
    uint64_t value = 0;
    for (unsigned i = 0; i < count; ++i) value |= (uint64_t)bytes[i] << (8 * i);
    return value;
}

static void encode_le(uint8_t *bytes, uint64_t value, unsigned count) {
    for (unsigned i = 0; i < count; ++i) bytes[i] = (uint8_t)(value >> (8 * i));
}

static void update_avail(ghostos_vm_virtio_net *net, unsigned queue,
    const ghostos_vm_virtio_net_io *io) {
    uint8_t bytes[2];
    if (io->read_memory(io->context, ghostos_vm_virtio_avail_base(net->queue_pfn) + 2,
        bytes, sizeof(bytes))) net->avail_last[queue] = (uint16_t)decode_le(bytes, 2);
}

static void interrupt(ghostos_vm_virtio_net *net, const ghostos_vm_virtio_net_io *io) {
    net->interrupt_status |= 1;
    if (io->interrupt) io->interrupt(io->context);
}

static bool poll_tx(ghostos_vm_virtio_net *net, const ghostos_vm_virtio_net_io *io) {
    const unsigned queue = 1;
    if (!net->queue_enabled[queue]) return true;
    update_avail(net, queue, io);
    uint64_t base = ghostos_vm_virtio_desc_base(net->queue_pfn);
    while (net->avail_last[queue] != net->used_count[queue]) {
        uint16_t index = net->used_count[queue] & (GHOSTOS_VM_VIRTIO_QUEUE_SIZE - 1);
        uint8_t descriptor[16];
        if (!io->read_memory(io->context, base + (uint64_t)index * 16,
            descriptor, sizeof(descriptor))) break;
        uint64_t address = decode_le(descriptor, 8);
        uint32_t length = (uint32_t)decode_le(descriptor + 8, 4);
        if (!length) break;
        uint8_t *packet = calloc(1, length);
        if (!packet) return false;
        if (!io->read_memory(io->context, address, packet, length)) { free(packet); break; }
        int32_t error = io->transmit ? io->transmit(io->context, packet, length) : 5;
        free(packet);
        if (error >= 0) net->last_error = error;
        uint8_t entry[8] = {0};
        encode_le(entry, index, 2);
        encode_le(entry + 4, length, 4);
        uint64_t used = ghostos_vm_virtio_used_base(net->queue_pfn);
        if (!io->write_memory(io->context, used + 4 + (uint64_t)index * 8, entry, sizeof(entry))) break;
        net->used_count[queue] = (uint16_t)(net->used_count[queue] + 1);
        uint8_t bytes[2];
        encode_le(bytes, net->used_count[queue], 2);
        (void)io->write_memory(io->context, used + 2, bytes, sizeof(bytes));
    }
    /* The legacy model asserts TX completion even when no entry advanced. */
    interrupt(net, io);
    return true;
}

static bool poll_rx(ghostos_vm_virtio_net *net, const ghostos_vm_virtio_net_io *io) {
    const unsigned queue = 0;
    if (!net->queue_enabled[queue] || !ghostos_vm_packet_queue_len(net->pending_rx)) return true;
    update_avail(net, queue, io);
    uint64_t base = ghostos_vm_virtio_desc_base(net->queue_pfn);
    uint64_t used = ghostos_vm_virtio_used_base(net->queue_pfn);
    unsigned delivered = 0;
    while (net->avail_last[queue] != net->used_count[queue] &&
        ghostos_vm_packet_queue_len(net->pending_rx)) {
        uint16_t index = net->used_count[queue] & (GHOSTOS_VM_VIRTIO_QUEUE_SIZE - 1);
        uint8_t descriptor[16];
        if (!io->read_memory(io->context, base + (uint64_t)index * 16,
            descriptor, sizeof(descriptor))) break;
        uint64_t address = decode_le(descriptor, 8);
        uint32_t length = (uint32_t)decode_le(descriptor + 8, 4);
        size_t packet_length = 0;
        const uint8_t *packet = ghostos_vm_packet_queue_peek(net->pending_rx, &packet_length);
        if (packet_length + 10 > length) {
            (void)ghostos_vm_packet_queue_pop(net->pending_rx);
            break;
        }
        size_t frame_length = packet_length + 10;
        uint8_t *frame = calloc(1, frame_length);
        if (!frame) { (void)ghostos_vm_packet_queue_pop(net->pending_rx); return false; }
        if (packet_length) memcpy(frame + 10, packet, packet_length);
        (void)ghostos_vm_packet_queue_pop(net->pending_rx);
        bool written = io->write_memory(io->context, address, frame, frame_length);
        free(frame);
        if (!written) break;
        uint8_t entry[8] = {0};
        encode_le(entry, index, 2);
        encode_le(entry + 4, frame_length, 4);
        if (!io->write_memory(io->context, used + 4 + (uint64_t)index * 8, entry, sizeof(entry))) break;
        net->used_count[queue] = (uint16_t)(net->used_count[queue] + 1);
        ++delivered;
    }
    if (delivered) {
        uint8_t bytes[2];
        encode_le(bytes, net->used_count[queue], 2);
        (void)io->write_memory(io->context, used + 2, bytes, sizeof(bytes));
        interrupt(net, io);
    }
    return true;
}

bool ghostos_vm_virtio_net_poll(ghostos_vm_virtio_net *net, const ghostos_vm_virtio_net_io *io) {
    if (!io->receive) net->last_error = 5;
    else {
        for (;;) {
            const uint8_t *packet = NULL;
            size_t length = 0;
            int32_t result = io->receive(io->context, &packet, &length);
            if (result == 0) break;
            if (result < 0) { net->last_error = -result - 1; break; }
            if (!ghostos_vm_packet_queue_push(net->pending_rx, packet, length)) {
                net->last_error = 2;
                break;
            }
        }
    }
    if (ghostos_vm_virtio_net_pending(net) && (!poll_tx(net, io) || !poll_rx(net, io))) return false;
    net->poll_pending = false;
    return true;
}
