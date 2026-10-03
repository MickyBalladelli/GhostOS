#include "ghostos/backup_crypto.h"
#include "ghostos/vm_snapshot_auth.h"
static void hmac(const uint8_t key[32], const uint8_t *message, size_t length, uint8_t tag[32]) {
    ghostos_vm_snapshot_auth_part part = {message, length};
    ghostos_vm_snapshot_hmac_sha256(key, &part, 1, tag);
}
static void nonce_for(const uint8_t key[32], const uint8_t digest[32], uint8_t nonce[12]) {
    uint8_t material[33] = {'n'};
    uint8_t tag[32];
    size_t i;
    for (i = 0; i < 32; ++i) material[1 + i] = digest[i];
    hmac(key, material, sizeof material, tag);
    for (i = 0; i < 12; ++i) nonce[i] = tag[i];
}
static uint8_t keystream_byte(const uint8_t key[32], const uint8_t nonce[12], size_t index) {
    uint8_t material[20];
    uint8_t tag[32];
    uint64_t block = index / 32;
    size_t i;
    for (i = 0; i < 12; ++i) material[i] = nonce[i];
    for (i = 0; i < 8; ++i) material[12 + i] = (uint8_t)(block >> (8 * i));
    hmac(key, material, sizeof material, tag);
    return tag[index % 32];
}
int ghostos_backup_object_id(const uint8_t key[32], const uint8_t plaintext_digest[32], uint8_t object_id[32]) {
    uint8_t material[33] = {'o'};
    size_t i;
    for (i = 0; i < 32; ++i) material[1 + i] = plaintext_digest[i];
    hmac(key, material, sizeof material, object_id);
    return 0;
}
int ghostos_backup_encrypt(const uint8_t key[32], const uint8_t plaintext_digest[32],
    const uint8_t *plaintext, size_t plaintext_length, uint8_t *output, size_t output_length, size_t *written) {
    uint8_t nonce[12];
    uint8_t tag[32];
    size_t required;
    size_t i;
    if (plaintext_length > GHOSTOS_BACKUP_CHUNK_BYTES) return 1;
    required = plaintext_length + GHOSTOS_BACKUP_CHUNK_OVERHEAD;
    if (output_length < required) return 2;
    nonce_for(key, plaintext_digest, nonce);
    for (i = 0; i < 12; ++i) output[i] = nonce[i];
    for (i = 0; i < plaintext_length; ++i) output[12 + i] = (uint8_t)(plaintext[i] ^ keystream_byte(key, nonce, i));
    hmac(key, output, 12 + plaintext_length, tag);
    for (i = 0; i < 32; ++i) output[12 + plaintext_length + i] = tag[i];
    *written = required;
    return 0;
}
int ghostos_backup_decrypt(const uint8_t key[32], const uint8_t object_id[32], const uint8_t *encoded,
    size_t encoded_length, const uint8_t plaintext_digest[32], uint8_t *output, size_t output_length, size_t *written) {
    uint8_t expected_id[32], expected_tag[32], nonce[12], derived[12], digest[32];
    size_t plaintext_length;
    size_t i;
    if (encoded_length < GHOSTOS_BACKUP_CHUNK_OVERHEAD) return 3;
    plaintext_length = encoded_length - GHOSTOS_BACKUP_CHUNK_OVERHEAD;
    if (plaintext_length > GHOSTOS_BACKUP_CHUNK_BYTES || output_length < plaintext_length) return 2;
    ghostos_backup_object_id(key, plaintext_digest, expected_id);
    if (!ghostos_vm_snapshot_auth_equal(expected_id, object_id, 32)) return 4;
    hmac(key, encoded, 12 + plaintext_length, expected_tag);
    if (!ghostos_vm_snapshot_auth_equal(expected_tag, encoded + 12 + plaintext_length, 32)) return 4;
    nonce_for(key, plaintext_digest, derived);
    for (i = 0; i < 12; ++i) nonce[i] = encoded[i];
    if (!ghostos_vm_snapshot_auth_equal(nonce, derived, 12)) return 4;
    for (i = 0; i < plaintext_length; ++i) output[i] = (uint8_t)(encoded[12 + i] ^ keystream_byte(key, nonce, i));
    ghostos_vm_snapshot_sha256(output, plaintext_length, digest);
    if (!ghostos_vm_snapshot_auth_equal(digest, plaintext_digest, 32)) return 4;
    *written = plaintext_length;
    return 0;
}
