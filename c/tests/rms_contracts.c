#include "ghostos/rms.h"
#include <assert.h>
#include <string.h>
static void paths_namespaces_and_counts_follow_existing_rules(void) {
    uint8_t path[64];
    size_t written = 0;
    uint32_t count = 0;
    size_t index = 9;
    const uint8_t lock_path[] = "/records/index";
    const uint8_t embedded_nul[] = { 'a', 0, 'b' };
    const uint8_t alice[] = "alice";
    const uint8_t users[] = "users";
    const char expected[] = "/.ghostos/data/users/616c696365";
    assert(ghostos_rms_lock_path((const uint8_t *)"", 0) == 1);
    assert(ghostos_rms_lock_path(embedded_nul, 3) == 1);
    assert(ghostos_rms_lock_path(lock_path, sizeof lock_path - 1) == 0);
    assert(ghostos_rms_resource_id(lock_path, sizeof lock_path - 1) == UINT64_C(0xc0b2aea3680f9b4b));
    assert(ghostos_rms_resource_id(NULL, 0) == UINT64_C(0xcbf29ce484222325));
    assert(ghostos_rms_namespace(users, 5) == 0);
    assert(ghostos_rms_namespace((const uint8_t *)"a-b_c.d", 7) == 0);
    assert(ghostos_rms_namespace((const uint8_t *)"", 0) == 1);
    assert(ghostos_rms_namespace((const uint8_t *)"users/x", 7) == 1);
    assert(ghostos_rms_key_path(users, 5, alice, 5, path, sizeof path, &written) == 0);
    assert(written == sizeof expected - 1);
    assert(memcmp(path, expected, written) == 0);
    assert(ghostos_rms_key_path(users, 5, alice, 0, path, sizeof path, &written) == 1);
    assert(ghostos_rms_key_path(users, 5, alice, 5, path, 4, &written) == 2);
    assert(ghostos_rms_reserve(0, 2, &index) == 0 && index == 0);
    assert(ghostos_rms_reserve(2, 2, &index) == 1);
    assert(ghostos_rms_add_count(2, &count) && count == 3);
    assert(!ghostos_rms_add_count(UINT32_MAX, &count));
    assert(ghostos_rms_sub_count(1, &count) && count == 0);
    assert(!ghostos_rms_sub_count(0, &count));
}
int main(void) {
    paths_namespaces_and_counts_follow_existing_rules();
    return 0;
}
