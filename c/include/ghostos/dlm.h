#ifndef GHOSTOS_DLM_H
#define GHOSTOS_DLM_H

#include "ghostos/capability.h"
#include "ghostos/contention.h"

#define GHOSTOS_DLM_MAX_LOCKS 256u
#define GHOSTOS_DLM_MAX_NODES 64u
#define GHOSTOS_DLM_MAX_FEDERATIONS 32u
#define GHOSTOS_DLM_MAX_RESOURCE_NAME_BYTES 64u

typedef enum { GHOSTOS_DLM_NODE_ACTIVE, GHOSTOS_DLM_NODE_FENCING,
               GHOSTOS_DLM_NODE_ISOLATED } ghostos_dlm_node_state;
typedef struct { uint32_t node; uint64_t epoch; } ghostos_dlm_node_fence_token;
typedef struct { uint32_t node; uint64_t epoch; ghostos_dlm_node_state state; bool occupied; }
    ghostos_dlm_node_fence_entry;
typedef struct { size_t capacity; ghostos_dlm_node_fence_entry entries[GHOSTOS_DLM_MAX_NODES]; }
    ghostos_dlm_node_fence_table;
typedef struct { uint64_t cluster_low, cluster_high, epoch; bool occupied; }
    ghostos_dlm_federation_fence_entry;
typedef struct { size_t capacity; ghostos_dlm_federation_fence_entry entries[GHOSTOS_DLM_MAX_FEDERATIONS]; }
    ghostos_dlm_federation_fence_table;

typedef enum { GHOSTOS_DLM_SHARED_MEMORY, GHOSTOS_DLM_FILE, GHOSTOS_DLM_NAMED }
    ghostos_dlm_resource_kind;
typedef enum { GHOSTOS_DLM_NULL, GHOSTOS_DLM_CONCURRENT_READ,
    GHOSTOS_DLM_CONCURRENT_WRITE, GHOSTOS_DLM_PROTECTED_READ,
    GHOSTOS_DLM_PROTECTED_WRITE, GHOSTOS_DLM_EXCLUSIVE } ghostos_dlm_lock_mode;
typedef enum { GHOSTOS_DLM_WHOLE_OBJECT, GHOSTOS_DLM_BYTE_RANGE } ghostos_dlm_range_kind;
typedef struct { ghostos_dlm_range_kind kind; uint64_t start, length; }
    ghostos_dlm_lock_range;
typedef struct { uint32_t node, address_space; } ghostos_dlm_lock_owner;
typedef struct { uint64_t raw; } ghostos_dlm_lock_handle;
typedef enum { GHOSTOS_DLM_GRANTED, GHOSTOS_DLM_QUEUED } ghostos_dlm_grant_state;
typedef struct { ghostos_dlm_grant_state state; ghostos_dlm_lock_handle handle; }
    ghostos_dlm_lock_grant;
typedef enum { GHOSTOS_DLM_OK, GHOSTOS_DLM_ACCESS_DENIED, GHOSTOS_DLM_CAPACITY,
    GHOSTOS_DLM_INVALID_HANDLE, GHOSTOS_DLM_INVALID_RESOURCE, GHOSTOS_DLM_INVALID_RANGE,
    GHOSTOS_DLM_INVALID_EPOCH, GHOSTOS_DLM_NODE_NOT_FOUND, GHOSTOS_DLM_NODE_NOT_ISOLATED,
    GHOSTOS_DLM_NOT_OWNER, GHOSTOS_DLM_EXPIRED, GHOSTOS_DLM_STALE_EPOCH,
    GHOSTOS_DLM_WOULD_BLOCK } ghostos_dlm_error;
typedef struct {
    bool occupied;
    uint32_t generation;
    uint64_t resource;
    ghostos_dlm_resource_kind resource_kind;
    uint8_t name[GHOSTOS_DLM_MAX_RESOURCE_NAME_BYTES];
    uint8_t name_length;
    ghostos_dlm_lock_owner owner;
    ghostos_dlm_lock_mode mode;
    bool granted;
    uint64_t sequence;
    ghostos_dlm_lock_range range;
    uint64_t lease_epoch, expires_at_us, node_epoch;
    bool federated;
    uint64_t federation_low, federation_high, federation_epoch;
    uint64_t requested_at_us, granted_at_us;
} ghostos_dlm_lock_entry;
typedef struct {
    size_t capacity;
    ghostos_dlm_lock_entry locks[GHOSTOS_DLM_MAX_LOCKS];
    uint64_t sequence, observed_at_us;
    uint64_t acquisitions, queued_acquisitions, promotions, releases, expirations;
    uint64_t wait_duration_histogram[GHOSTOS_LOCK_DURATION_BUCKETS];
    uint64_t hold_duration_histogram[GHOSTOS_LOCK_DURATION_BUCKETS];
    uint64_t max_wait_duration, max_hold_duration;
} ghostos_dlm;
typedef struct {
    uint64_t resource;
    ghostos_dlm_lock_owner owner;
    ghostos_dlm_lock_mode mode;
    bool granted;
    uint64_t requested_at_us, granted_at_us;
} ghostos_dlm_active_owner;
typedef struct {
    size_t active_locks;
    ghostos_dlm_active_owner active_owners[8];
    uint8_t active_owner_count;
    uint64_t acquisitions, queued_acquisitions, promotions, releases, expirations;
    uint64_t wait_duration_histogram[GHOSTOS_LOCK_DURATION_BUCKETS];
    uint64_t hold_duration_histogram[GHOSTOS_LOCK_DURATION_BUCKETS];
    uint64_t max_wait_duration, max_hold_duration;
} ghostos_dlm_contention_report;

