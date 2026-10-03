#ifndef GHOSTOS_PERSISTENCE_H
#define GHOSTOS_PERSISTENCE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_PERSISTENCE_COMMAND_PORT UINT16_C(0x5400)
#define GHOSTOS_PERSISTENCE_LENGTH_PORT UINT16_C(0x5404)
#define GHOSTOS_PERSISTENCE_DATA_PORT UINT16_C(0x5408)
#define GHOSTOS_PERSISTENCE_LOAD UINT8_C(1)
#define GHOSTOS_PERSISTENCE_SAVE UINT8_C(2)
#define GHOSTOS_PERSISTENCE_FLUSH UINT8_C(3)
#define GHOSTOS_PERSISTENCE_MAX_BYTES (60u * 1024u)

bool ghostos_persistence_load(uint8_t *bytes, size_t capacity, size_t *length);
void ghostos_persistence_save(const uint8_t *bytes, size_t length);

#define GHOSTOS_PERSISTENCE_BOOT_BYTES 56u
#define GHOSTOS_PERSISTENCE_CRASH_BYTES 1024u
#define GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES 24u
#define GHOSTOS_PERSISTENCE_CONTAINER_BYTES \
    (GHOSTOS_PERSISTENCE_CONTAINER_HEADER_BYTES + GHOSTOS_PERSISTENCE_BOOT_BYTES + GHOSTOS_PERSISTENCE_CRASH_BYTES)
typedef struct {
    uint8_t boot[GHOSTOS_PERSISTENCE_BOOT_BYTES];
    size_t boot_length;
    uint8_t crash[GHOSTOS_PERSISTENCE_CRASH_BYTES];
    size_t crash_length;
} ghostos_persistent_records;
void ghostos_persistence_records_decode(const uint8_t *bytes, size_t length,
    ghostos_persistent_records *records);
bool ghostos_persistence_records_encode(const ghostos_persistent_records *records,
    uint8_t *bytes, size_t capacity, size_t *length);
bool ghostos_persistence_records_update(ghostos_persistent_records *records,
    bool crash, const uint8_t *bytes, size_t length);
bool ghostos_persistence_records_boot(const ghostos_persistent_records *records,
    uint8_t *bytes, size_t capacity, size_t *length);

#endif
