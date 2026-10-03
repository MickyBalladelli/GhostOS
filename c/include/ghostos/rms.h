#ifndef GHOSTOS_RMS_H
#define GHOSTOS_RMS_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Lock path: 0 valid, 1 invalid. Namespace: 0 valid, 1 invalid.
 * Key path: 0 written, 1 empty key, 2 key too long.
 * Reserve: 0 stored the free index, 1 capacity. */
int ghostos_rms_lock_path(const uint8_t *path, size_t length);
uint64_t ghostos_rms_resource_id(const uint8_t *path, size_t length);
int ghostos_rms_namespace(const uint8_t *name, size_t length);
int ghostos_rms_key_path(const uint8_t *namespace, size_t namespace_length,
    const uint8_t *key, size_t key_length, uint8_t *destination, size_t destination_length,
    size_t *written);
int ghostos_rms_reserve(size_t len, size_t capacity, size_t *index);
bool ghostos_rms_add_count(uint32_t count, uint32_t *next);
bool ghostos_rms_sub_count(uint32_t count, uint32_t *next);
#endif
