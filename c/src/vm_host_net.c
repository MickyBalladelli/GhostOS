#include "ghostos/vm_host_net.h"
#include "ghostos/vm_net.h"
#include "ghostos/vm_packet.h"
#include <stdlib.h>
#include <string.h>

struct ghostos_vm_host_net {
    ghostos_vm_mac_address mac;
    size_t transmitted, received;
    bool udp, admin, promiscuous;
};
static void increment(size_t *value) { if (*value != SIZE_MAX) ++*value; }
ghostos_vm_host_net *ghostos_vm_host_net_new(const ghostos_vm_mac_address *mac, bool udp) {
    ghostos_vm_host_net *n = calloc(1, sizeof(*n));
    if (n) { n->mac = *mac; n->udp = udp; n->admin = true; }
    return n;
}
void ghostos_vm_host_net_free(ghostos_vm_host_net *n) { free(n); }
bool ghostos_vm_host_net_admin(const ghostos_vm_host_net *n) { return n->admin; }
void ghostos_vm_host_net_set_admin(ghostos_vm_host_net *n, bool up) { n->admin = up; }
void ghostos_vm_host_net_set_promiscuous(ghostos_vm_host_net *n, bool enabled) { n->promiscuous = enabled; }
size_t ghostos_vm_host_net_transmitted(const ghostos_vm_host_net *n) { return n->transmitted; }
size_t ghostos_vm_host_net_received(const ghostos_vm_host_net *n) { return n->received; }
int32_t ghostos_vm_host_net_transmit(ghostos_vm_host_net *n, const ghostos_vm_host_net_io *io,
    const uint8_t *packet, size_t length) {
    if (!n->admin) return 4;
    if (!io->link_up(io->context)) return 3;
    uint32_t validation = ghostos_vm_net_validate_packet(length);
    if (validation) return validation == 1 ? 1 : 0;
    uint8_t frame[4 + GHOSTOS_VM_ETHERNET_FRAME_MAX];
    size_t frame_length;
    bool encoded = n->udp
        ? ghostos_vm_net_host_frame_encode(packet, length, frame, sizeof(frame), &frame_length)
        : ghostos_vm_packet_pad(packet, length, frame, sizeof(frame), &frame_length);
    if (!encoded) return 5;
    int32_t result = io->send(io->context, frame, frame_length);
    if (result == -1) increment(&n->transmitted);
    return result;
}
int32_t ghostos_vm_host_net_receive(ghostos_vm_host_net *n, const ghostos_vm_host_net_io *io,
    uint8_t output[1518], size_t *length) {
    *length = 0;
    if (!n->admin) return -5;
    if (!io->link_up(io->context)) return -4;
    uint8_t frame[4 + GHOSTOS_VM_ETHERNET_FRAME_MAX];
    size_t received = 0, capacity = n->udp ? sizeof(frame) : GHOSTOS_VM_ETHERNET_FRAME_MAX;
    int32_t result = io->receive(io->context, frame, capacity, &received);
    if (result == 0) return 0;
    if (result != 1 || received > capacity) return -6;
    size_t offset = 0, packet_length = received;
    if (n->udp && !ghostos_vm_net_host_frame_decode(frame, received, &offset, &packet_length)) return 0;
    if (packet_length < GHOSTOS_VM_ETHERNET_HEADER_LEN
        || !ghostos_vm_mac_matches(frame + offset, 6, &n->mac, n->promiscuous)) return 0;
    memcpy(output, frame + offset, packet_length); *length = packet_length;
    increment(&n->received);
    return 1;
}
