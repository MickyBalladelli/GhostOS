#ifndef GHOSTOS_PATH_PATTERN_H
#define GHOSTOS_PATH_PATTERN_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_PATTERN_BYTES 192

typedef enum {
    GHOSTOS_PATTERN_OK = 0,
    GHOSTOS_PATTERN_EMPTY,
    GHOSTOS_PATTERN_TOO_LONG,
    GHOSTOS_PATTERN_INVALID_PATH,
    GHOSTOS_PATTERN_TRAILING_ESCAPE,
    GHOSTOS_PATTERN_UNTERMINATED_CLASS,
    GHOSTOS_PATTERN_EMPTY_CLASS,
    GHOSTOS_PATTERN_INVALID_RANGE
} ghostos_pattern_error;

/* Byte slices are borrowed. Rust callers supply UTF-8; matching retains the
 * original byte-based class behavior and UTF-8-width wildcard consumption. */
ghostos_pattern_error ghostos_pattern_parse(const uint8_t *pattern,
    size_t length, bool *magic);
bool ghostos_pattern_matches(const uint8_t *pattern, size_t pattern_length,
    const uint8_t *candidate, size_t candidate_length);
ghostos_pattern_error ghostos_pattern_unescape(const uint8_t *pattern,
    size_t length, uint8_t *output, size_t capacity, size_t *written);

#endif