bool ghostos_dlm_node_fence_init(ghostos_dlm_node_fence_table *table, size_t capacity);
ghostos_dlm_error ghostos_dlm_node_admit(ghostos_dlm_node_fence_table *table, uint32_t node, uint64_t epoch);
ghostos_dlm_error ghostos_dlm_node_begin_eviction(ghostos_dlm_node_fence_table *table, uint32_t node, uint64_t expected_epoch, ghostos_dlm_node_fence_token *token);
ghostos_dlm_error ghostos_dlm_node_confirm_isolated(ghostos_dlm_node_fence_table *table, ghostos_dlm_node_fence_token token);
ghostos_dlm_error ghostos_dlm_node_validate(const ghostos_dlm_node_fence_table *table, uint32_t node, uint64_t epoch);
bool ghostos_dlm_node_is_isolated(const ghostos_dlm_node_fence_table *table, uint32_t node);
bool ghostos_dlm_get_node_state(const ghostos_dlm_node_fence_table *table, uint32_t node, ghostos_dlm_node_state *state);
bool ghostos_dlm_node_epoch(const ghostos_dlm_node_fence_table *table, uint32_t node, uint64_t *epoch);
bool ghostos_dlm_federation_init(ghostos_dlm_federation_fence_table *table, size_t capacity);
ghostos_dlm_error ghostos_dlm_federation_establish(ghostos_dlm_federation_fence_table *table, uint64_t cluster_low, uint64_t cluster_high, uint64_t epoch);
ghostos_dlm_error ghostos_dlm_federation_advance(ghostos_dlm_federation_fence_table *table, uint64_t cluster_low, uint64_t cluster_high, uint64_t expected_epoch, uint64_t next_epoch);
ghostos_dlm_error ghostos_dlm_federation_validate(const ghostos_dlm_federation_fence_table *table, uint64_t cluster_low, uint64_t cluster_high, uint64_t epoch);
bool ghostos_dlm_federation_epoch(const ghostos_dlm_federation_fence_table *table, uint64_t cluster_low, uint64_t cluster_high, uint64_t *epoch);
bool ghostos_dlm_init(ghostos_dlm *manager, size_t capacity);
bool ghostos_dlm_lock_mode_compatible(ghostos_dlm_lock_mode requested, ghostos_dlm_lock_mode granted);
ghostos_dlm_error ghostos_dlm_acquire(ghostos_dlm *manager, const ghostos_capability_space *capabilities, ghostos_capability_handle authority, ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind, const uint8_t *name, size_t name_length, ghostos_dlm_lock_range range, ghostos_dlm_lock_mode mode, bool wait, uint64_t now_us, uint64_t lease_duration_us, ghostos_dlm_lock_grant *grant);
ghostos_dlm_error ghostos_dlm_acquire_unleased(ghostos_dlm *manager, const ghostos_capability_space *capabilities, ghostos_capability_handle authority, ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind, const uint8_t *name, size_t name_length, ghostos_dlm_lock_mode mode, bool wait, ghostos_dlm_lock_grant *grant);
ghostos_dlm_error ghostos_dlm_acquire_node(ghostos_dlm *manager, const ghostos_capability_space *capabilities, ghostos_capability_handle authority, ghostos_dlm_lock_owner owner, uint64_t node_epoch, uint64_t resource, ghostos_dlm_resource_kind kind, const uint8_t *name, size_t name_length, ghostos_dlm_lock_range range, ghostos_dlm_lock_mode mode, bool wait, uint64_t now_us, uint64_t lease_duration_us, const ghostos_dlm_node_fence_table *fences, ghostos_dlm_lock_grant *grant);
ghostos_dlm_error ghostos_dlm_acquire_federated(ghostos_dlm *manager, const ghostos_capability_space *capabilities, ghostos_capability_handle authority, ghostos_dlm_lock_owner owner, uint64_t resource, ghostos_dlm_resource_kind kind, const uint8_t *name, size_t name_length, ghostos_dlm_lock_range range, ghostos_dlm_lock_mode mode, bool wait, uint64_t now_us, uint64_t lease_duration_us, uint64_t cluster_low, uint64_t cluster_high, uint64_t federation_epoch, const ghostos_dlm_federation_fence_table *fences, ghostos_dlm_lock_grant *grant);
ghostos_dlm_error ghostos_dlm_convert(ghostos_dlm *manager, const ghostos_capability_space *capabilities, ghostos_capability_handle authority, ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, ghostos_dlm_lock_mode mode);
ghostos_dlm_error ghostos_dlm_convert_node(ghostos_dlm *manager, const ghostos_capability_space *capabilities, ghostos_capability_handle authority, ghostos_dlm_lock_owner owner, uint64_t node_epoch, ghostos_dlm_lock_handle handle, ghostos_dlm_lock_mode mode, const ghostos_dlm_node_fence_table *fences);
ghostos_dlm_error ghostos_dlm_release(ghostos_dlm *manager, ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, uint64_t now_us, size_t *promoted);
ghostos_dlm_error ghostos_dlm_renew(ghostos_dlm *manager, ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, uint64_t expected_epoch, uint64_t now_us, uint64_t duration_us, uint64_t *new_epoch);
ghostos_dlm_error ghostos_dlm_renew_node(ghostos_dlm *manager, ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, uint64_t node_epoch, uint64_t expected_epoch, uint64_t now_us, uint64_t duration_us, const ghostos_dlm_node_fence_table *fences, uint64_t *new_epoch);
ghostos_dlm_error ghostos_dlm_renew_federated(ghostos_dlm *manager, ghostos_dlm_lock_owner owner, ghostos_dlm_lock_handle handle, uint64_t expected_epoch, uint64_t now_us, uint64_t duration_us, const ghostos_dlm_federation_fence_table *fences, uint64_t *new_epoch);
ghostos_dlm_error ghostos_dlm_validate_node_lease(const ghostos_dlm *manager, ghostos_dlm_lock_handle handle, uint64_t now_us, const ghostos_dlm_node_fence_table *fences);
ghostos_dlm_error ghostos_dlm_validate_federated_lease(const ghostos_dlm *manager, ghostos_dlm_lock_handle handle, uint64_t now_us, const ghostos_dlm_federation_fence_table *fences);
size_t ghostos_dlm_fence_cluster(ghostos_dlm *manager, uint64_t cluster_low, uint64_t cluster_high, uint64_t current_epoch);
size_t ghostos_dlm_expire(ghostos_dlm *manager, uint64_t now_us);
ghostos_dlm_error ghostos_dlm_evict_node(ghostos_dlm *manager, uint32_t node, const ghostos_dlm_node_fence_table *fences, size_t *removed);
ghostos_dlm_error ghostos_dlm_is_granted(const ghostos_dlm *manager, ghostos_dlm_lock_handle handle, bool *granted);
ghostos_dlm_error ghostos_dlm_resource(const ghostos_dlm *manager, ghostos_dlm_lock_handle handle, uint64_t *resource, ghostos_dlm_resource_kind *kind, uint8_t *name, size_t name_capacity, size_t *name_length);
ghostos_dlm_error ghostos_dlm_lease(const ghostos_dlm *manager, ghostos_dlm_lock_handle handle, ghostos_dlm_lock_range *range, uint64_t *lease_epoch, uint64_t *expires_at_us);
size_t ghostos_dlm_used(const ghostos_dlm *manager);
ghostos_dlm_contention_report ghostos_dlm_contention(const ghostos_dlm *manager, uint64_t now_us);
const ghostos_dlm_lock_entry *ghostos_dlm_lock(const ghostos_dlm *manager, size_t index);
ghostos_dlm_lock_entry *ghostos_dlm_lock_mut(ghostos_dlm *manager, size_t index);
ghostos_status ghostos_dlm_error_status(ghostos_dlm_error error);
void ghostos_dlm_kernel_init(void);
bool ghostos_dlm_kernel_lock_summary(size_t index, uint64_t *resource, uint32_t *owner_node,
    uint32_t *address_space, uint32_t *mode, bool *granted,
    uint64_t *requested_at_us, uint64_t *granted_at_us);
void ghostos_dlm_kernel_counters(uint64_t *acquisitions, uint64_t *queued,
    uint64_t *promotions, uint64_t *releases, uint64_t *expirations,
    uint64_t wait_histogram[GHOSTOS_LOCK_DURATION_BUCKETS],
    uint64_t hold_histogram[GHOSTOS_LOCK_DURATION_BUCKETS],
    uint64_t *max_wait, uint64_t *max_hold, size_t *active, uint64_t now_us);

#endif
