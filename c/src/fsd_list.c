#include "ghostos/fsd_list.h"
static void store_le16(uint8_t *bytes, uint16_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
}
static void store_le32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)value;
    bytes[1] = (uint8_t)(value >> 8);
    bytes[2] = (uint8_t)(value >> 16);
    bytes[3] = (uint8_t)(value >> 24);
}
static void store_le64(uint8_t *bytes, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) bytes[i] = (uint8_t)(value >> (8 * i));
}
int ghostos_fsd_list(uint16_t rights, const uint8_t *const *names, const uint8_t *name_lengths, size_t count, size_t skip,
    uint8_t *output, size_t output_capacity, size_t *written, size_t *next) {
    size_t index, cursor = 0;
    if ((rights & 1u) == 0) return 1;
    if (skip > count) skip = count;
    for (index = skip; index < count; ++index) {
        size_t required = 22u + name_lengths[index];
        size_t byte;
        if (required < 22u || cursor > output_capacity || required > output_capacity - cursor) {
            if (!cursor) return 2;
            *written = cursor;
            *next = index;
            return 0;
        }
        store_le16(output + cursor, name_lengths[index]);
        output[cursor + 2] = 1;
        output[cursor + 3] = 0;
        store_le64(output + cursor + 4, 0);
        store_le32(output + cursor + 12, 1);
        store_le32(output + cursor + 16, 1);
        store_le16(output + cursor + 20, 0644);
        for (byte = 0; byte < name_lengths[index]; ++byte) output[cursor + 22 + byte] = names[index][byte];
        cursor += required;
    }
    *written = cursor;
    *next = 0;
    return 0;
}
