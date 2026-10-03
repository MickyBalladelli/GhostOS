#include "ghostos/agent.h"
enum { GHOSTOS_AGENT_EXECUTE = 1u << 2, GHOSTOS_AGENT_WRITE = 1u << 1 };
_Static_assert(sizeof(ghostos_agent_grant) == 16, "agent grant ABI");
_Static_assert(offsetof(ghostos_agent_grant, occupied) == 8, "agent occupancy ABI");
int ghostos_agent_scope(uint64_t resource, uint16_t rights, uint8_t transports, uint64_t lifetime_us) {
    return !resource || !rights || !transports || !lifetime_us ? 1 : 0;
}
int ghostos_agent_parent(uint32_t parent_issuer, uint32_t issuer, uint64_t parent_resource,
    uint64_t resource, uint64_t parent_epoch, uint64_t epoch) {
    return parent_issuer != issuer || parent_resource != resource || parent_epoch != epoch ? 1 : 0;
}
int ghostos_agent_covers(uint16_t have, uint16_t need) {
    return (have & need) == need ? 0 : 1;
}
int ghostos_agent_transport_lifetime(uint8_t have, uint8_t need, uint64_t lifetime_us, uint64_t max_lifetime_us) {
    return (have & need) != need || !lifetime_us || lifetime_us > max_lifetime_us ? 1 : 0;
}
bool ghostos_agent_add_time(uint64_t now_us, uint64_t lifetime_us, uint64_t *expires_at_us) {
    if (lifetime_us > UINT64_MAX - now_us) return false;
    *expires_at_us = now_us + lifetime_us;
    return true;
}
bool ghostos_agent_within_expiry(uint64_t expires_at_us, uint64_t parent_expiry_us) {
    return expires_at_us <= parent_expiry_us;
}
int ghostos_agent_slot(const ghostos_agent_grant *grants, size_t count, uint64_t now_us, size_t *index) {
    size_t i;
    for (i = 0; i < count; ++i) {
        if (!grants[i].occupied || grants[i].expires_at_us <= now_us) {
            *index = i;
            return 0;
        }
    }
    return 1;
}
uint64_t ghostos_agent_next_id(uint64_t value) {
    uint64_t next = value + 1;
    return next < 1 ? 1 : next;
}
int ghostos_agent_required(uint16_t required) {
    return required ? 0 : 1;
}
int ghostos_agent_grant_access(uint64_t now_us, uint64_t expires_at_us, uint16_t rights,
    uint16_t required, uint8_t transports, uint8_t transport) {
    if (now_us >= expires_at_us || (rights & required) != required || (transports & transport) != transport)
        return 2;
    return 0;
}
uint16_t ghostos_agent_run_rights(uint16_t task_rights, bool commit) {
    uint16_t rights = (uint16_t)(task_rights | GHOSTOS_AGENT_EXECUTE);
    if (commit) rights = (uint16_t)(rights | GHOSTOS_AGENT_WRITE);
    return rights;
}
size_t ghostos_agent_active(const ghostos_agent_grant *grants, size_t count, uint64_t now_us) {
    size_t active = 0;
    size_t i;
    for (i = 0; i < count; ++i)
        if (grants[i].occupied && grants[i].expires_at_us > now_us) active++;
    return active;
}
int ghostos_agent_finish(bool commit, bool authorized, bool success) {
    if (commit && !authorized) return 2;
    return commit && success ? 1 : 0;
}
