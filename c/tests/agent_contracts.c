#include "ghostos/agent.h"
#include <assert.h>
static void grants_reuse_expired_slots_and_keep_parent_limits(void) {
    ghostos_agent_grant grants[2] = {{5, true}, {40, true}};
    size_t index = 9;
    uint64_t expires = 0;
    assert(ghostos_agent_scope(0, 4, 2, 30) == 1);
    assert(ghostos_agent_scope(77, 4, 2, 30) == 0);
    assert(ghostos_agent_parent(1, 2, 77, 77, 1, 1) == 1);
    assert(ghostos_agent_covers(1, 2) == 1);
    assert(ghostos_agent_transport_lifetime(2, 2, 0, 30) == 1);
    assert(ghostos_agent_transport_lifetime(2, 2, 40, 30) == 1);
    assert(!ghostos_agent_add_time(UINT64_MAX, 1, &expires));
    assert(ghostos_agent_add_time(10, 20, &expires) && expires == 30);
    assert(!ghostos_agent_within_expiry(30, 20));
    assert(ghostos_agent_slot(grants, 2, 5, &index) == 0 && index == 0);
    assert(ghostos_agent_slot(grants, 2, 4, &index) == 1);
    assert(ghostos_agent_next_id(UINT64_MAX) == 1);
    assert(ghostos_agent_required(0) == 1);
    assert(ghostos_agent_grant_access(30, 30, 4, 4, 2, 2) == 2);
    assert(ghostos_agent_grant_access(11, 30, 4, 4, 2, 2) == 0);
    assert(ghostos_agent_run_rights(4, true) == 6);
    assert(ghostos_agent_run_rights(4, false) == 4);
    assert(ghostos_agent_active(grants, 2, 5) == 1);
    assert(ghostos_agent_finish(true, false, true) == 2);
    assert(ghostos_agent_finish(true, true, false) == 0);
    assert(ghostos_agent_finish(true, true, true) == 1);
}
int main(void) {
    grants_reuse_expired_slots_and_keep_parent_limits();
    return 0;
}
