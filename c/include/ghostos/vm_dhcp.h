#ifndef GHOSTOS_VM_DHCP_H
#define GHOSTOS_VM_DHCP_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint8_t source_mac[6];
    uint8_t mac[6];
    uint32_t xid;
    uint16_t flags;
    uint8_t ciaddr[4];
    uint8_t message_type;
    uint8_t requested_ip_present;
    uint8_t requested_ip[4];
    uint8_t server_id_present;
    uint8_t server_id[4];
} ghostos_vm_dhcp_request;

bool ghostos_vm_dhcp_write_option(uint8_t *output, size_t output_capacity,
    size_t cursor, uint8_t code, const uint8_t *value, size_t value_length,
    size_t *output_cursor);
bool ghostos_vm_dhcp_decode_request(const uint8_t *frame, size_t frame_length,
    ghostos_vm_dhcp_request *request);

#endif
