#include "ghostos/backup_crypto.h"
#include "ghostos/vm_snapshot_auth.h"
#include <assert.h>
#include <string.h>
static void encrypted_chunk_round_trips_and_rejects_a_changed_tag(void) {
    uint8_t key[32], digest[32], object_id[32], encoded[64], plain[8];
    const uint8_t text[] = "old";
    size_t encoded_length = 0;
    size_t written = 0;
    size_t i;
    for (i = 0; i < 32; ++i) key[i] = (uint8_t)(i + 1);
    ghostos_vm_snapshot_sha256(text, 3, digest);
    assert(!ghostos_backup_object_id(key, digest, object_id));
    assert(ghostos_backup_encrypt(key, digest, text, 4097, encoded, sizeof encoded, &encoded_length) == 1);
    assert(!ghostos_backup_encrypt(key, digest, text, 3, encoded, sizeof encoded, &encoded_length));
    assert(encoded_length == 47);
    assert(!ghostos_backup_decrypt(key, object_id, encoded, encoded_length, digest, plain, sizeof plain, &written));
    assert(written == 3 && !memcmp(plain, text, 3));
    encoded[encoded_length - 1] ^= 1;
    assert(ghostos_backup_decrypt(key, object_id, encoded, encoded_length, digest, plain, sizeof plain, &written) == 4);
    assert(ghostos_backup_decrypt(key, object_id, encoded, 10, digest, plain, sizeof plain, &written) == 3);
}
int main(void) {
    encrypted_chunk_round_trips_and_rejects_a_changed_tag();
    return 0;
}
