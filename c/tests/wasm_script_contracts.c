#include "ghostos/wasm_script.h"
#include <assert.h>
static void grants_reject_duplicates_before_host_calls(void) {
    uint64_t handles[] = {1, 1};
    uint64_t operations[] = {1, 1};
    uint64_t stored_handles[2] = {1, 0};
    uint64_t stored_operations[2] = {1, 0};
    bool occupied[2] = {true, false};
    uint64_t found = 0;
    assert(!ghostos_wasm_limits(0, 1, 1, 1, 1, 1, 1));
    assert(ghostos_wasm_limits(1, 1, 1, 1, 1, 1, 1));
    assert(ghostos_wasm_module(4, 8, 0) == 1);
    assert(ghostos_wasm_module(9, 8, 3) == 2);
    assert(ghostos_wasm_module(8, 8, 3) == 0);
    assert(ghostos_wasm_grants(handles, operations, 3, 2) == 1);
    assert(ghostos_wasm_grants(handles, operations, 1, 2) == 0);
    handles[1] = 0;
    assert(ghostos_wasm_grants(handles, operations, 2, 2) == 2);
    handles[1] = 1;
    operations[1] = 2;
    assert(ghostos_wasm_grants(handles, operations, 2, 2) == 3);
    assert(ghostos_wasm_allows(1, 0));
    assert(!ghostos_wasm_allows(1, 1));
    assert(ghostos_wasm_find(stored_handles, stored_operations, occupied, 2, 1, &found) && found == 1);
    assert(!ghostos_wasm_find(stored_handles, stored_operations, occupied, 2, 9, &found));
    assert(ghostos_wasm_check(false, true) == 0);
    assert(ghostos_wasm_check(true, true) == 1);
    assert(ghostos_wasm_invoke(false, true, true) == 1);
    assert(ghostos_wasm_invoke(true, false, false) == 2);
    assert(ghostos_wasm_invoke(true, true, true) == 0);
    assert(ghostos_wasm_fuel_consumed(5, 8) == 0);
    assert(ghostos_wasm_fuel_consumed(8, 3) == 5);
}
int main(void) {
    grants_reject_duplicates_before_host_calls();
    return 0;
}
