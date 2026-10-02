#include "ghostos/vm_e1000.h"
#include "ghostos/vm_packet.h"
#include <stdlib.h>
#include <string.h>

struct ghostos_vm_e1000 {
    uint8_t mac[6];
    uint32_t ctrl, ims, icr, rctl, tctl;
    uint32_t rdbal, rdbah, rdlen, rdh, rdt, tdbal, tdbah, tdlen, tdh, tdt;
    ghostos_vm_packet_queue *pending_rx;
    int32_t last_error;
};
static uint32_t read32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}
static uint64_t read64(const uint8_t *p) {
    return read32(p) | (uint64_t)read32(p + 4) << 32;
}
static void check_interrupt(const ghostos_vm_e1000 *net, const ghostos_vm_e1000_io *io) {
    if ((net->icr & net->ims) && io && io->interrupt) io->interrupt(io->context);
}
void ghostos_vm_e1000_signal(ghostos_vm_e1000 *net, uint32_t cause, const ghostos_vm_e1000_io *io) {
    if (!net) return;
    net->icr |= cause;
    check_interrupt(net, io);
}

ghostos_vm_e1000 *ghostos_vm_e1000_new(const uint8_t mac[6]) {
    if (!mac) return NULL;
    ghostos_vm_e1000 *net = calloc(1, sizeof(*net));
    if (!net) return NULL;
    net->pending_rx = ghostos_vm_packet_queue_new(128, GHOSTOS_VM_ETHERNET_FRAME_MAX * 2);
    if (!net->pending_rx) { free(net); return NULL; }
    memcpy(net->mac, mac, sizeof(net->mac));
    net->last_error = -1;
    return net;
}
void ghostos_vm_e1000_free(ghostos_vm_e1000 *net) {
    if (!net) return;
    ghostos_vm_packet_queue_free(net->pending_rx);
    free(net);
}
void ghostos_vm_e1000_reset(ghostos_vm_e1000 *net) {
    if (!net) return;
    uint8_t mac[6];
    memcpy(mac, net->mac, sizeof(mac));
    ghostos_vm_packet_queue *queue = net->pending_rx;
    ghostos_vm_packet_queue_clear(queue);
    memset(net, 0, sizeof(*net));
    memcpy(net->mac, mac, sizeof(mac));
    net->pending_rx = queue;
    net->last_error = -1;
}
void ghostos_vm_e1000_clear_rx(ghostos_vm_e1000 *net) {
    if (net) ghostos_vm_packet_queue_clear(net->pending_rx);
}
int32_t ghostos_vm_e1000_take_error(ghostos_vm_e1000 *net) {
    if (!net) return -1;
    int32_t error = net->last_error;
    net->last_error = -1;
    return error;
}

uint32_t ghostos_vm_e1000_read(const ghostos_vm_e1000 *net,
    uint64_t address, uint8_t size, bool carrier, uint64_t *output) {
    if (size != 4 || !net || !output) return GHOSTOS_VM_E1000_SIZE;
    uint32_t value;
    switch (address & (GHOSTOS_VM_E1000_MMIO_SIZE - 1)) {
        case 0: value = net->ctrl; break;
        case 8: value = 0x80000020u | (carrier ? 2 : 0); break;
        case 0x14: value = 16; break;
        case 0xc0: value = net->icr; break;
        case 0xd0: value = net->ims; break;
        case 0x100: value = net->rctl; break;
        case 0x400: value = net->tctl; break;
        case 0x2800: value = net->rdbal; break;
        case 0x2804: value = net->rdbah; break;
        case 0x2808: value = net->rdlen; break;
        case 0x2810: value = net->rdh; break;
        case 0x2818: value = net->rdt; break;
        case 0x3800: value = net->tdbal; break;
        case 0x3804: value = net->tdbah; break;
        case 0x3808: value = net->tdlen; break;
        case 0x3810: value = net->tdh; break;
        case 0x3818: value = net->tdt; break;
        case 0x5400: value = read32(net->mac); break;
        case 0x5404: value = 0x80000000u | net->mac[4] | (uint32_t)net->mac[5] << 8; break;
        default: value = 0; break;
    }
    *output = value;
    return GHOSTOS_VM_E1000_OK;
}

uint32_t ghostos_vm_e1000_write(ghostos_vm_e1000 *net, uint64_t address,
    uint8_t size, uint32_t value, const ghostos_vm_e1000_io *io) {
    if (size != 4 || !net) return GHOSTOS_VM_E1000_SIZE;
    switch (address & (GHOSTOS_VM_E1000_MMIO_SIZE - 1)) {
        case 0:
            if (value & (1u << 26)) ghostos_vm_e1000_reset(net);
            else net->ctrl = (net->ctrl & ~(1u << 6)) | (value & (1u << 6));
            break;
        case 0xc0: net->icr &= ~value; break;
        case 0xd0: net->ims |= value; check_interrupt(net, io); break;
        case 0xd8: net->ims &= ~value; check_interrupt(net, io); break;
        case 0x100: {
            uint32_t old = net->rctl;
            net->rctl = value;
            if (!(old & 1) && (value & 1)) { ghostos_vm_e1000_clear_rx(net); net->rdh = 0; }
            break;
        }
        case 0x400: net->tctl = value; break;
        case 0x2800: net->rdbal = value; break;
        case 0x2804: net->rdbah = value; break;
        case 0x2808: net->rdlen = value; break;
        case 0x2810: net->rdh = value; break;
        case 0x2818: net->rdt = value; break;
        case 0x3800: net->tdbal = value; break;
        case 0x3804: net->tdbah = value; break;
        case 0x3808: net->tdlen = value; break;
        case 0x3810: net->tdh = value; break;
        case 0x3818: net->tdt = value; break;
        default: break;
    }
    return GHOSTOS_VM_E1000_OK;
}

