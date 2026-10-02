#ifndef GHOSTOS_VM_PACKET_H
#define GHOSTOS_VM_PACKET_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_VM_ETHERNET_FRAME_MAX 1518u
#define GHOSTOS_VM_ETHERNET_FRAME_MIN 60u
#define GHOSTOS_VM_ETHERNET_HEADER_LEN 14u

typedef struct ghostos_vm_packet_queue ghostos_vm_packet_queue;

ghostos_vm_packet_queue *ghostos_vm_packet_queue_new(size_t max_packets,
    size_t max_bytes);
void ghostos_vm_packet_queue_free(ghostos_vm_packet_queue *queue);
bool ghostos_vm_packet_queue_push(ghostos_vm_packet_queue *queue,
    const uint8_t *packet, size_t length);
bool ghostos_vm_packet_queue_pop(ghostos_vm_packet_queue *queue);
const uint8_t *ghostos_vm_packet_queue_peek(const ghostos_vm_packet_queue *queue,
    size_t *length);
size_t ghostos_vm_packet_queue_len(const ghostos_vm_packet_queue *queue);
size_t ghostos_vm_packet_queue_bytes(const ghostos_vm_packet_queue *queue);
void ghostos_vm_packet_queue_clear(ghostos_vm_packet_queue *queue);
bool ghostos_vm_packet_pad(const uint8_t *packet, size_t length,
    uint8_t *output, size_t output_capacity, size_t *output_length);
const char *ghostos_vm_net_error_message(uint32_t error);
const char *ghostos_vm_net_error_code(uint32_t error);

#endif
