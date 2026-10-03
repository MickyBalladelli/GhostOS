#include "ghostos/volume_record.h"
static int put_bytes(uint8_t *bytes, size_t capacity, size_t *cursor, const uint8_t *value, size_t length) {
    size_t i;
    if (*cursor > capacity || length > capacity - *cursor) return 2;
    for (i = 0; i < length; ++i) bytes[*cursor + i] = value[i];
    *cursor += length;
    return 0;
}
static int put_u8(uint8_t *bytes, size_t capacity, size_t *cursor, uint8_t value) {
    return put_bytes(bytes, capacity, cursor, &value, 1);
}
static int put_u16(uint8_t *bytes, size_t capacity, size_t *cursor, uint16_t value) {
    uint8_t encoded[2] = {(uint8_t)value, (uint8_t)(value >> 8)};
    return put_bytes(bytes, capacity, cursor, encoded, 2);
}
static int put_u32(uint8_t *bytes, size_t capacity, size_t *cursor, uint32_t value) {
    uint8_t encoded[4] = {(uint8_t)value, (uint8_t)(value >> 8), (uint8_t)(value >> 16), (uint8_t)(value >> 24)};
    return put_bytes(bytes, capacity, cursor, encoded, 4);
}
static int put_u64(uint8_t *bytes, size_t capacity, size_t *cursor, uint64_t value) {
    uint8_t encoded[8];
    size_t i;
    for (i = 0; i < 8; ++i) encoded[i] = (uint8_t)(value >> (8 * i));
    return put_bytes(bytes, capacity, cursor, encoded, 8);
}
static int take(const uint8_t *bytes, size_t length, size_t *cursor, size_t count, const uint8_t **value) {
    if (*cursor > length || count > length - *cursor) return 1;
    *value = bytes + *cursor;
    *cursor += count;
    return 0;
}
static int take_u8(const uint8_t *bytes, size_t length, size_t *cursor, uint8_t *value) {
    const uint8_t *encoded = 0;
    if (take(bytes, length, cursor, 1, &encoded)) return 1;
    *value = encoded[0];
    return 0;
}
static int take_u16(const uint8_t *bytes, size_t length, size_t *cursor, uint16_t *value) {
    const uint8_t *encoded = 0;
    if (take(bytes, length, cursor, 2, &encoded)) return 1;
    *value = (uint16_t)encoded[0] | ((uint16_t)encoded[1] << 8);
    return 0;
}
static int take_u32(const uint8_t *bytes, size_t length, size_t *cursor, uint32_t *value) {
    const uint8_t *encoded = 0;
    if (take(bytes, length, cursor, 4, &encoded)) return 1;
    *value = (uint32_t)encoded[0] | ((uint32_t)encoded[1] << 8) | ((uint32_t)encoded[2] << 16) | ((uint32_t)encoded[3] << 24);
    return 0;
}
static int take_u64(const uint8_t *bytes, size_t length, size_t *cursor, uint64_t *value) {
    const uint8_t *encoded = 0;
    size_t i;
    uint64_t parsed = 0;
    if (take(bytes, length, cursor, 8, &encoded)) return 1;
    for (i = 0; i < 8; ++i) parsed |= (uint64_t)encoded[i] << (8 * i);
    *value = parsed;
    return 0;
}
int ghostos_volume_encode_record(const ghostos_volume_record *record, uint8_t *bytes, size_t capacity, size_t *written) {
    uint8_t name[GHOSTOS_VOLUME_NAME];
    size_t cursor = 0, i;
    int status;
    if (record->name_length > GHOSTOS_VOLUME_NAME) return 1;
    for (i = 0; i < GHOSTOS_VOLUME_NAME; ++i) name[i] = i < record->name_length ? record->name[i] : 0;
    status = put_u16(bytes, capacity, &cursor, record->name_length);
    if (!status) status = put_bytes(bytes, capacity, &cursor, name, GHOSTOS_VOLUME_NAME);
    if (!status) status = put_u32(bytes, capacity, &cursor, record->version);
    if (!status) status = put_u64(bytes, capacity, &cursor, record->object_id);
    if (!status) status = put_u64(bytes, capacity, &cursor, record->size);
    if (!status) status = put_u32(bytes, capacity, &cursor, record->data);
    if (!status) status = put_u64(bytes, capacity, &cursor, record->checksum);
    if (!status) status = put_u64(bytes, capacity, &cursor, record->created_at);
    if (!status) status = put_u8(bytes, capacity, &cursor, record->deleted ? 1 : 0);
    if (!status) status = put_u8(bytes, capacity, &cursor, record->file_type);
    if (!status) status = put_u32(bytes, capacity, &cursor, record->link_count);
    if (!status) status = put_u16(bytes, capacity, &cursor, record->mode);
    if (status) return status;
    *written = cursor;
    return 0;
}
int ghostos_volume_decode_record(const uint8_t *bytes, size_t length, ghostos_volume_record *record, size_t *consumed) {
    const uint8_t *name = 0;
    size_t cursor = 0, i;
    uint16_t name_length = 0, mode = 0;
    uint32_t version = 0, data = 0, link_count = 0;
    uint64_t object_id = 0, size = 0, checksum = 0, created_at = 0;
    uint8_t deleted = 0, file_type = 0;
    if (take_u16(bytes, length, &cursor, &name_length) || name_length > GHOSTOS_VOLUME_NAME) return 1;
    if (take(bytes, length, &cursor, GHOSTOS_VOLUME_NAME, &name)) return 1;
    for (i = 0; i < name_length; ++i) if (name[i] == 0) return 1;
    if (take_u32(bytes, length, &cursor, &version) || take_u64(bytes, length, &cursor, &object_id) ||
        take_u64(bytes, length, &cursor, &size) || take_u32(bytes, length, &cursor, &data) ||
        take_u64(bytes, length, &cursor, &checksum) || take_u64(bytes, length, &cursor, &created_at) ||
        take_u8(bytes, length, &cursor, &deleted) || take_u8(bytes, length, &cursor, &file_type) ||
        take_u32(bytes, length, &cursor, &link_count) || take_u16(bytes, length, &cursor, &mode))
        return 1;
    if (deleted > 1) return 1;
    if (!file_type) file_type = 1;
    else if (file_type > 3) return 1;
    if (!link_count) link_count = 1;
    for (i = 0; i < name_length; ++i) record->name[i] = name[i];
    record->name_length = name_length;
    record->version = version;
    record->object_id = object_id;
    record->size = size;
    record->data = data;
    record->checksum = checksum;
    record->created_at = created_at;
    record->deleted = deleted != 0;
    record->file_type = file_type;
    record->link_count = link_count;
    record->mode = mode;
    *consumed = cursor;
    return 0;
}
