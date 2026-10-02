#ifndef GHOSTOS_VM_DHCP_H
#define GHOSTOS_VM_DHCP_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

bool ghostos_vm_dhcp_write_option(uint8_t *output, size_t output_capacity,
    size_t cursor, uint8_t code, const uint8_t *value, size_t value_length,
    size_t *output_cursor);

#endif
