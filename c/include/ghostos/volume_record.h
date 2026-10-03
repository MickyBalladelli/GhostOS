#ifndef GHOSTOS_VOLUME_RECORD_H
#define GHOSTOS_VOLUME_RECORD_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 corrupt, 2 buffer too small.
   The name field is 192 bytes. File type 0 decodes as regular. A zero link
   count decodes as 1. Deleted must be 0 or 1. */
#define GHOSTOS_VOLUME_NAME 192u
#define GHOSTOS_VOLUME_RECORD (2u + GHOSTOS_VOLUME_NAME + 4u + 8u + 8u + 4u + 8u + 8u + 1u + 1u + 4u + 2u)
typedef struct {
    uint8_t name[GHOSTOS_VOLUME_NAME];
    uint16_t name_length;
    uint32_t version;
    uint64_t object_id, size, checksum, created_at;
    uint32_t data, link_count;
    uint16_t mode;
    uint8_t file_type;
    bool deleted;
} ghostos_volume_record;
int ghostos_volume_encode_record(const ghostos_volume_record *record, uint8_t *bytes, size_t capacity, size_t *written);
int ghostos_volume_decode_record(const uint8_t *bytes, size_t length, ghostos_volume_record *record, size_t *consumed);
#endif
