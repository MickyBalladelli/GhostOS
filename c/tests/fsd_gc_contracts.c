#include "ghostos/fsd_gc.h"
#include <assert.h>
#include <string.h>
static void collection_caps_the_report_and_keeps_checkpoint_blocks(void) {
    ghostos_fsd_gc_process processes[1];
    ghostos_fsd_block blocks[4];
    uint64_t authority = (1ull << 32) | 1;
    size_t live = 0, freed = 9;
    memset(processes, 0, sizeof processes);
    memset(blocks, 0, sizeof blocks);
    assert(ghostos_fsd_collect(processes, 1, 1, authority, blocks, 4, 1, 4, 1, &live, &freed) == 1);
    processes[0].occupied = true;
    processes[0].process = 1;
    processes[0].generation = 1;
    processes[0].rights = 1;
    assert(ghostos_fsd_collect(processes, 1, 1, authority, blocks, 4, 1, 4, 1, &live, &freed) == 3);
    processes[0].rights = 8;
    blocks[0].occupied = true;
    blocks[0].child = 2;
    blocks[1].occupied = true;
    blocks[2].occupied = true;
    blocks[3].occupied = true;
    assert(!ghostos_fsd_collect(processes, 1, 1, authority, blocks, 4, 1, 4, 1, &live, &freed));
    assert(live == 3 && freed == 1 && !blocks[2].occupied && blocks[3].occupied);
    blocks[2].occupied = true;
    assert(!ghostos_fsd_collect(processes, 1, 1, authority, blocks, 4, 1, 4, 0, &live, &freed));
    assert(live == 3 && freed == 0 && !blocks[2].occupied);
}
int main(void) {
    collection_caps_the_report_and_keeps_checkpoint_blocks();
    return 0;
}
