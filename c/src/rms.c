#include "ghostos/rms.h"
enum { GHOSTOS_RMS_ROOT_BYTES = 15, GHOSTOS_RMS_NAMESPACE_BYTES = 48 };
static const uint8_t GHOSTOS_RMS_ROOT[GHOSTOS_RMS_ROOT_BYTES] = {
    '/', '.', 'g', 'h', 'o', 's', 't', 'o', 's', '/', 'd', 'a', 't', 'a', '/'
};
static const uint8_t GHOSTOS_RMS_HEX[16] = {
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f'
};
int ghostos_rms_lock_path(const uint8_t *path, size_t length) {
    size_t i;
    if (!length) return 1;
    for (i = 0; i < length; ++i) if (!path[i]) return 1;
    return 0;
}
uint64_t ghostos_rms_resource_id(const uint8_t *path, size_t length) {
    uint64_t hash = UINT64_C(0xcbf29ce484222325);
    size_t i;
    for (i = 0; i < length; ++i) {
        hash ^= path[i];
        hash *= UINT64_C(0x100000001b3);
    }
    return hash < 1 ? 1 : hash;
}
static bool namespace_byte(uint8_t byte) {
    return (byte >= '0' && byte <= '9') || (byte >= 'A' && byte <= 'Z') ||
        (byte >= 'a' && byte <= 'z') || byte == '-' || byte == '_' || byte == '.';
}
int ghostos_rms_namespace(const uint8_t *name, size_t length) {
    size_t i;
    if (!length || length > GHOSTOS_RMS_NAMESPACE_BYTES) return 1;
    for (i = 0; i < length; ++i) if (!namespace_byte(name[i])) return 1;
    return 0;
}
int ghostos_rms_key_path(const uint8_t *namespace, size_t namespace_length,
    const uint8_t *key, size_t key_length, uint8_t *destination, size_t destination_length,
    size_t *written) {
    size_t key_bytes, total, cursor, i;
    if (!key_length) return 1;
    if (key_length > SIZE_MAX / 2) return 2;
    key_bytes = key_length * 2;
    if (namespace_length > SIZE_MAX - GHOSTOS_RMS_ROOT_BYTES) return 2;
    total = GHOSTOS_RMS_ROOT_BYTES + namespace_length;
    if (total > SIZE_MAX - 1) return 2;
    total += 1;
    if (key_bytes > SIZE_MAX - total) return 2;
    total += key_bytes;
    if (total > destination_length) return 2;
    for (i = 0; i < GHOSTOS_RMS_ROOT_BYTES; ++i) destination[i] = GHOSTOS_RMS_ROOT[i];
    cursor = GHOSTOS_RMS_ROOT_BYTES;
    for (i = 0; i < namespace_length; ++i) destination[cursor++] = namespace[i];
    destination[cursor++] = '/';
    for (i = 0; i < key_length; ++i) {
        destination[cursor++] = GHOSTOS_RMS_HEX[key[i] >> 4];
        destination[cursor++] = GHOSTOS_RMS_HEX[key[i] & 0x0f];
    }
    *written = cursor;
    return 0;
}
int ghostos_rms_reserve(size_t len, size_t capacity, size_t *index) {
    if (len >= capacity) return 1;
    *index = len;
    return 0;
}
bool ghostos_rms_add_count(uint32_t count, uint32_t *next) {
    if (count == UINT32_MAX) return false;
    *next = count + 1;
    return true;
}
bool ghostos_rms_sub_count(uint32_t count, uint32_t *next) {
    if (!count) return false;
    *next = count - 1;
    return true;
}
