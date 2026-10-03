#ifndef GHOSTOS_CONFIG_PARSER_H
#define GHOSTOS_CONFIG_PARSER_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 unknown section, 2 invalid value, 3 unknown key,
 * 4 duplicate key, 5 invalid integer, 6 unsupported schema, 7 missing field,
 * 8 invalid boolean, 9 section not implemented, 10 duplicate name,
 * 11 invalid string, 12 capacity.
 * Service kind: system=0, network=1, storage=2, compute=3.
 * Restart: never=0, on-failure=1, always=2. */
#define GHOSTOS_CONFIG_SERVICE_NAME 48u
typedef struct {
    uint8_t name[GHOSTOS_CONFIG_SERVICE_NAME];
    uint8_t name_length, kind, restart;
    bool enabled;
    uint64_t image;
} ghostos_config_service;
int ghostos_config_parse_system(const uint8_t *source, size_t length, uint16_t *schema, uint64_t *revision);
int ghostos_config_parse_services(const uint8_t *source, size_t length, uint16_t *schema, uint64_t *revision,
    ghostos_config_service *services, size_t capacity, size_t *count);
#endif
