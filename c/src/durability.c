#include "ghostos/durability.h"

_Static_assert(sizeof(ghostos_durability_event) == 16, "event size");
_Static_assert(offsetof(ghostos_durability_event, kind) == 8, "event kind offset");
_Static_assert(offsetof(ghostos_durability_event, reserved) == 12, "event reserved offset");

const ghostos_durability_layer_contract ghostos_durability_contract[6] = {
    {0, 0, 0, 2, 0, 0, 3},
    {1, 0, 1, 2, 1, 1, 3},
    {2, 0, 1, 2, 0, 1, 3},
    {3, 0, 2, 2, 0, 1, 3},
    {4, 0, 2, 2, 0, 1, 3},
    {5, 3, 3, 3, 3, 3, 3}
};

bool ghostos_no_interruption(void *context, uint32_t domain, uint32_t boundary)
{
    (void)context;
    (void)domain;
    (void)boundary;
    return false;
}

uint32_t ghostos_durability_record(ghostos_durability_event *events,
    size_t capacity, size_t *length, ghostos_durability_event event)
{
    if (*length >= capacity) return GHOSTOS_TRACE_FULL;
    events[*length] = event;
    ++*length;
    return GHOSTOS_DURABILITY_OK;
}

static size_t find_before(const ghostos_durability_event *events, size_t end,
    uint64_t transaction, uint32_t kind)
{
    while (end != 0) {
        --end;
        if (events[end].kind == kind && events[end].transaction == transaction)
            return end;
    }
    return SIZE_MAX;
}

uint32_t ghostos_durability_verify(const ghostos_durability_event *events,
    size_t length, uint64_t *failed_transaction)
{
    const uint32_t required[] = {GHOSTOS_APPLICATION_WRITE, GHOSTOS_SYNFS_WRITE,
        GHOSTOS_COMMIT, GHOSTOS_BLOCK_DATA_WRITE, GHOSTOS_BLOCK_COMMIT_RECORD,
        GHOSTOS_BLOCK_FLUSH};
    for (size_t index = 0; index < length; ++index) {
        if (events[index].kind != GHOSTOS_SYNC_ACKNOWLEDGED) continue;
        uint64_t transaction = events[index].transaction;
        size_t steps[6];
        for (size_t step = 0; step < 6; ++step) {
            steps[step] = find_before(events, index, transaction, required[step]);
            if (steps[step] == SIZE_MAX) {
                *failed_transaction = transaction;
                return GHOSTOS_MISSING_STEP;
            }
        }
        bool invalid = false;
        for (size_t step = 1; step < 6; ++step)
            if (steps[step - 1] >= steps[step]) invalid = true;
        size_t storage = find_before(events, index, transaction, GHOSTOS_STORAGE_DAEMON_WRITE);
        size_t cache = find_before(events, index, transaction, GHOSTOS_CACHE_FLUSH);
        size_t rename = find_before(events, index, transaction, GHOSTOS_RENAME);
        if (rename != SIZE_MAX && rename > steps[2]) invalid = true;
        if (storage != SIZE_MAX && (storage < steps[2] || storage > steps[3])) invalid = true;
        if (cache != SIZE_MAX && (storage == SIZE_MAX || cache < storage || cache > steps[3])) invalid = true;
        for (size_t after = steps[4] + 1; after < index; ++after)
            if (events[after].kind == GHOSTOS_BLOCK_DATA_WRITE &&
                events[after].transaction == transaction) invalid = true;
        if (invalid) {
            *failed_transaction = transaction;
            return GHOSTOS_INVALID_ORDER;
        }
    }
    size_t power_loss = SIZE_MAX;
    for (size_t index = 0; index < length; ++index) {
        if (events[index].kind == GHOSTOS_POWER_LOSS) power_loss = index;
        else if (events[index].kind == GHOSTOS_RECOVERED) {
            uint64_t transaction = events[index].transaction;
            if (power_loss == SIZE_MAX || find_before(events, power_loss,
                transaction, GHOSTOS_SYNC_ACKNOWLEDGED) == SIZE_MAX) {
                *failed_transaction = transaction;
                return GHOSTOS_RECOVERED_VOLATILE;
            }
        }
    }
    return GHOSTOS_DURABILITY_OK;
}
