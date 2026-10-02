#include "ghostos/vm_dhcp.h"

bool ghostos_vm_dhcp_write_option(uint8_t *output, size_t output_capacity,
    size_t cursor, uint8_t code, const uint8_t *value, size_t value_length,
    size_t *output_cursor) {
    if (!output || !output_cursor || (!value && value_length) || value_length > UINT8_MAX ||
        cursor > output_capacity || output_capacity - cursor < 2 ||
        value_length > output_capacity - cursor - 2) return false;
    output[cursor] = code;
    output[cursor + 1] = (uint8_t)value_length;
    for (size_t index = 0; index < value_length; ++index)
        output[cursor + 2 + index] = value[index];
    *output_cursor = cursor + 2 + value_length;
    return true;
}
