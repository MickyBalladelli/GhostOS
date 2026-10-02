#ifndef GHOSTOS_VM_HOST_NET_H
#define GHOSTOS_VM_HOST_NET_H

#include "ghostos/vm_mac.h"

typedef struct ghostos_vm_host_net ghostos_vm_host_net;
typedef struct {
    bool (*link_up)(void *context);
    /* Send: -1 success or NetError (0..5). Receive: 1 data, 0 would-block,
     * -1 host failure. Callbacks are synchronous and never retained. */
    int32_t (*send)(void *context, const uint8_t *frame, size_t length);
    int32_t (*receive)(void *context, uint8_t *frame, size_t capacity, size_t *length);
    void *context;
} ghostos_vm_host_net_io;

ghostos_vm_host_net *ghostos_vm_host_net_new(const ghostos_vm_mac_address *mac, bool udp);
void ghostos_vm_host_net_free(ghostos_vm_host_net *net);
bool ghostos_vm_host_net_admin(const ghostos_vm_host_net *net);
void ghostos_vm_host_net_set_admin(ghostos_vm_host_net *net, bool up);
void ghostos_vm_host_net_set_promiscuous(ghostos_vm_host_net *net, bool enabled);
size_t ghostos_vm_host_net_transmitted(const ghostos_vm_host_net *net);
size_t ghostos_vm_host_net_received(const ghostos_vm_host_net *net);
/* -1 success or NetError. Receive: 1 packet, 0 empty, -(NetError + 1).
 * Output capacity is 1518 bytes. Invalid frames consume one datagram and
 * return empty, preserving the host backend's existing receive behavior. */
int32_t ghostos_vm_host_net_transmit(ghostos_vm_host_net *net, const ghostos_vm_host_net_io *io,
    const uint8_t *packet, size_t length);
int32_t ghostos_vm_host_net_receive(ghostos_vm_host_net *net, const ghostos_vm_host_net_io *io,
    uint8_t output[1518], size_t *length);

#endif
