#ifndef GHOSTOS_CONFIG_CAPABILITY_H
#define GHOSTOS_CONFIG_CAPABILITY_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Uses the config parser result codes, plus 13 for an invalid rights list.
   Kind: ipc=0, file=1, network=2, memory=3, device=4, clock=5.
   Rights: read=1, write=2, execute=4, map=8, bind=16, connect=32, send=64, receive=128, admin=256. */
#define GHOSTOS_CONFIG_CAPABILITY_SERVICE 48u
#define GHOSTOS_CONFIG_CAPABILITY_RESOURCE 64u
typedef struct {
    uint8_t service[GHOSTOS_CONFIG_CAPABILITY_SERVICE];
    uint8_t resource[GHOSTOS_CONFIG_CAPABILITY_RESOURCE];
    uint8_t service_length, resource_length, kind;
    bool required;
    uint16_t rights;
} ghostos_config_capability;
int ghostos_config_parse_capabilities(const uint8_t *source, size_t length, const uint8_t *const *service_names,
    const uint8_t *service_lengths, size_t service_count, ghostos_config_capability *capabilities, size_t capacity,
    size_t *count);
#endif
