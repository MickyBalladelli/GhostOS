#ifndef GHOSTOS_VM_SEGMENT_H
#define GHOSTOS_VM_SEGMENT_H

#include "ghostos/vm_mac.h"

typedef struct ghostos_vm_segment ghostos_vm_segment;
/* NetError values 0..5, -1 success, -2 host allocation failure. */
ghostos_vm_segment *ghostos_vm_segment_new(size_t ports, bool loopback);
void ghostos_vm_segment_free(ghostos_vm_segment *segment);
void ghostos_vm_segment_set_link(ghostos_vm_segment *segment, bool up);
bool ghostos_vm_segment_link(const ghostos_vm_segment *segment);
void ghostos_vm_segment_set_limit(ghostos_vm_segment *segment, size_t limit);
void ghostos_vm_segment_drop_next(ghostos_vm_segment *segment, size_t count);
void ghostos_vm_segment_clear(ghostos_vm_segment *segment);
bool ghostos_vm_segment_disconnect(ghostos_vm_segment *segment, const ghostos_vm_mac_address *mac);
int32_t ghostos_vm_segment_connect(ghostos_vm_segment *segment, const ghostos_vm_mac_address *mac, size_t *port);
bool ghostos_vm_segment_admin(const ghostos_vm_segment *segment, size_t port);
void ghostos_vm_segment_set_admin(ghostos_vm_segment *segment, size_t port, bool up);
void ghostos_vm_segment_set_promiscuous(ghostos_vm_segment *segment, size_t port, bool enabled);
size_t ghostos_vm_segment_queued(const ghostos_vm_segment *segment, size_t port);
size_t ghostos_vm_segment_transmitted(const ghostos_vm_segment *segment);
int32_t ghostos_vm_segment_transmit(ghostos_vm_segment *segment, size_t port, const uint8_t *packet, size_t length);
/* Receive copies and removes the first matching frame. 1 frame, 0 empty,
 * -(NetError + 1) failure. Capacity must be at least 1518 bytes. */
int32_t ghostos_vm_segment_receive(ghostos_vm_segment *segment, size_t port, uint8_t output[1518], size_t *length);
/* Loopback permits independent handles for the same port index. Their MAC,
 * admin and promiscuous settings belong to the handle, not the hub. */
int32_t ghostos_vm_loopback_transmit(ghostos_vm_segment *hub, size_t port, bool admin_up, const uint8_t *packet, size_t length);
int32_t ghostos_vm_loopback_receive(ghostos_vm_segment *hub, size_t port, const ghostos_vm_mac_address *mac,
    bool admin_up, bool promiscuous, uint8_t output[1518], size_t *length);

#endif
