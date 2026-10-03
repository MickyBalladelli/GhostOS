#include "ghostos/platform_io.h"
#include <assert.h>

/* Ports of all four existing platform-io queue model cases. Payload values
 * remain in caller-owned storage, matching the generic Rust adapter. */
static void queue_saturation_keeps_every_submitted_slot_bounded(void)
{
    ghostos_io_slot slots[2] = {{0}};
    ghostos_io_cursors cursors = {0};
    size_t index;
    uint64_t first, second, token;
    assert(ghostos_io_submit(slots, 2, &cursors, &index, &first) == GHOSTOS_IO_OK);
    assert(ghostos_io_submit(slots, 2, &cursors, &index, &second) == GHOSTOS_IO_OK);
    assert(ghostos_io_pending(slots, 2) == 2);
    assert(ghostos_io_submit(slots, 2, &cursors, &index, &token) == GHOSTOS_IO_QUEUE_FULL);
    assert(ghostos_io_dispatch(slots, 2, &cursors, &index, &token));
    assert(ghostos_io_complete(slots, 2, token) == GHOSTOS_IO_OK);
    assert(ghostos_io_pending(slots, 2) == 2);
    assert(ghostos_io_submit(slots, 2, &cursors, &index, &token) == GHOSTOS_IO_QUEUE_FULL);
    assert(ghostos_io_poll(slots, 2, &cursors, &index, &token));
    assert(token == first);
    assert(ghostos_io_pending(slots, 2) == 1);
    assert(ghostos_io_submit(slots, 2, &cursors, &index, &token) == GHOSTOS_IO_OK);
    assert(first != second);
}

static void duplicate_completion_is_rejected_and_published_once(void)
{
    ghostos_io_slot slots[1] = {{0}};
    ghostos_io_cursors cursors = {0};
    size_t index;
    uint64_t token;
    uint8_t result = 0;
    assert(ghostos_io_submit(slots, 1, &cursors, &index, &token) == GHOSTOS_IO_OK);
    assert(ghostos_io_dispatch(slots, 1, &cursors, &index, &token));
    assert(ghostos_io_complete(slots, 1, token) == GHOSTOS_IO_OK);
    result = 8;
    assert(ghostos_io_complete(slots, 1, token) == GHOSTOS_IO_REQUEST_NOT_DISPATCHED);
    assert(ghostos_io_poll(slots, 1, &cursors, &index, &token));
    assert(result == 8);
    assert(!ghostos_io_poll(slots, 1, &cursors, &index, &token));
}

static void cancellation_races_follow_the_request_state(void)
{
    ghostos_io_slot slots[1] = {{0}};
    ghostos_io_cursors cursors = {0};
    size_t index;
    uint64_t token;
    assert(ghostos_io_submit(slots, 1, &cursors, &index, &token) == GHOSTOS_IO_OK);
    assert(ghostos_io_cancel(slots, 1, token) == GHOSTOS_IO_OK);
    assert(ghostos_io_cancel(slots, 1, token) == GHOSTOS_IO_INVALID_TOKEN);
    assert(!ghostos_io_dispatch(slots, 1, &cursors, &index, &token));
    assert(ghostos_io_submit(slots, 1, &cursors, &index, &token) == GHOSTOS_IO_OK);
    assert(ghostos_io_dispatch(slots, 1, &cursors, &index, &token));
    assert(ghostos_io_cancel(slots, 1, token) == GHOSTOS_IO_CANNOT_CANCEL);
    assert(ghostos_io_complete(slots, 1, token) == GHOSTOS_IO_OK);
    assert(ghostos_io_cancel(slots, 1, token) == GHOSTOS_IO_CANNOT_CANCEL);
    assert(ghostos_io_poll(slots, 1, &cursors, &index, &token));
}

static void stale_generation_cannot_complete_a_reused_slot(void)
{
    ghostos_io_slot slots[1] = {{0}};
    ghostos_io_cursors cursors = {0};
    size_t index;
    uint64_t stale, current, token;
    assert(ghostos_io_submit(slots, 1, &cursors, &index, &stale) == GHOSTOS_IO_OK);
    assert(ghostos_io_cancel(slots, 1, stale) == GHOSTOS_IO_OK);
    assert(ghostos_io_submit(slots, 1, &cursors, &index, &current) == GHOSTOS_IO_OK);
    assert(stale != current);
    assert(ghostos_io_complete(slots, 1, stale) == GHOSTOS_IO_INVALID_TOKEN);
    assert(ghostos_io_dispatch(slots, 1, &cursors, &index, &token));
    assert(token == current);
    assert(ghostos_io_complete(slots, 1, current) == GHOSTOS_IO_OK);
    assert(ghostos_io_poll(slots, 1, &cursors, &index, &token));
}

int main(void)
{
    queue_saturation_keeps_every_submitted_slot_bounded();
    duplicate_completion_is_rejected_and_published_once();
    cancellation_races_follow_the_request_state();
    stale_generation_cannot_complete_a_reused_slot();
    return 0;
}
