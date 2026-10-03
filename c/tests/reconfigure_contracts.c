#include "ghostos/reconfigure.h"
#include <assert.h>
static void activation_keeps_the_previous_revision_until_commit(void) {
    ghostos_reconfigure_entry history[2] = {0};
    size_t length = 0;
    size_t index = 9;
    uint64_t active = 0;
    bool has_active = false;
    uint64_t revision = 0;
    uint64_t previous = 0;
    uint64_t generation = 0;
    assert(!ghostos_reconfigure_prepare(length, 2, false, 0, false, 0, 1));
    assert(!ghostos_reconfigure_commit(history, 2, &length, &active, &has_active, 1, 10, 9));
    assert(has_active && active == 1 && length == 1 && history[0].has_previous == false);
    assert(ghostos_reconfigure_prepare(length, 2, true, active, false, 0, 1) == 3);
    assert(!ghostos_reconfigure_prepare(length, 2, true, active, true, 1, 2));
    assert(!ghostos_reconfigure_commit(history, 2, &length, &active, &has_active, 2, 11, 10));
    assert(history[1].has_previous && history[1].previous_revision == 1);
    assert(!ghostos_reconfigure_rollback(history, length, &index, &revision, &previous, &generation));
    assert(index == 1 && revision == 1 && previous == 2 && generation == 11);
    length = 1;
    active = 1;
    assert(ghostos_reconfigure_prepare(length, 2, true, active, true, 9, 3) == 2);
    assert(ghostos_reconfigure_prepare(2, 2, true, 2, true, 1, 3) == 1);
    assert(ghostos_reconfigure_prepare(0, 0, false, 0, false, 0, 1) == 1);
}
int main(void) {
    activation_keeps_the_previous_revision_until_commit();
    return 0;
}
