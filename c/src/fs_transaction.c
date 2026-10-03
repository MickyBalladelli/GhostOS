#include "ghostos/fs_transaction.h"
static bool same_name(const ghostos_fs_name *name, const uint8_t *path, size_t length) {
    size_t i;
    if (!name->occupied || name->length != length) return false;
    for (i = 0; i < length; ++i) if (name->bytes[i] != path[i]) return false;
    return true;
}
static void copy_name(ghostos_fs_name *destination, const uint8_t *path, size_t length) {
    size_t i;
    destination->occupied = true;
    destination->length = (uint8_t)length;
    for (i = 0; i < length; ++i) destination->bytes[i] = path[i];
}
static int find_name(ghostos_fs_name *names, size_t count, const uint8_t *path, size_t length, size_t *index) {
    size_t i;
    for (i = 0; i < count; ++i) if (same_name(&names[i], path, length)) { *index = i; return 0; }
    return 4;
}
int ghostos_fs_transaction_begin(ghostos_fs_transaction *transaction, const ghostos_fs_name *names, size_t count, uint64_t generation) {
    size_t i;
    if (transaction->active) return 1;
    if (count > GHOSTOS_FS_TRANSACTION_NAMES) return 2;
    transaction->active = true;
    transaction->committed = false;
    transaction->failed = false;
    transaction->generation = generation;
    transaction->original_generation = generation;
    transaction->operations = 0;
    transaction->count = count;
    for (i = 0; i < count; ++i) transaction->saved[i] = names[i];
    return 0;
}
int ghostos_fs_transaction_rename(ghostos_fs_transaction *transaction, ghostos_fs_name *names, const uint8_t *old_path, size_t old_length, const uint8_t *new_path, size_t new_length) {
    size_t index = 0;
    int status;
    if (!transaction->active || transaction->committed) return 1;
    if (transaction->failed) return 1;
    if (!old_length || !new_length || old_length > GHOSTOS_FS_NAME || new_length > GHOSTOS_FS_NAME) {
        transaction->failed = true;
        return 4;
    }
    status = find_name(names, transaction->count, old_path, old_length, &index);
    if (status) { transaction->failed = true; return status; }
    if (transaction->operations == UINT32_MAX) { transaction->failed = true; return 2; }
    names[index].occupied = false;
    copy_name(&names[index], new_path, new_length);
    transaction->operations += 1;
    if (transaction->generation < UINT64_MAX) transaction->generation += 1;
    return 0;
}
int ghostos_fs_transaction_lookup(const ghostos_fs_name *names, size_t count, const uint8_t *path, size_t length) {
    size_t index = 0;
    return find_name((ghostos_fs_name *)names, count, path, length, &index);
}
int ghostos_fs_transaction_commit(ghostos_fs_transaction *transaction, uint64_t *generation, uint32_t *operations) {
    if (!transaction->active || transaction->failed) return 1;
    transaction->committed = true;
    transaction->active = false;
    *generation = transaction->generation;
    *operations = transaction->operations;
    return 0;
}
int ghostos_fs_transaction_abort(ghostos_fs_transaction *transaction, ghostos_fs_name *names, uint64_t *generation) {
    size_t i;
    if (transaction->committed) return 0;
    for (i = 0; i < transaction->count; ++i) names[i] = transaction->saved[i];
    *generation = transaction->original_generation;
    transaction->active = false;
    transaction->failed = false;
    return 0;
}
