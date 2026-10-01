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

#endif
