#ifndef GHOSTOS_CONFIG_PARSER_H
#define GHOSTOS_CONFIG_PARSER_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 unknown section, 2 invalid value, 3 unknown key,
 * 4 duplicate key, 5 invalid integer, 6 unsupported schema, 7 missing field,
 * 9 section not implemented by this parser. */
int ghostos_config_parse_system(const uint8_t *source, size_t length, uint16_t *schema, uint64_t *revision);
#endif
