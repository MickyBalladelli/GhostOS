#ifndef GHOSTOS_VM_NET_H
#define GHOSTOS_VM_NET_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>

uint64_t ghostos_vm_net_align_up(uint64_t value, uint64_t alignment);
bool ghostos_vm_net_host_frame_encode(const uint8_t *packet, size_t packet_length,
    uint8_t *output, size_t output_capacity, size_t *output_length);
bool ghostos_vm_net_host_frame_decode(const uint8_t *frame, size_t frame_length,
    size_t *packet_offset, size_t *packet_length);
uint32_t ghostos_vm_net_validate_packet(size_t packet_length);
uint32_t ghostos_vm_net_ipv4_to_number(const uint8_t address[4]);
void ghostos_vm_net_ipv4_from_number(uint32_t value, uint8_t address[4]);
uint16_t ghostos_vm_net_checksum(const uint8_t *bytes, size_t length);

#endif
