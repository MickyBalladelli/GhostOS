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

typedef struct {
    uint8_t server_ip[4], pool_start[4], pool_end[4], subnet_mask[4], gateway[4], dns[4];
    uint32_t lease_time_secs;
} ghostos_vm_dhcp_config;
typedef struct { uint8_t mac[6], address[4]; } ghostos_vm_dhcp_reservation;
/* 0 success; 1 invalid pool, 2 pool too large, 3 lease duration,
 * 4 reservation count, 5 duplicate reservation, 6 reservation outside pool. */
uint32_t ghostos_vm_dhcp_validate_config(const ghostos_vm_dhcp_config *config,
    const ghostos_vm_dhcp_reservation *reservations, size_t count);
/* Maximum output capacity needed is 618 bytes. False leaves length zero. */
bool ghostos_vm_dhcp_encode_reply(const ghostos_vm_dhcp_config *config,
    const ghostos_vm_dhcp_request *request, uint8_t message_type,
    const uint8_t address[4], uint8_t *output, size_t capacity, size_t *length);

bool ghostos_vm_dhcp_write_option(uint8_t *output, size_t output_capacity,
    size_t cursor, uint8_t code, const uint8_t *value, size_t value_length,
    size_t *output_cursor);
bool ghostos_vm_dhcp_decode_request(const uint8_t *frame, size_t frame_length,
    ghostos_vm_dhcp_request *request);

#endif
