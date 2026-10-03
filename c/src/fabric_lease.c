#include "ghostos/fabric_lease.h"
enum { GHOSTOS_FABRIC_PAGE = 4096 };
static bool power_of_two(uint64_t value) { return value && (value & (value - 1)) == 0; }
static int align_up(uint64_t value, uint64_t alignment, uint64_t *aligned) {
    if (!alignment || value > UINT64_MAX - (alignment - 1)) return 4;
    *aligned = (value + alignment - 1) & ~(alignment - 1);
    return 0;
}
static bool overlaps(uint64_t start, uint64_t length, uint64_t other_start, uint64_t other_length) {
    return start < other_start + other_length && other_start < start + length;
}
static size_t expire(ghostos_fabric_lease *leases, size_t capacity, uint64_t now_us) {
    size_t i, expired = 0;
    for (i = 0; i < capacity; ++i) {
        if (leases[i].occupied && now_us >= leases[i].expires_at_us) {
            leases[i].occupied = false;
            expired += 1;
        }
    }
    return expired;
}
static int valid_lease(const ghostos_fabric_lease *leases, size_t capacity, uint64_t handle, size_t *slot) {
    uint32_t generation = (uint32_t)(handle >> 32);
    size_t index = (uint32_t)handle;
    if (index >= capacity || !generation || !leases[index].occupied || leases[index].generation != generation) return 8;
    *slot = index;
    return 0;
}
int ghostos_fabric_lease_allocate(ghostos_fabric_lease *leases, size_t capacity, bool pool_present, bool draining,
    uint32_t pool, uint64_t pool_start, uint64_t pool_length, uint64_t owner, uint64_t length, uint64_t alignment,
    uint8_t rights, uint64_t now_us, uint64_t duration_us, uint64_t *handle, uint64_t *start) {
    uint64_t cursor, pool_end;
    int status;
    expire(leases, capacity, now_us);
    if (!pool_present) return 1;
    if (draining) return 2;
    if (!length || alignment < GHOSTOS_FABRIC_PAGE || !power_of_two(alignment) || length % GHOSTOS_FABRIC_PAGE != 0 || !duration_us)
        return 3;
    status = align_up(pool_start, alignment, &cursor);
    if (status) return status;
    if (pool_start > UINT64_MAX - pool_length) return 4;
    pool_end = pool_start + pool_length;
    for (;;) {
        uint64_t end, conflict_end = 0;
        size_t i, slot = 0;
        bool conflict = false, found = false;
        uint32_t generation;
        if (cursor > UINT64_MAX - length) return 4;
        end = cursor + length;
        if (end > pool_end) return 5;
        for (i = 0; i < capacity; ++i) {
            uint64_t lease_end;
            if (!leases[i].occupied || leases[i].pool != pool) continue;
            if (leases[i].start > UINT64_MAX - leases[i].length) return 4;
            lease_end = leases[i].start + leases[i].length;
            if (!overlaps(cursor, length, leases[i].start, leases[i].length)) continue;
            if (!conflict || lease_end >= conflict_end) { conflict_end = lease_end; conflict = true; }
        }
        if (conflict) {
            uint64_t next;
            status = align_up(conflict_end, alignment, &next);
            if (status) return status;
            if (next <= cursor) return 5;
            cursor = next;
            continue;
        }
        for (i = 0; i < capacity; ++i) if (!leases[i].occupied) { slot = i; found = true; break; }
        if (!found) return 5;
        generation = leases[slot].generation + 1;
        if (!generation) generation = 1;
        leases[slot].occupied = true;
        leases[slot].generation = generation;
        leases[slot].pool = pool;
        leases[slot].owner = owner;
        leases[slot].start = cursor;
        leases[slot].length = length;
        leases[slot].rights = rights;
        leases[slot].expires_at_us = now_us > UINT64_MAX - duration_us ? UINT64_MAX : now_us + duration_us;
        *handle = ((uint64_t)generation << 32) | slot;
        *start = cursor;
        return 0;
    }
}
int ghostos_fabric_lease_largest_free(ghostos_fabric_lease *leases, size_t capacity, bool pool_present, uint32_t pool,
    uint64_t pool_start, uint64_t pool_length, uint64_t alignment, uint64_t now_us, uint64_t *start, uint64_t *length) {
    uint64_t cursor, pool_end, best_start = 0, best_length = 0;
    int status;
    expire(leases, capacity, now_us);
    if (!pool_present) return 1;
    if (alignment < GHOSTOS_FABRIC_PAGE || !power_of_two(alignment)) return 3;
    status = align_up(pool_start, alignment, &cursor);
    if (status) return status;
    if (pool_start > UINT64_MAX - pool_length) return 4;
    pool_end = pool_start + pool_length;
    while (cursor < pool_end) {
        size_t i;
        bool found = false;
        uint64_t next_start = 0, next_end = 0, gap_end, gap;
        for (i = 0; i < capacity; ++i) {
            uint64_t lease_end;
            if (!leases[i].occupied || leases[i].pool != pool) continue;
            if (leases[i].start > UINT64_MAX - leases[i].length) return 4;
            lease_end = leases[i].start + leases[i].length;
            if (lease_end <= cursor) continue;
            if (!found || leases[i].start < next_start) {
                next_start = leases[i].start;
                next_end = lease_end;
                found = true;
            }
        }
        gap_end = found ? next_start : pool_end;
        if (gap_end > cursor) {
            gap = (gap_end - cursor) / GHOSTOS_FABRIC_PAGE * GHOSTOS_FABRIC_PAGE;
            if (gap > best_length) { best_start = cursor; best_length = gap; }
        }
        if (!found) break;
        status = align_up(cursor > next_end ? cursor : next_end, alignment, &cursor);
        if (status) return status;
    }
    if (!best_length) return 5;
    *start = best_start;
    *length = best_length;
    return 0;
}
int ghostos_fabric_lease_authorize(const ghostos_fabric_lease *leases, size_t capacity, uint64_t handle, uint64_t owner,
    uint64_t address, uint8_t access, uint64_t now_us) {
    size_t slot = 0;
    bool permitted;
    int status = valid_lease(leases, capacity, handle, &slot);
    if (status) return status;
    if (leases[slot].owner != owner) return 6;
    if (now_us >= leases[slot].expires_at_us) return 7;
    if (access == 0) permitted = (leases[slot].rights & 1) != 0;
    else if (access == 1) permitted = (leases[slot].rights & 2) != 0;
    else permitted = false;
    if (address < leases[slot].start || address >= leases[slot].start + leases[slot].length || !permitted) return 6;
    return 0;
}
