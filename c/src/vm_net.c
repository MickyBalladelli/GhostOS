#include "ghostos/vm_net.h"

#include <string.h>

static const uint8_t host_frame_magic[] = { 'S', 'N', 'E', 'T' };
enum { ethernet_header_length = 14, ethernet_minimum_frame = 60,
    ethernet_maximum_frame = 1514 };

uint64_t ghostos_vm_net_align_up(uint64_t value, uint64_t alignment) {
    return (value + alignment - 1) & ~(alignment - 1);
}

bool ghostos_vm_net_host_frame_encode(const uint8_t *packet, size_t packet_length,
    uint8_t *output, size_t output_capacity, size_t *output_length) {
    if ((!packet && packet_length) || !output_length || packet_length < ethernet_header_length ||
        packet_length > ethernet_maximum_frame) return false;
    size_t padded_length = packet_length < ethernet_minimum_frame ?
        ethernet_minimum_frame : packet_length;
    size_t required = sizeof(host_frame_magic) + padded_length;
    *output_length = required;
    if (!output) return output_capacity == 0;
    if (output_capacity < required) return false;
    memcpy(output, host_frame_magic, sizeof(host_frame_magic));
    memcpy(output + sizeof(host_frame_magic), packet, packet_length);
    if (padded_length > packet_length)
        memset(output + sizeof(host_frame_magic) + packet_length, 0, padded_length - packet_length);
    return true;
}

bool ghostos_vm_net_host_frame_decode(const uint8_t *frame, size_t frame_length,
    size_t *packet_offset, size_t *packet_length) {
    if ((!frame && frame_length) || !packet_offset || !packet_length) return false;
    if (frame_length < sizeof(host_frame_magic) ||
        memcmp(frame, host_frame_magic, sizeof(host_frame_magic)) != 0) return false;
    *packet_offset = sizeof(host_frame_magic);
    *packet_length = frame_length - sizeof(host_frame_magic);
    return true;
}
