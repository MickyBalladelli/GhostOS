#ifndef GHOSTOS_WEBAUTHN_H
#define GHOSTOS_WEBAUTHN_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_WEBAUTHN_MAX_ASSERTION 512u
#define GHOSTOS_WEBAUTHN_MAX_KEY_BYTES 96u
#define GHOSTOS_WEBAUTHN_MAX_KEYS 4u

void ghostos_webauthn_sha256(const uint8_t *input, size_t input_length, uint8_t output[32]);

#endif
