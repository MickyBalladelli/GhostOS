#include "ghostos/actors.h"
enum { GHOSTOS_ACTOR_PAGE = 4096 };
_Static_assert(sizeof(ghostos_actor_entry) == 24, "actor entry ABI");
_Static_assert(offsetof(ghostos_actor_entry, kind) == 20, "actor kind ABI");
_Static_assert(sizeof(ghostos_actor_route) == 56, "actor route ABI");
_Static_assert(offsetof(ghostos_actor_route, write) == 52, "actor route flag ABI");
static int range_end(uint64_t start, uint64_t length, bool checked, uint64_t *end) {
    if (length > UINT64_MAX - start) {
        if (checked) return 9;
        *end = start + length;
        return 0;
    }
    *end = start + length;
    return 0;
}
int ghostos_actor_validate_mailbox(uint32_t destination, uint64_t range_start,
    uint64_t range_length, uint32_t subject, uint64_t auth_start, uint64_t auth_length,
    bool write, uint32_t lease_epoch, uint64_t expires_at_us, uint32_t local,
    uint64_t now_us, bool checked) {
    uint64_t end = 0;
    uint64_t auth_end = 0;
    int status = range_end(range_start, range_length, checked, &end);
    if (status) return status;
    if (!end) return 6;
    if (destination == local || range_start % GHOSTOS_ACTOR_PAGE ||
        range_length % GHOSTOS_ACTOR_PAGE || subject != local || !write) return 6;
    status = range_end(auth_start, auth_length, checked, &auth_end);
    if (status) return status;
    if (range_start < auth_start || range_start >= auth_end) return 6;
    if (end - 1 < auth_start || end - 1 >= auth_end) return 6;
    if (!lease_epoch || now_us >= expires_at_us) return 6;
    return 0;
}
int ghostos_actor_decode_words(uint64_t source_node, uint64_t source_local,
    uint64_t destination_node, uint64_t destination_local) {
    if (!(uint32_t)source_node || !(uint32_t)destination_node) return 3;
    if (source_node > UINT32_MAX || destination_node > UINT32_MAX ||
        !source_local || !destination_local) return 3;
    return 0;
}
bool ghostos_actor_endpoint_matches(uint32_t actor_node, uint8_t kind,
    uint32_t endpoint, uint32_t local_node) {
    if (kind == 1) return actor_node == local_node;
    if (kind == 2) return actor_node != local_node && endpoint == actor_node;
    return false;
}
int ghostos_actor_register(const ghostos_actor_entry *entries, size_t count,
    uint32_t node, uint64_t local_id, uint8_t kind, uint32_t endpoint,
    uint32_t generation, uint32_t local_node, size_t *index) {
    size_t free_slot = count;
    size_t i;
    if (!generation || !ghostos_actor_endpoint_matches(node, kind, endpoint, local_node))
        return 5;
    for (i = 0; i < count; ++i) {
        if (!entries[i].kind) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (entries[i].node == node && entries[i].local == local_id) return 1;
    }
    if (free_slot == count) return 2;
    *index = free_slot;
    return 0;
}
int ghostos_actor_find(const ghostos_actor_entry *entries, size_t count,
    uint32_t node, uint64_t local_id, size_t *index) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (entries[i].kind && entries[i].node == node && entries[i].local == local_id) {
            *index = i;
            return 0;
        }
    }
    return 8;
}
int ghostos_actor_add_route(const ghostos_actor_route *routes, size_t count,
    uint32_t destination, uint64_t range_start, uint64_t range_length, uint32_t subject,
    uint64_t auth_start, uint64_t auth_length, bool write, uint32_t lease_epoch,
    uint64_t expires_at_us, uint32_t local, uint64_t now_us, bool checked, size_t *index) {
    size_t free_slot = count;
    size_t i;
    int status = ghostos_actor_validate_mailbox(destination, range_start, range_length,
        subject, auth_start, auth_length, write, lease_epoch, expires_at_us, local,
        now_us, checked);
    if (status) return status;
    for (i = 0; i < count; ++i) {
        if (!routes[i].occupied) {
            if (free_slot == count) free_slot = i;
            continue;
        }
        if (routes[i].node == destination) return 1;
    }
    if (free_slot == count) return 2;
    *index = free_slot;
    return 0;
}
int ghostos_actor_prepare(const ghostos_actor_entry *entries, size_t actor_count,
    const ghostos_actor_route *routes, size_t route_count, uint32_t actor_node,
    uint64_t actor_local, uint64_t image_high, uint64_t image_low, uint32_t generation,
    uint32_t local_node, size_t *directory, size_t *route, bool *local_spawn) {
    size_t free_slot = actor_count;
    size_t i;
    if (!actor_local || (!image_high && !image_low) || !generation) return 4;
    for (i = 0; i < actor_count; ++i) {
        if (!entries[i].kind) {
            if (free_slot == actor_count) free_slot = i;
            continue;
        }
        if (entries[i].node == actor_node && entries[i].local == actor_local) return 1;
    }
    if (free_slot == actor_count) return 2;
    if (actor_node == local_node) {
        *directory = free_slot;
        *local_spawn = true;
        return 0;
    }
    for (i = 0; i < route_count; ++i) {
        if (routes[i].occupied && routes[i].node == actor_node) {
            *directory = free_slot;
            *route = i;
            *local_spawn = false;
            return 0;
        }
    }
    return 7;
}
