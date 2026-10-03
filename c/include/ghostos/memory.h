#ifndef GHOSTOS_MEMORY_H
#define GHOSTOS_MEMORY_H

#include <stdbool.h>
#include <stddef.h>

/* Freestanding byte operations: no hosted headers or runtime dependencies.
 * Copy requires non-overlapping ranges, like the original memcpy calls. */
static inline void ghostos_memory_zero(void *destination, size_t length)
{
    unsigned char *bytes = destination;
    for (size_t index = 0; index < length; ++index) bytes[index] = 0;
}

static inline void ghostos_memory_copy(void *destination, const void *source, size_t length)
{
    unsigned char *output = destination;
    const unsigned char *input = source;
    for (size_t index = 0; index < length; ++index) output[index] = input[index];
}

static inline bool ghostos_memory_equal(const void *left, const void *right, size_t length)
{
    const unsigned char *a = left;
    const unsigned char *b = right;
    for (size_t index = 0; index < length; ++index)
        if (a[index] != b[index]) return false;
    return true;
}

#endif
