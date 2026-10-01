#ifndef GHOSTOS_WEBAUTHN_H
#define GHOSTOS_WEBAUTHN_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_WEBAUTHN_MAX_ASSERTION 512u
#define GHOSTOS_WEBAUTHN_MAX_KEY_BYTES 96u
#define GHOSTOS_WEBAUTHN_MAX_KEYS 4u

typedef enum {
    GHOSTOS_WEBAUTHN_KEY_VALID = 0,
    GHOSTOS_WEBAUTHN_KEY_INVALID_CBOR,
    GHOSTOS_WEBAUTHN_KEY_NOT_ES256,
    GHOSTOS_WEBAUTHN_KEY_COORDINATE_RANGE,
    GHOSTOS_WEBAUTHN_KEY_POINT_INVALID
} ghostos_webauthn_key_error;

bool ghostos_webauthn_valid_cose_es256_public_key(const uint8_t *key, size_t key_length);
ghostos_webauthn_key_error ghostos_webauthn_cose_es256_public_key_error(
    const uint8_t *key, size_t key_length);
bool ghostos_webauthn_verify_assertion(const uint8_t *assertion, size_t assertion_length,
    const uint8_t challenge[32], const uint8_t keys[GHOSTOS_WEBAUTHN_MAX_KEYS][GHOSTOS_WEBAUTHN_MAX_KEY_BYTES],
    const uint8_t key_lengths[GHOSTOS_WEBAUTHN_MAX_KEYS], size_t key_count,
    size_t *matched_key, uint8_t key_fingerprint[32], uint32_t *sign_count);

#endif
