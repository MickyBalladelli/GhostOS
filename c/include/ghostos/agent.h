#ifndef GHOSTOS_AGENT_H
#define GHOSTOS_AGENT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Scope and parent checks: 0 accepted, 1 rejected.
 * Slot: 0 stored index, 1 capacity. Consume: 0 allowed, 1 empty rights,
 * 2 denied. Finish: 0 discard, 1 commit, 2 commit not authorized. */
int ghostos_agent_scope(uint64_t resource, uint16_t rights, uint8_t transports, uint64_t lifetime_us);
int ghostos_agent_parent(uint32_t parent_issuer, uint32_t issuer, uint64_t parent_resource,
    uint64_t resource, uint64_t parent_epoch, uint64_t epoch);
int ghostos_agent_covers(uint16_t have, uint16_t need);
int ghostos_agent_transport_lifetime(uint8_t have, uint8_t need, uint64_t lifetime_us, uint64_t max_lifetime_us);
bool ghostos_agent_add_time(uint64_t now_us, uint64_t lifetime_us, uint64_t *expires_at_us);
bool ghostos_agent_within_expiry(uint64_t expires_at_us, uint64_t parent_expiry_us);
typedef struct { uint64_t expires_at_us; bool occupied; } ghostos_agent_grant;
int ghostos_agent_slot(const ghostos_agent_grant *grants, size_t count, uint64_t now_us, size_t *index);
uint64_t ghostos_agent_next_id(uint64_t value);
int ghostos_agent_required(uint16_t required);
int ghostos_agent_grant_access(uint64_t now_us, uint64_t expires_at_us, uint16_t rights,
    uint16_t required, uint8_t transports, uint8_t transport);
uint16_t ghostos_agent_run_rights(uint16_t task_rights, bool commit);
size_t ghostos_agent_active(const ghostos_agent_grant *grants, size_t count, uint64_t now_us);
int ghostos_agent_finish(bool commit, bool authorized, bool success);
#endif