static size_t ring_length(uint32_t length) {
    size_t count = length / 16;
    return count < 4096 ? count : 4096;
}
static bool descriptor_address(uint64_t base, size_t index, uint64_t *address) {
    uint64_t offset = index * 16;
    if (base > UINT64_MAX - offset) return false;
    *address = base + offset;
    return true;
}
static uint32_t poll_tx(ghostos_vm_e1000 *net, const ghostos_vm_e1000_io *io, bool checked) {
    size_t count = ring_length(net->tdlen), sent = 0;
    if (!count) return GHOSTOS_VM_E1000_OK;
    uint64_t base = net->tdbal | (uint64_t)net->tdbah << 32;
    while (sent < count) {
        size_t index = net->tdh % count;
        uint8_t descriptor[16];
        uint64_t address;
        if (!descriptor_address(base, index, &address) ||
            !io->read_memory(io->context, address, descriptor, sizeof(descriptor))) break;
        size_t length = read32(descriptor + 8) & 0xfff;
        if (!length) break;
        uint8_t packet[4095];
        if (!io->read_memory(io->context, read64(descriptor), packet, length)) break;
        int32_t result = io->transmit ? io->transmit(io->context, packet, length) : 5;
        if (result >= 0) net->last_error = result;
        uint8_t status = descriptor[12] | 3;
        if (address > UINT64_MAX - 12) break;
        (void)io->write_memory(io->context, address + 12, &status, 1);
        if (checked && net->tdh == UINT32_MAX) return GHOSTOS_VM_E1000_LEGACY_OVERFLOW;
        net->tdh = (net->tdh + 1) % (uint32_t)count;
        ++sent;
    }
    if (sent) ghostos_vm_e1000_signal(net, 1, io);
    return GHOSTOS_VM_E1000_OK;
}

static void poll_rx(ghostos_vm_e1000 *net, const ghostos_vm_e1000_io *io) {
    size_t count = ring_length(net->rdlen), delivered = 0;
    if (!count) return;
    uint64_t base = net->rdbal | (uint64_t)net->rdbah << 32;
    while (ghostos_vm_packet_queue_len(net->pending_rx)) {
        size_t length;
        const uint8_t *packet = ghostos_vm_packet_queue_peek(net->pending_rx, &length);
        size_t index = ((uint64_t)net->rdt + 1) % count;
        uint8_t descriptor[16];
        uint64_t address;
        if (!descriptor_address(base, index, &address) ||
            !io->read_memory(io->context, address, descriptor, sizeof(descriptor))) break;
        size_t capacity = read32(descriptor + 8) & 0xffff;
        if (length < 6) { (void)ghostos_vm_packet_queue_pop(net->pending_rx); continue; }
        if (length > capacity || !io->write_memory(io->context, read64(descriptor), packet, length)) break;
        bool broadcast = true;
        for (size_t i = 0; i < 6; ++i) if (packet[i] != 0xff) broadcast = false;
        uint8_t completion[] = {(uint8_t)length, (uint8_t)(length >> 8),
            (uint8_t)(length >> 16), broadcast ? 7 : 3};
        if (address > UINT64_MAX - 8) break;
        (void)io->write_memory(io->context, address + 8, completion, sizeof(completion));
        (void)ghostos_vm_packet_queue_pop(net->pending_rx);
        net->rdt = (uint32_t)index;
        ++delivered;
    }
    if (delivered) ghostos_vm_e1000_signal(net, 1u << 6, io);
}

uint32_t ghostos_vm_e1000_poll(ghostos_vm_e1000 *net,
    const ghostos_vm_e1000_io *io, bool checked) {
    if (!net || !io || !io->read_memory || !io->write_memory) return GHOSTOS_VM_E1000_OK;
    if (net->tctl & 1) {
        uint32_t result = poll_tx(net, io, checked);
        if (result) return result;
    }
    if (net->rctl & 1) {
        if (io->receive) {
            for (;;) {
                const uint8_t *packet = NULL;
                size_t length = 0;
                int32_t result = io->receive(io->context, &packet, &length);
                if (result == 0) break;
                if (result < 0) { net->last_error = -(result + 1); break; }
                if (!ghostos_vm_packet_queue_push(net->pending_rx, packet, length)) {
                    net->last_error = 2;
                    break;
                }
            }
        } else net->last_error = 5;
        poll_rx(net, io);
    }
    return GHOSTOS_VM_E1000_OK;
}
