#ifndef GHOSTOS_ACTORS_H
#define GHOSTOS_ACTORS_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Kind: empty=0, local=1, remote=2.
 * Result: success=0, already registered=1, capacity=2, corrupt envelope=3,
 * invalid actor=4, invalid endpoint=5, invalid mailbox=6, node route missing=7,
 * not found=8, checked address overflow=9. */
typedef struct {
    uint64_t local;
    uint32_t node;
    uint32_t endpoint;
    uint32_t generation;
    uint8_t kind;
} ghostos_actor_entry;
typedef struct {
    uint64_t range_start, range_length, auth_start, auth_length, expires_at_us;
    uint32_t node, subject, lease_epoch;
    bool write, occupied;
} ghostos_actor_route;
int ghostos_actor_validate_mailbox(uint32_t destination, uint64_t range_start,
    uint64_t range_length, uint32_t subject, uint64_t auth_start, uint64_t auth_length,
    bool write, uint32_t lease_epoch, uint64_t expires_at_us, uint32_t local,
    uint64_t now_us, bool checked);
int ghostos_actor_decode_words(uint64_t source_node, uint64_t source_local,
    uint64_t destination_node, uint64_t destination_local);
bool ghostos_actor_endpoint_matches(uint32_t actor_node, uint8_t kind,
    uint32_t endpoint, uint32_t local_node);
int ghostos_actor_register(const ghostos_actor_entry *entries, size_t count,
    uint32_t node, uint64_t local_id, uint8_t kind, uint32_t endpoint,
    uint32_t generation, uint32_t local_node, size_t *index);
int ghostos_actor_find(const ghostos_actor_entry *entries, size_t count,
    uint32_t node, uint64_t local_id, size_t *index);
int ghostos_actor_add_route(const ghostos_actor_route *routes, size_t count,
    uint32_t destination, uint64_t range_start, uint64_t range_length, uint32_t subject,
    uint64_t auth_start, uint64_t auth_length, bool write, uint32_t lease_epoch,
    uint64_t expires_at_us, uint32_t local, uint64_t now_us, bool checked, size_t *index);
int ghostos_actor_prepare(const ghostos_actor_entry *entries, size_t actor_count,
    const ghostos_actor_route *routes, size_t route_count, uint32_t actor_node,
    uint64_t actor_local, uint64_t image_high, uint64_t image_low, uint32_t generation,
    uint32_t local_node, size_t *directory, size_t *route, bool *local_spawn);
#endif
