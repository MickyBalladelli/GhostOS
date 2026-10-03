#include "ghostos/kv_cache.h"
#include <assert.h>
#include <string.h>
static void allocation_rounds_up_and_cache_rejects_the_following_token(void) {
    ghostos_kv_allocation allocations[2];
    ghostos_kv_cache caches[2];
    uint64_t handle = 0, start = 0, length = 0, cache = 0, reserved = 0;
    memset(allocations, 0, sizeof allocations);
    memset(caches, 0, sizeof caches);
    assert(!ghostos_kv_allocate(allocations, 2, 0x100000000ull, 0x20000, 5000, 4096, &handle, &start, &length));
    assert(length == 8192 && start == 0x100000000ull);
    assert(!ghostos_kv_resolve(allocations, 2, handle, 0));
    assert(ghostos_kv_resolve(allocations, 2, handle, length) == 5);
    assert(!ghostos_kv_release(allocations, 2, handle));
    assert(ghostos_kv_info(allocations, 2, handle, &length) == 1);
    assert(!ghostos_kv_open(caches, 2, 3, 4096, 2, &cache, &reserved));
    assert(reserved == 2);
    assert(!ghostos_kv_commit(caches, 2, cache, 1));
    assert(caches[0].committed_tokens == 1);
    assert(ghostos_kv_token(caches, 2, cache, 2, 0) == 5);
}
int main(void) {
    allocation_rounds_up_and_cache_rejects_the_following_token();
    return 0;
}
