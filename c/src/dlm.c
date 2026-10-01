#include "ghostos/dlm.h"

static ghostos_dlm kernel_dlm;
static ghostos_dlm_node_fence_table kernel_node_fences;
static bool kernel_dlm_ready;

void ghostos_dlm_kernel_init(void)
{
    if (kernel_dlm_ready) return;
    (void)ghostos_dlm_init(&kernel_dlm, GHOSTOS_DLM_MAX_LOCKS);
    (void)ghostos_dlm_node_fence_init(&kernel_node_fences, GHOSTOS_DLM_MAX_NODES);
    kernel_dlm_ready = true;
}

bool ghostos_dlm_kernel_lock_summary(size_t index, uint64_t *resource,
                                     uint32_t *owner_node, bool *granted)
{
    ghostos_dlm_kernel_init();
    const ghostos_dlm_lock_entry *entry = ghostos_dlm_lock(&kernel_dlm, index);
    if (!entry || !entry->occupied) return false;
    if (resource) *resource = entry->resource;
    if (owner_node) *owner_node = entry->owner.node;
    if (granted) *granted = entry->granted;
    return true;
}

void ghostos_dlm_kernel_counters(uint64_t *acquisitions, uint64_t *queued,
    uint64_t *promotions, uint64_t *releases, uint64_t *expirations,
    uint64_t wait_histogram[GHOSTOS_LOCK_DURATION_BUCKETS],
    uint64_t hold_histogram[GHOSTOS_LOCK_DURATION_BUCKETS],
    uint64_t *max_wait, uint64_t *max_hold, size_t *active, uint64_t now_us)
{
    ghostos_dlm_kernel_init();
    ghostos_dlm_contention_report report = ghostos_dlm_contention(&kernel_dlm, now_us);
    if (acquisitions) *acquisitions = report.acquisitions;
    if (queued) *queued = report.queued_acquisitions;
    if (promotions) *promotions = report.promotions;
    if (releases) *releases = report.releases;
    if (expirations) *expirations = report.expirations;
    if (max_wait) *max_wait = report.max_wait_duration;
    if (max_hold) *max_hold = report.max_hold_duration;
    if (active) *active = report.active_locks;
    if (wait_histogram) for (size_t i = 0; i < GHOSTOS_LOCK_DURATION_BUCKETS; ++i)
        wait_histogram[i] = report.wait_duration_histogram[i];
    if (hold_histogram) for (size_t i = 0; i < GHOSTOS_LOCK_DURATION_BUCKETS; ++i)
        hold_histogram[i] = report.hold_duration_histogram[i];
}

static bool same_owner(ghostos_dlm_lock_owner a, ghostos_dlm_lock_owner b)
{
    return a.node == b.node && a.address_space == b.address_space;
}

static bool same_name(const ghostos_dlm_lock_entry *entry, const uint8_t *name,
                      size_t length)
{
    if (entry->name_length != length) return false;
    for (size_t i = 0; i < length; ++i) if (entry->name[i] != name[i]) return false;
    return true;
}

static bool valid_utf8_name(const uint8_t *bytes, size_t length)
{
    if (!bytes || !length || length > GHOSTOS_DLM_MAX_RESOURCE_NAME_BYTES) return false;
    for (size_t i = 0; i < length;) {
        uint8_t first = bytes[i++];
        if (!first) return false;
        if (first < 0x80) continue;
        size_t trailing;
        uint32_t value;
        if (first >= 0xc2 && first <= 0xdf) { trailing = 1; value = first & 0x1f; }
        else if (first >= 0xe0 && first <= 0xef) { trailing = 2; value = first & 0x0f; }
        else if (first >= 0xf0 && first <= 0xf4) { trailing = 3; value = first & 0x07; }
        else return false;
        if (trailing > length - i) return false;
        for (size_t j = 0; j < trailing; ++j) {
            uint8_t next = bytes[i++];
            if ((next & 0xc0) != 0x80) return false;
            value = (value << 6) | (next & 0x3f);
        }
        if ((trailing == 1 && value < 0x80) ||
            (trailing == 2 && value < 0x800) ||
            (trailing == 3 && value < 0x10000) ||
            (value >= 0xd800 && value <= 0xdfff) || value > 0x10ffff) return false;
    }
    return true;
}

static uint64_t sat_add(uint64_t a, uint64_t b)
{
    return UINT64_MAX - a < b ? UINT64_MAX : a + b;
}

static ghostos_dlm_error find_node(const ghostos_dlm_node_fence_table *table,
                                   uint32_t node, size_t *slot)
{
    if (!table || !node) return GHOSTOS_DLM_NODE_NOT_FOUND;
    for (size_t i = 0; i < table->capacity; ++i)
        if (table->entries[i].occupied && table->entries[i].node == node) {
            if (slot) *slot = i;
            return GHOSTOS_DLM_OK;
        }
    return GHOSTOS_DLM_NODE_NOT_FOUND;
}

bool ghostos_dlm_node_fence_init(ghostos_dlm_node_fence_table *table, size_t capacity)
{
    if (!table || capacity > GHOSTOS_DLM_MAX_NODES) return false;
    table->capacity = capacity;
    for (size_t i = 0; i < capacity; ++i)
        table->entries[i] = (ghostos_dlm_node_fence_entry){0};
    return true;
}

