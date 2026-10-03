#ifndef GHOSTOS_CONFIG_NETWORK_H
#define GHOSTOS_CONFIG_NETWORK_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Uses the config parser result codes. Interface mode: static=0, dhcp=1. */
#define GHOSTOS_CONFIG_INTERFACE_NAME 48u
#define GHOSTOS_CONFIG_ADDRESS 64u
typedef struct {
    uint8_t name[GHOSTOS_CONFIG_INTERFACE_NAME];
    uint8_t address[GHOSTOS_CONFIG_ADDRESS];
    uint8_t name_length, address_length, mode;
    bool enabled;
    uint32_t mtu;
} ghostos_config_interface;
typedef struct {
    uint8_t destination[GHOSTOS_CONFIG_ADDRESS];
    uint8_t gateway[GHOSTOS_CONFIG_ADDRESS];
    uint8_t interface_name[GHOSTOS_CONFIG_INTERFACE_NAME];
    uint8_t destination_length, gateway_length, interface_length;
    uint32_t metric;
} ghostos_config_route;
int ghostos_config_parse_network(const uint8_t *source, size_t length, uint8_t *hostname, size_t hostname_capacity,
    size_t *hostname_length, ghostos_config_interface *interfaces, size_t interface_capacity, size_t *interface_count,
    ghostos_config_route *routes, size_t route_capacity, size_t *route_count);
#endif
