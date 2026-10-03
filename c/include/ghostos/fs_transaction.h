#ifndef GHOSTOS_FS_TRANSACTION_H
#define GHOSTOS_FS_TRANSACTION_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 transaction aborted, 2 version overflow, 4 not found.
   Commit publishes the staged names. Abort, or a failed commit, restores the
   names and generation captured at begin. */
#define GHOSTOS_FS_NAME 32u
#define GHOSTOS_FS_TRANSACTION_NAMES 8u
typedef struct {
    bool occupied;
    uint8_t length;
    uint8_t bytes[GHOSTOS_FS_NAME];
} ghostos_fs_name;
typedef struct {
    bool active, committed, failed;
    uint64_t generation, original_generation;
    uint32_t operations;
    size_t count;
    ghostos_fs_name saved[GHOSTOS_FS_TRANSACTION_NAMES];
} ghostos_fs_transaction;
int ghostos_fs_transaction_begin(ghostos_fs_transaction *transaction, const ghostos_fs_name *names, size_t count, uint64_t generation);
int ghostos_fs_transaction_rename(ghostos_fs_transaction *transaction, ghostos_fs_name *names, const uint8_t *old_path, size_t old_length, const uint8_t *new_path, size_t new_length);
int ghostos_fs_transaction_lookup(const ghostos_fs_name *names, size_t count, const uint8_t *path, size_t length);
int ghostos_fs_transaction_commit(ghostos_fs_transaction *transaction, uint64_t *generation, uint32_t *operations);
int ghostos_fs_transaction_abort(ghostos_fs_transaction *transaction, ghostos_fs_name *names, uint64_t *generation);
#endif