ghostos_dlm_error ghostos_dlm_node_admit(ghostos_dlm_node_fence_table *table,
                                          uint32_t node, uint64_t epoch)
{
    if (!table || !node) return GHOSTOS_DLM_NODE_NOT_FOUND;
    if (!epoch) return GHOSTOS_DLM_INVALID_EPOCH;
    for (size_t i = 0; i < table->capacity; ++i) {
        ghostos_dlm_node_fence_entry *entry = &table->entries[i];
        if (entry->occupied && entry->node == node) {
            if (entry->state != GHOSTOS_DLM_NODE_ISOLATED || epoch <= entry->epoch)
                return GHOSTOS_DLM_STALE_EPOCH;
            entry->epoch = epoch;
            entry->state = GHOSTOS_DLM_NODE_ACTIVE;
            return GHOSTOS_DLM_OK;
        }
    }
    for (size_t i = 0; i < table->capacity; ++i)
        if (!table->entries[i].occupied) {
            table->entries[i] = (ghostos_dlm_node_fence_entry){node, epoch,
                                                    GHOSTOS_DLM_NODE_ACTIVE, true};
            return GHOSTOS_DLM_OK;
        }
    return GHOSTOS_DLM_CAPACITY;
}

ghostos_dlm_error ghostos_dlm_node_begin_eviction(ghostos_dlm_node_fence_table *table,
    uint32_t node, uint64_t expected_epoch, ghostos_dlm_node_fence_token *token)
{
    size_t slot;
    ghostos_dlm_error error = find_node(table, node, &slot);
    if (error) return error;
    ghostos_dlm_node_fence_entry *entry = &table->entries[slot];
    if (entry->state != GHOSTOS_DLM_NODE_ACTIVE || entry->epoch != expected_epoch)
        return GHOSTOS_DLM_STALE_EPOCH;
    if (entry->epoch == UINT64_MAX) return GHOSTOS_DLM_INVALID_EPOCH;
    entry->epoch++;
    entry->state = GHOSTOS_DLM_NODE_FENCING;
    if (token) *token = (ghostos_dlm_node_fence_token){node, entry->epoch};
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_node_confirm_isolated(ghostos_dlm_node_fence_table *table,
                                                      ghostos_dlm_node_fence_token token)
{
    size_t slot;
    ghostos_dlm_error error = find_node(table, token.node, &slot);
    if (error) return error;
    ghostos_dlm_node_fence_entry *entry = &table->entries[slot];
    if (entry->state != GHOSTOS_DLM_NODE_FENCING || entry->epoch != token.epoch)
        return GHOSTOS_DLM_STALE_EPOCH;
    entry->state = GHOSTOS_DLM_NODE_ISOLATED;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_node_validate(const ghostos_dlm_node_fence_table *table,
                                             uint32_t node, uint64_t epoch)
{
    size_t slot;
    ghostos_dlm_error error = find_node(table, node, &slot);
    if (error) return error;
    const ghostos_dlm_node_fence_entry *entry = &table->entries[slot];
    return entry->state == GHOSTOS_DLM_NODE_ACTIVE && entry->epoch == epoch
        ? GHOSTOS_DLM_OK : GHOSTOS_DLM_STALE_EPOCH;
}

bool ghostos_dlm_node_is_isolated(const ghostos_dlm_node_fence_table *table, uint32_t node)
{
    size_t slot;
    return find_node(table, node, &slot) == GHOSTOS_DLM_OK &&
           table->entries[slot].state == GHOSTOS_DLM_NODE_ISOLATED;
}

bool ghostos_dlm_get_node_state(const ghostos_dlm_node_fence_table *table, uint32_t node,
                                ghostos_dlm_node_state *state)
{
    size_t slot;
    if (find_node(table, node, &slot) != GHOSTOS_DLM_OK) return false;
    if (state) *state = table->entries[slot].state;
    return true;
}

bool ghostos_dlm_node_epoch(const ghostos_dlm_node_fence_table *table, uint32_t node,
                            uint64_t *epoch)
{
    size_t slot;
    if (find_node(table, node, &slot) != GHOSTOS_DLM_OK) return false;
    if (epoch) *epoch = table->entries[slot].epoch;
    return true;
}

static bool cluster_empty(uint64_t low, uint64_t high)
{
    return low == 0 && high == 0;
}

static ghostos_dlm_error find_federation(const ghostos_dlm_federation_fence_table *table,
    uint64_t low, uint64_t high, size_t *slot)
{
    if (!table) return GHOSTOS_DLM_INVALID_EPOCH;
    for (size_t i = 0; i < table->capacity; ++i)
        if (table->entries[i].occupied && table->entries[i].cluster_low == low &&
            table->entries[i].cluster_high == high) {
            if (slot) *slot = i;
            return GHOSTOS_DLM_OK;
        }
    return GHOSTOS_DLM_INVALID_EPOCH;
}

bool ghostos_dlm_federation_init(ghostos_dlm_federation_fence_table *table, size_t capacity)
{
    if (!table || capacity > GHOSTOS_DLM_MAX_FEDERATIONS) return false;
    table->capacity = capacity;
    for (size_t i = 0; i < capacity; ++i)
        table->entries[i] = (ghostos_dlm_federation_fence_entry){0};
    return true;
}

ghostos_dlm_error ghostos_dlm_federation_establish(ghostos_dlm_federation_fence_table *table,
    uint64_t low, uint64_t high, uint64_t epoch)
{
    if (cluster_empty(low, high) || !epoch || !table) return GHOSTOS_DLM_INVALID_EPOCH;
    size_t slot;
    if (find_federation(table, low, high, &slot) == GHOSTOS_DLM_OK) {
        if (epoch < table->entries[slot].epoch) return GHOSTOS_DLM_STALE_EPOCH;
        table->entries[slot].epoch = epoch;
        return GHOSTOS_DLM_OK;
    }
    for (size_t i = 0; i < table->capacity; ++i)
        if (!table->entries[i].occupied) {
            table->entries[i] = (ghostos_dlm_federation_fence_entry){low, high, epoch, true};
            return GHOSTOS_DLM_OK;
        }
    return GHOSTOS_DLM_CAPACITY;
}

ghostos_dlm_error ghostos_dlm_federation_advance(ghostos_dlm_federation_fence_table *table,
    uint64_t low, uint64_t high, uint64_t expected_epoch, uint64_t next_epoch)
{
    if (next_epoch <= expected_epoch) return GHOSTOS_DLM_INVALID_EPOCH;
    size_t slot;
    ghostos_dlm_error error = find_federation(table, low, high, &slot);
    if (error) return error;
    if (table->entries[slot].epoch != expected_epoch) return GHOSTOS_DLM_STALE_EPOCH;
    table->entries[slot].epoch = next_epoch;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_federation_validate(
    const ghostos_dlm_federation_fence_table *table, uint64_t low, uint64_t high,
    uint64_t epoch)
{
    size_t slot;
    ghostos_dlm_error error = find_federation(table, low, high, &slot);
    if (error) return error;
    return table->entries[slot].epoch == epoch ? GHOSTOS_DLM_OK : GHOSTOS_DLM_STALE_EPOCH;
}

bool ghostos_dlm_federation_epoch(const ghostos_dlm_federation_fence_table *table,
                                  uint64_t low, uint64_t high, uint64_t *epoch)
{
    size_t slot;
    if (find_federation(table, low, high, &slot) != GHOSTOS_DLM_OK) return false;
    if (epoch) *epoch = table->entries[slot].epoch;
    return true;
}

bool ghostos_dlm_init(ghostos_dlm *manager, size_t capacity)
{
    if (!manager || capacity > GHOSTOS_DLM_MAX_LOCKS) return false;
    manager->capacity = capacity;
    manager->sequence = manager->observed_at_us = 0;
    manager->acquisitions = manager->queued_acquisitions = manager->promotions = 0;
    manager->releases = manager->expirations = 0;
    manager->max_wait_duration = manager->max_hold_duration = 0;
    for (size_t i = 0; i < GHOSTOS_LOCK_DURATION_BUCKETS; ++i)
        manager->wait_duration_histogram[i] = manager->hold_duration_histogram[i] = 0;
    for (size_t i = 0; i < capacity; ++i)
        manager->locks[i] = (ghostos_dlm_lock_entry){0};
    return true;
}

bool ghostos_dlm_lock_mode_compatible(ghostos_dlm_lock_mode requested,
                                      ghostos_dlm_lock_mode granted)
{
    static const bool matrix[6][6] = {
        {true,true,true,true,true,true}, {true,true,true,true,true,false},
        {true,true,true,false,false,false}, {true,true,false,true,false,false},
        {true,true,false,false,false,false}, {true,false,false,false,false,false}
    };
    return (unsigned)requested < 6 && (unsigned)granted < 6 && matrix[requested][granted];
}

static bool range_valid(ghostos_dlm_lock_range range)
{
    return range.kind == GHOSTOS_DLM_WHOLE_OBJECT ||
        (range.kind == GHOSTOS_DLM_BYTE_RANGE && range.length &&
         range.start <= UINT64_MAX - range.length);
}

static bool ranges_overlap(ghostos_dlm_lock_range a, ghostos_dlm_lock_range b)
{
    if (a.kind == GHOSTOS_DLM_WHOLE_OBJECT || b.kind == GHOSTOS_DLM_WHOLE_OBJECT)
        return true;
    return a.start < b.start + b.length && b.start < a.start + a.length;
}

static size_t valid_slot(const ghostos_dlm *manager, ghostos_dlm_lock_handle handle)
{
    size_t slot = (uint32_t)handle.raw;
    uint32_t generation = (uint32_t)(handle.raw >> 32);
    if (!manager || slot >= manager->capacity || !generation ||
        !manager->locks[slot].occupied || manager->locks[slot].generation != generation)
        return SIZE_MAX;
    return slot;
}

static size_t owned_slot(const ghostos_dlm *manager, ghostos_dlm_lock_owner owner,
                         ghostos_dlm_lock_handle handle, ghostos_dlm_error *error)
{
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) { *error = GHOSTOS_DLM_INVALID_HANDLE; return slot; }
    if (!same_owner(manager->locks[slot].owner, owner)) {
        *error = GHOSTOS_DLM_NOT_OWNER;
        return SIZE_MAX;
    }
    *error = GHOSTOS_DLM_OK;
    return slot;
}

static bool can_grant(const ghostos_dlm *manager, uint64_t resource,
    ghostos_dlm_lock_range range, ghostos_dlm_lock_mode mode, size_t except)
{
    for (size_t i = 0; i < manager->capacity; ++i) {
        const ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (i != except && entry->occupied && entry->granted && entry->resource == resource &&
            ranges_overlap(range, entry->range) &&
            !ghostos_dlm_lock_mode_compatible(mode, entry->mode)) return false;
    }
    return true;
}

static bool has_waiter(const ghostos_dlm *manager, uint64_t resource,
                       ghostos_dlm_lock_range range)
{
    for (size_t i = 0; i < manager->capacity; ++i) {
        const ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (entry->occupied && !entry->granted && entry->resource == resource &&
            ranges_overlap(range, entry->range)) return true;
    }
    return false;
}

static void record_wait(ghostos_dlm *manager, uint64_t requested, uint64_t granted)
{
    uint64_t duration = granted >= requested ? granted - requested : 0;
    size_t bucket = ghostos_lock_duration_bucket(duration);
    manager->wait_duration_histogram[bucket] = sat_add(manager->wait_duration_histogram[bucket], 1);
    if (duration > manager->max_wait_duration) manager->max_wait_duration = duration;
}

static void record_hold(ghostos_dlm *manager, uint64_t granted, uint64_t released)
{
    uint64_t duration = released >= granted ? released - granted : 0;
    size_t bucket = ghostos_lock_duration_bucket(duration);
    manager->hold_duration_histogram[bucket] = sat_add(manager->hold_duration_histogram[bucket], 1);
    if (duration > manager->max_hold_duration) manager->max_hold_duration = duration;
}

static size_t promote(ghostos_dlm *manager, uint64_t resource)
{
    size_t promoted = 0;
    for (;;) {
        size_t selected = SIZE_MAX;
        uint64_t oldest = UINT64_MAX;
        for (size_t i = 0; i < manager->capacity; ++i) {
            ghostos_dlm_lock_entry *entry = &manager->locks[i];
            if (!entry->occupied || entry->granted || entry->resource != resource ||
                entry->sequence >= oldest || !can_grant(manager, resource, entry->range,
                                                         entry->mode, i)) continue;
            bool blocked_by_older = false;
            for (size_t j = 0; j < manager->capacity; ++j) {
                const ghostos_dlm_lock_entry *older = &manager->locks[j];
                if (older->occupied && !older->granted && older->resource == resource &&
                    older->sequence < entry->sequence && ranges_overlap(older->range, entry->range)) {
                    blocked_by_older = true;
                    break;
                }
            }
            if (!blocked_by_older) { selected = i; oldest = entry->sequence; }
        }
        if (selected == SIZE_MAX) break;
        ghostos_dlm_lock_entry *entry = &manager->locks[selected];
        record_wait(manager, entry->requested_at_us, manager->observed_at_us);
        entry->granted = true;
        entry->granted_at_us = manager->observed_at_us;
        manager->promotions = sat_add(manager->promotions, 1);
        promoted++;
    }
    return promoted;
}

static size_t promote_all(ghostos_dlm *manager)
{
    size_t promoted = 0;
    for (size_t i = 0; i < manager->capacity; ++i)
        if (manager->locks[i].occupied && !manager->locks[i].granted)
            promoted += promote(manager, manager->locks[i].resource);
    return promoted;
}

static ghostos_dlm_error acquire_internal(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind,
    const uint8_t *name, size_t name_length, ghostos_dlm_lock_range range,
    ghostos_dlm_lock_mode mode, bool wait, uint64_t expires_at, uint64_t requested_at,
    ghostos_dlm_lock_grant *grant)
{
    if (!manager || !capabilities || !grant || !resource ||
        !valid_utf8_name(name, name_length) ||
        (unsigned)kind > GHOSTOS_DLM_NAMED || (unsigned)mode > GHOSTOS_DLM_EXCLUSIVE ||
        !range_valid(range))
        return GHOSTOS_DLM_INVALID_RESOURCE;
    uint16_t rights = (mode == GHOSTOS_DLM_NULL || mode == GHOSTOS_DLM_CONCURRENT_READ ||
                       mode == GHOSTOS_DLM_PROTECTED_READ) ? GHOSTOS_RIGHT_READ : GHOSTOS_RIGHT_WRITE;
    ghostos_capability_object object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_DISTRIBUTED_RESOURCE, 0, 0, resource, 0);
    if (ghostos_capability_authorize(capabilities, owner.address_space, authority,
                                     object, rights) != GHOSTOS_CAP_OK)
        return GHOSTOS_DLM_ACCESS_DENIED;
    for (size_t i = 0; i < manager->capacity; ++i) {
        const ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (entry->occupied && entry->resource == resource &&
            (entry->resource_kind != kind || !same_name(entry, name, name_length)))
            return GHOSTOS_DLM_INVALID_RESOURCE;
    }
    bool can_now = !has_waiter(manager, resource, range) &&
                   can_grant(manager, resource, range, mode, SIZE_MAX);
    if (!can_now && !wait) return GHOSTOS_DLM_WOULD_BLOCK;
    size_t slot = SIZE_MAX;
    for (size_t i = 0; i < manager->capacity; ++i)
        if (!manager->locks[i].occupied) { slot = i; break; }
    if (slot == SIZE_MAX) return GHOSTOS_DLM_CAPACITY;
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    uint32_t generation = entry->generation + 1;
    if (!generation) generation = 1;
    uint64_t sequence = manager->sequence + 1;
    manager->sequence = sequence;
    manager->observed_at_us = manager->observed_at_us > requested_at
        ? manager->observed_at_us : requested_at;
    *entry = (ghostos_dlm_lock_entry){
        .occupied = true, .generation = generation, .resource = resource,
        .resource_kind = kind, .name_length = (uint8_t)name_length, .owner = owner,
        .mode = mode, .granted = can_now, .sequence = sequence, .range = range,
        .lease_epoch = 1, .expires_at_us = expires_at, .requested_at_us = requested_at,
        .granted_at_us = can_now ? manager->observed_at_us : 0
    };
    for (size_t i = 0; i < name_length; ++i) entry->name[i] = name[i];
    manager->acquisitions = sat_add(manager->acquisitions, 1);
    if (!can_now) manager->queued_acquisitions = sat_add(manager->queued_acquisitions, 1);
    if (grant) *grant = (ghostos_dlm_lock_grant){can_now ? GHOSTOS_DLM_GRANTED : GHOSTOS_DLM_QUEUED,
                                                  {(uint64_t)generation << 32 | (uint32_t)slot}};
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_acquire(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind,
    const uint8_t *name, size_t name_length, ghostos_dlm_lock_range range,
    ghostos_dlm_lock_mode mode, bool wait, uint64_t now_us, uint64_t duration_us,
    ghostos_dlm_lock_grant *grant)
{
    if (!duration_us || now_us > UINT64_MAX - duration_us) return GHOSTOS_DLM_INVALID_RANGE;
    if (!range_valid(range)) return GHOSTOS_DLM_INVALID_RANGE;
    if (!manager) return GHOSTOS_DLM_INVALID_RESOURCE;
    size_t expired = ghostos_dlm_expire(manager, now_us);
    (void)expired;
    return acquire_internal(manager, capabilities, authority, owner, resource, kind, name,
        name_length, range, mode, wait, now_us + duration_us, now_us, grant);
}

ghostos_dlm_error ghostos_dlm_acquire_unleased(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind,
    const uint8_t *name, size_t name_length, ghostos_dlm_lock_mode mode, bool wait,
    ghostos_dlm_lock_grant *grant)
{
    if (!manager) return GHOSTOS_DLM_INVALID_RESOURCE;
    return acquire_internal(manager, capabilities, authority, owner, resource, kind, name,
        name_length, (ghostos_dlm_lock_range){GHOSTOS_DLM_WHOLE_OBJECT, 0, 0}, mode,
        wait, UINT64_MAX, manager->observed_at_us, grant);
}

ghostos_dlm_error ghostos_dlm_acquire_node(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t node_epoch, uint64_t resource,
    ghostos_dlm_resource_kind kind, const uint8_t *name, size_t name_length,
    ghostos_dlm_lock_range range, ghostos_dlm_lock_mode mode, bool wait,
    uint64_t now_us, uint64_t duration_us, const ghostos_dlm_node_fence_table *fences,
    ghostos_dlm_lock_grant *grant)
{
    ghostos_dlm_error error = ghostos_dlm_node_validate(fences, owner.node, node_epoch);
    if (error) return error;
    error = ghostos_dlm_acquire(manager, capabilities, authority, owner, resource, kind,
                                name, name_length, range, mode, wait, now_us, duration_us, grant);
    if (error) return error;
    size_t slot = valid_slot(manager, grant->handle);
    manager->locks[slot].node_epoch = node_epoch;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_acquire_federated(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind,
    const uint8_t *name, size_t name_length, ghostos_dlm_lock_range range,
    ghostos_dlm_lock_mode mode, bool wait, uint64_t now_us, uint64_t duration_us,
    uint64_t low, uint64_t high, uint64_t epoch,
    const ghostos_dlm_federation_fence_table *fences, ghostos_dlm_lock_grant *grant)
{
    ghostos_dlm_error error = ghostos_dlm_federation_validate(fences, low, high, epoch);
    if (error) return error;
    error = ghostos_dlm_acquire(manager, capabilities, authority, owner, resource, kind,
                                name, name_length, range, mode, wait, now_us, duration_us, grant);
    if (error) return error;
    size_t slot = valid_slot(manager, grant->handle);
    manager->locks[slot].federated = true;
    manager->locks[slot].federation_low = low;
    manager->locks[slot].federation_high = high;
    manager->locks[slot].federation_epoch = epoch;
    return GHOSTOS_DLM_OK;
}

static ghostos_dlm_error convert_internal(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t node_epoch, bool check_node,
    ghostos_dlm_lock_handle handle, ghostos_dlm_lock_mode mode)
{
    ghostos_dlm_error error;
    size_t slot = owned_slot(manager, owner, handle, &error);
    if (error) return error;
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if ((!check_node && entry->node_epoch) || (check_node && entry->node_epoch != node_epoch))
        return check_node ? GHOSTOS_DLM_STALE_EPOCH : GHOSTOS_DLM_INVALID_EPOCH;
    uint16_t rights = (mode == GHOSTOS_DLM_NULL || mode == GHOSTOS_DLM_CONCURRENT_READ ||
                       mode == GHOSTOS_DLM_PROTECTED_READ) ? GHOSTOS_RIGHT_READ : GHOSTOS_RIGHT_WRITE;
    ghostos_capability_object object = ghostos_capability_object_make(
        GHOSTOS_OBJECT_DISTRIBUTED_RESOURCE, 0, 0, entry->resource, 0);
    if (ghostos_capability_authorize(capabilities, owner.address_space, authority,
                                     object, rights) != GHOSTOS_CAP_OK)
        return GHOSTOS_DLM_ACCESS_DENIED;
    if (!entry->granted || !can_grant(manager, entry->resource, entry->range, mode, slot))
        return GHOSTOS_DLM_WOULD_BLOCK;
    entry->mode = mode;
    (void)promote(manager, entry->resource);
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_convert(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, ghostos_dlm_lock_mode mode)
{
    return convert_internal(manager, capabilities, authority, owner, 0, false, handle, mode);
}

ghostos_dlm_error ghostos_dlm_convert_node(ghostos_dlm *manager,
    const ghostos_capability_space *capabilities, ghostos_capability_handle authority,
    ghostos_dlm_lock_owner owner, uint64_t node_epoch, ghostos_dlm_lock_handle handle,
    ghostos_dlm_lock_mode mode, const ghostos_dlm_node_fence_table *fences)
{
    ghostos_dlm_error error = ghostos_dlm_node_validate(fences, owner.node, node_epoch);
    if (error) return error;
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) return GHOSTOS_DLM_INVALID_HANDLE;
    if (manager->locks[slot].federated) return GHOSTOS_DLM_INVALID_EPOCH;
    error = convert_internal(manager, capabilities, authority, owner, node_epoch, true, handle, mode);
    if (error) return error;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_release(ghostos_dlm *manager, ghostos_dlm_lock_owner owner,
    ghostos_dlm_lock_handle handle, uint64_t now_us, size_t *promoted)
{
    ghostos_dlm_error error;
    size_t slot = owned_slot(manager, owner, handle, &error);
    if (error) return error;
    if (now_us > manager->observed_at_us) manager->observed_at_us = now_us;
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    uint64_t resource = entry->resource;
    if (entry->granted) record_hold(manager, entry->granted_at_us, manager->observed_at_us);
    entry->occupied = false;
    manager->releases = sat_add(manager->releases, 1);
    size_t count = promote(manager, resource);
    if (promoted) *promoted = count;
    return GHOSTOS_DLM_OK;
}

static ghostos_dlm_error renew_slot(ghostos_dlm *manager, size_t slot,
    uint64_t expected_epoch, uint64_t now_us, uint64_t duration_us, uint64_t *new_epoch)
{
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (entry->expires_at_us <= now_us || entry->lease_epoch != expected_epoch)
        return GHOSTOS_DLM_EXPIRED;
    if (!duration_us || now_us > UINT64_MAX - duration_us) return GHOSTOS_DLM_INVALID_RANGE;
    entry->expires_at_us = now_us + duration_us;
    entry->lease_epoch++;
    if (!entry->lease_epoch) entry->lease_epoch = 1;
    if (new_epoch) *new_epoch = entry->lease_epoch;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_renew(ghostos_dlm *manager, ghostos_dlm_lock_owner owner,
    ghostos_dlm_lock_handle handle, uint64_t expected_epoch, uint64_t now_us,
    uint64_t duration_us, uint64_t *new_epoch)
{
    ghostos_dlm_error error;
    size_t slot = owned_slot(manager, owner, handle, &error);
    if (error) return error;
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (entry->federated || entry->node_epoch) return GHOSTOS_DLM_INVALID_EPOCH;
    if (now_us > manager->observed_at_us) manager->observed_at_us = now_us;
    return renew_slot(manager, slot, expected_epoch, now_us, duration_us, new_epoch);
}

ghostos_dlm_error ghostos_dlm_renew_node(ghostos_dlm *manager, ghostos_dlm_lock_owner owner,
    ghostos_dlm_lock_handle handle, uint64_t node_epoch, uint64_t expected_epoch,
    uint64_t now_us, uint64_t duration_us, const ghostos_dlm_node_fence_table *fences,
    uint64_t *new_epoch)
{
    ghostos_dlm_error error = ghostos_dlm_node_validate(fences, owner.node, node_epoch);
    if (error) return error;
    size_t slot = owned_slot(manager, owner, handle, &error);
    if (error) return error;
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (entry->node_epoch != node_epoch || entry->federated) return GHOSTOS_DLM_STALE_EPOCH;
    if (now_us > manager->observed_at_us) manager->observed_at_us = now_us;
    return renew_slot(manager, slot, expected_epoch, now_us, duration_us, new_epoch);
}

ghostos_dlm_error ghostos_dlm_renew_federated(ghostos_dlm *manager,
    ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, uint64_t expected_epoch,
    uint64_t now_us, uint64_t duration_us, const ghostos_dlm_federation_fence_table *fences,
    uint64_t *new_epoch)
{
    ghostos_dlm_error error;
    size_t slot = owned_slot(manager, owner, handle, &error);
    if (error) return error;
    ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (!entry->federated) return GHOSTOS_DLM_INVALID_EPOCH;
    error = ghostos_dlm_federation_validate(fences, entry->federation_low,
        entry->federation_high, entry->federation_epoch);
    if (error) return error;
    if (now_us > manager->observed_at_us) manager->observed_at_us = now_us;
    return renew_slot(manager, slot, expected_epoch, now_us, duration_us, new_epoch);
}

ghostos_dlm_error ghostos_dlm_validate_node_lease(const ghostos_dlm *manager,
    ghostos_dlm_lock_handle handle, uint64_t now_us, const ghostos_dlm_node_fence_table *fences)
{
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) return GHOSTOS_DLM_INVALID_HANDLE;
    const ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (!entry->granted || entry->expires_at_us <= now_us) return GHOSTOS_DLM_EXPIRED;
    if (!entry->node_epoch) return GHOSTOS_DLM_INVALID_EPOCH;
    return ghostos_dlm_node_validate(fences, entry->owner.node, entry->node_epoch);
}

ghostos_dlm_error ghostos_dlm_validate_federated_lease(const ghostos_dlm *manager,
    ghostos_dlm_lock_handle handle, uint64_t now_us,
    const ghostos_dlm_federation_fence_table *fences)
{
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) return GHOSTOS_DLM_INVALID_HANDLE;
    const ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (!entry->granted || entry->expires_at_us <= now_us) return GHOSTOS_DLM_EXPIRED;
    if (!entry->federated) return GHOSTOS_DLM_INVALID_EPOCH;
    return ghostos_dlm_federation_validate(fences, entry->federation_low,
        entry->federation_high, entry->federation_epoch);
}

size_t ghostos_dlm_fence_cluster(ghostos_dlm *manager, uint64_t low, uint64_t high,
                                  uint64_t current_epoch)
{
    size_t removed = 0;
    for (size_t i = 0; i < manager->capacity; ++i) {
        ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (entry->occupied && entry->federated && entry->federation_low == low &&
            entry->federation_high == high && entry->federation_epoch != current_epoch) {
            if (entry->granted) record_hold(manager, entry->granted_at_us, manager->observed_at_us);
            entry->occupied = false;
            manager->releases = sat_add(manager->releases, 1);
            removed++;
        }
    }
    if (removed) (void)promote_all(manager);
    return removed;
}

size_t ghostos_dlm_expire(ghostos_dlm *manager, uint64_t now_us)
{
    if (!manager) return 0;
    if (now_us > manager->observed_at_us) manager->observed_at_us = now_us;
    size_t expired = 0;
    for (size_t i = 0; i < manager->capacity; ++i) {
        ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (entry->occupied && entry->expires_at_us <= now_us) {
            if (entry->granted) record_hold(manager, entry->granted_at_us, now_us);
            entry->occupied = false;
            manager->releases = sat_add(manager->releases, 1);
            expired++;
        }
    }
    manager->expirations = sat_add(manager->expirations, expired);
    if (expired) (void)promote_all(manager);
    return expired;
}

ghostos_dlm_error ghostos_dlm_evict_node(ghostos_dlm *manager, uint32_t node,
    const ghostos_dlm_node_fence_table *fences, size_t *removed_out)
{
    if (!ghostos_dlm_node_is_isolated(fences, node)) return GHOSTOS_DLM_NODE_NOT_ISOLATED;
    size_t removed = 0;
    for (size_t i = 0; i < manager->capacity; ++i) {
        ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (entry->occupied && entry->owner.node == node) {
            if (entry->granted) record_hold(manager, entry->granted_at_us, manager->observed_at_us);
            entry->occupied = false;
            manager->releases = sat_add(manager->releases, 1);
            removed++;
        }
    }
    (void)promote_all(manager);
    if (removed_out) *removed_out = removed;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_is_granted(const ghostos_dlm *manager,
    ghostos_dlm_lock_handle handle, bool *granted)
{
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) return GHOSTOS_DLM_INVALID_HANDLE;
    if (granted) *granted = manager->locks[slot].granted;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_resource(const ghostos_dlm *manager,
    ghostos_dlm_lock_handle handle, uint64_t *resource, ghostos_dlm_resource_kind *kind,
    uint8_t *name, size_t name_capacity, size_t *name_length)
{
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) return GHOSTOS_DLM_INVALID_HANDLE;
    const ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (name && name_capacity < entry->name_length) return GHOSTOS_DLM_CAPACITY;
    if (resource) *resource = entry->resource;
    if (kind) *kind = entry->resource_kind;
    if (name) for (size_t i = 0; i < entry->name_length; ++i) name[i] = entry->name[i];
    if (name_length) *name_length = entry->name_length;
    return GHOSTOS_DLM_OK;
}

ghostos_dlm_error ghostos_dlm_lease(const ghostos_dlm *manager,
    ghostos_dlm_lock_handle handle, ghostos_dlm_lock_range *range,
    uint64_t *lease_epoch, uint64_t *expires_at_us)
{
    size_t slot = valid_slot(manager, handle);
    if (slot == SIZE_MAX) return GHOSTOS_DLM_INVALID_HANDLE;
    const ghostos_dlm_lock_entry *entry = &manager->locks[slot];
    if (range) *range = entry->range;
    if (lease_epoch) *lease_epoch = entry->lease_epoch;
    if (expires_at_us) *expires_at_us = entry->expires_at_us;
    return GHOSTOS_DLM_OK;
}

size_t ghostos_dlm_used(const ghostos_dlm *manager)
{
    size_t used = 0;
    if (!manager) return 0;
    for (size_t i = 0; i < manager->capacity; ++i) used += manager->locks[i].occupied;
    return used;
}

ghostos_dlm_contention_report ghostos_dlm_contention(const ghostos_dlm *manager,
                                                       uint64_t now_us)
{
    ghostos_dlm_contention_report report = {0};
    if (!manager) return report;
    report.acquisitions = manager->acquisitions;
    report.queued_acquisitions = manager->queued_acquisitions;
    report.promotions = manager->promotions;
    report.releases = manager->releases;
    report.expirations = manager->expirations;
    report.max_wait_duration = manager->max_wait_duration;
    report.max_hold_duration = manager->max_hold_duration;
    for (size_t i = 0; i < GHOSTOS_LOCK_DURATION_BUCKETS; ++i) {
        report.wait_duration_histogram[i] = manager->wait_duration_histogram[i];
        report.hold_duration_histogram[i] = manager->hold_duration_histogram[i];
    }
    for (size_t i = 0; i < manager->capacity; ++i) {
        const ghostos_dlm_lock_entry *entry = &manager->locks[i];
        if (!entry->occupied) continue;
        report.active_locks++;
        if (entry->granted) {
            uint64_t end = now_us > manager->observed_at_us ? now_us : manager->observed_at_us;
            uint64_t duration = end >= entry->granted_at_us ? end - entry->granted_at_us : 0;
            if (duration > report.max_hold_duration) report.max_hold_duration = duration;
        }
        if (report.active_owner_count < 8)
            report.active_owners[report.active_owner_count++] = (ghostos_dlm_active_owner){
                entry->resource, entry->owner, entry->mode, entry->granted,
                entry->requested_at_us, entry->granted_at_us
            };
    }
    return report;
}

const ghostos_dlm_lock_entry *ghostos_dlm_lock(const ghostos_dlm *manager, size_t index)
{
    return manager && index < manager->capacity ? &manager->locks[index] : NULL;
}

ghostos_dlm_lock_entry *ghostos_dlm_lock_mut(ghostos_dlm *manager, size_t index)
{
    return manager && index < manager->capacity ? &manager->locks[index] : NULL;
}

ghostos_status ghostos_dlm_error_status(ghostos_dlm_error error)
{
    unsigned severity = GHOSTOS_SEVERITY_ERROR;
    unsigned code;
    switch (error) {
    case GHOSTOS_DLM_OK: return GHOSTOS_STATUS_NORMAL;
    case GHOSTOS_DLM_WOULD_BLOCK: severity = GHOSTOS_SEVERITY_WARNING; code = 1; break;
    case GHOSTOS_DLM_CAPACITY: code = 2; break;
    case GHOSTOS_DLM_INVALID_HANDLE: code = 3; break;
    case GHOSTOS_DLM_INVALID_RESOURCE: code = 4; break;
    case GHOSTOS_DLM_INVALID_RANGE: code = 5; break;
    case GHOSTOS_DLM_EXPIRED: severity = GHOSTOS_SEVERITY_WARNING; code = 6; break;
    case GHOSTOS_DLM_INVALID_EPOCH: code = 7; break;
    case GHOSTOS_DLM_STALE_EPOCH: code = 8; break;
    case GHOSTOS_DLM_NODE_NOT_FOUND: code = 9; break;
    case GHOSTOS_DLM_NODE_NOT_ISOLATED: code = 10; break;
    case GHOSTOS_DLM_ACCESS_DENIED:
    case GHOSTOS_DLM_NOT_OWNER: return GHOSTOS_STATUS_ACCESS_DENIED;
    default: return GHOSTOS_STATUS_INVALID_ARGUMENT;
    }
    return GHOSTOS_STATUS_BITS(severity, GHOSTOS_FACILITY_DLM, code, 0);
}
