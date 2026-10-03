#include "ghostos/fs_transaction.h"
#include <assert.h>
#include <string.h>
static void abandoned_rename_restores_the_old_name(void) {
    const uint8_t old_path[] = {'/', 'o', 'l', 'd'};
    const uint8_t new_path[] = {'/', 'n', 'e', 'w'};
    ghostos_fs_name names[1];
    ghostos_fs_transaction transaction;
    uint64_t generation = 3, committed_generation = 0;
    uint32_t operations = 0;
    memset(&transaction, 0, sizeof transaction);
    memset(names, 0, sizeof names);
    names[0].occupied = true;
    names[0].length = sizeof old_path;
    memcpy(names[0].bytes, old_path, sizeof old_path);
    assert(!ghostos_fs_transaction_begin(&transaction, names, 1, generation));
    assert(!ghostos_fs_transaction_rename(&transaction, names, old_path, sizeof old_path, new_path, sizeof new_path));
    assert(!ghostos_fs_transaction_lookup(names, 1, new_path, sizeof new_path));
    assert(!ghostos_fs_transaction_abort(&transaction, names, &generation));
    assert(generation == 3);
    assert(!ghostos_fs_transaction_lookup(names, 1, old_path, sizeof old_path));
    assert(ghostos_fs_transaction_lookup(names, 1, new_path, sizeof new_path) == 4);
    assert(!ghostos_fs_transaction_begin(&transaction, names, 1, generation));
    assert(ghostos_fs_transaction_rename(&transaction, names, new_path, sizeof new_path, old_path, sizeof old_path) == 4);
    assert(ghostos_fs_transaction_commit(&transaction, &committed_generation, &operations) == 1);
    assert(!ghostos_fs_transaction_abort(&transaction, names, &generation));
    assert(generation == 3 && names[0].occupied && names[0].length == sizeof old_path);
    assert(!ghostos_fs_transaction_begin(&transaction, names, 1, generation));
    assert(!ghostos_fs_transaction_rename(&transaction, names, old_path, sizeof old_path, new_path, sizeof new_path));
    assert(!ghostos_fs_transaction_commit(&transaction, &committed_generation, &operations));
    assert(committed_generation == 4 && operations == 1);
    assert(!ghostos_fs_transaction_lookup(names, 1, new_path, sizeof new_path));
    assert(ghostos_fs_transaction_lookup(names, 1, old_path, sizeof old_path) == 4);
}
int main(void) {
    abandoned_rename_restores_the_old_name();
    return 0;
}
