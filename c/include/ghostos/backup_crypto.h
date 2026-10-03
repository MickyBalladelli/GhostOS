#ifndef GHOSTOS_BACKUP_CRYPTO_H
#define GHOSTOS_BACKUP_CRYPTO_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 chunk too large, 2 buffer too small, 3 invalid length,
 * 4 authentication failed. */
#define GHOSTOS_BACKUP_CHUNK_BYTES 4096u
#define GHOSTOS_BACKUP_CHUNK_OVERHEAD 44u
int ghostos_backup_object_id(const uint8_t key[32], const uint8_t plaintext_digest[32], uint8_t object_id[32]);
int ghostos_backup_encrypt(const uint8_t key[32], const uint8_t plaintext_digest[32],
    const uint8_t *plaintext, size_t plaintext_length, uint8_t *output, size_t output_length, size_t *written);
int ghostos_backup_decrypt(const uint8_t key[32], const uint8_t object_id[32], const uint8_t *encoded,
    size_t encoded_length, const uint8_t plaintext_digest[32], uint8_t *output, size_t output_length, size_t *written);
#endif
