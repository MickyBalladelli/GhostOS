#ifndef GHOSTOS_RANDOM_H
#define GHOSTOS_RANDOM_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_RANDOM_MAX_REQUEST_BYTES 4096u

void ghostos_random_initialize(void);
bool ghostos_random_ready(void);
bool ghostos_random_fill(uint8_t *output, size_t length);

#endif
