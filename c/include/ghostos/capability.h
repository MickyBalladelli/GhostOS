#ifndef GHOSTOS_CAPABILITY_H
#define GHOSTOS_CAPABILITY_H

#include "ghostos/status.h"

#include <stdatomic.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_CAPABILITIES 256
#define GHOSTOS_CAPABILITY_NO_SLOT UINT32_MAX

typedef uint64_t ghostos_capability_handle;
typedef enum { GHOSTOS_OBJECT_UNTYPED=1,GHOSTOS_OBJECT_MMIO,GHOSTOS_OBJECT_MEMORY_REGION,
    GHOSTOS_OBJECT_ADDRESS_SPACE,GHOSTOS_OBJECT_THREAD,GHOSTOS_OBJECT_SYSTEM_CONTROL,
    GHOSTOS_OBJECT_NETWORK_DIAGNOSTIC,GHOSTOS_OBJECT_DMA_DEVICE,GHOSTOS_OBJECT_IPC_CHANNEL,
    GHOSTOS_OBJECT_DISTRIBUTED_RESOURCE,GHOSTOS_OBJECT_LOGICAL_NAMESPACE } ghostos_capability_object_kind;
typedef struct { uint64_t start,length; } ghostos_capability_range;
typedef struct { ghostos_capability_object_kind kind; uint8_t scope; uint32_t value32; uint64_t value0,value1; } ghostos_capability_object;
typedef struct { uint32_t owner; ghostos_capability_object object; uint16_t rights; bool has_parent; ghostos_capability_handle parent; bool has_backing; ghostos_capability_range backing; } ghostos_capability_info;
typedef struct { bool has_parent,has_first_child,has_next_sibling; ghostos_capability_handle parent,first_child,next_sibling; } ghostos_capability_links;
typedef enum { GHOSTOS_CAP_OK,GHOSTOS_CAP_FULL,GHOSTOS_CAP_INVALID_HANDLE,GHOSTOS_CAP_ACCESS_DENIED,
    GHOSTOS_CAP_RIGHTS_ESCALATION,GHOSTOS_CAP_EMPTY_RIGHTS,GHOSTOS_CAP_INVALID_QUOTA } ghostos_capability_error;
typedef enum { GHOSTOS_QUOTA_IPC_MESSAGES,GHOSTOS_QUOTA_PAGE_FAULTS,GHOSTOS_QUOTA_MEMORY_BYTES } ghostos_capability_quota_resource;
typedef enum { GHOSTOS_CAP_QUOTA_ALLOWED,GHOSTOS_CAP_QUOTA_THROTTLED,GHOSTOS_CAP_QUOTA_REJECTED } ghostos_capability_quota_decision;
typedef struct { uint64_t capacity,refill_per_second; } ghostos_quota_bucket_config;
typedef struct { ghostos_quota_bucket_config ipc,page_fault,memory; uint64_t max_memory_bytes; } ghostos_quota_policy;
typedef struct { uint64_t retry_after_us; ghostos_capability_quota_decision decision; } ghostos_quota_result;
typedef struct { uint64_t memory_in_use,max_memory_bytes; } ghostos_quota_usage;
typedef struct { uint64_t tokens,last_refill_us,remainder; } ghostos_quota_bucket_state;
typedef struct {
    ghostos_quota_policy policy;
    atomic_flag locks[3];
    atomic_uint_fast64_t clock,memory_in_use;
    ghostos_quota_bucket_state buckets[3];
} ghostos_capability_quota;
typedef struct {
    uint32_t generation;
    bool occupied;
    ghostos_capability_info info;
    uint32_t parent_slot,first_child,next_sibling;
} ghostos_capability_descriptor;
typedef void (*ghostos_capability_revoke_hook)(void *context,ghostos_capability_info info);
typedef void (*ghostos_capability_event_hook)(void *context,uint8_t level,uint16_t operation,uint64_t handle,uint32_t owner,uint64_t value,ghostos_status status,bool trace);
typedef bool (*ghostos_capability_entry_callback)(void *context,ghostos_capability_handle handle,ghostos_capability_info info);
typedef struct {
    size_t capacity;
    ghostos_capability_descriptor entries[GHOSTOS_MAX_CAPABILITIES];
    ghostos_capability_quota quotas[GHOSTOS_MAX_CAPABILITIES];
    ghostos_capability_event_hook event_hook;
    void *event_context;
} ghostos_capability_space;

enum { GHOSTOS_RIGHT_READ=1u<<0,GHOSTOS_RIGHT_WRITE=1u<<1,GHOSTOS_RIGHT_EXECUTE=1u<<2,GHOSTOS_RIGHT_MAP=1u<<3,
    GHOSTOS_RIGHT_CREATE=1u<<4,GHOSTOS_RIGHT_SEND=1u<<5,GHOSTOS_RIGHT_RECEIVE=1u<<6,GHOSTOS_RIGHT_DELEGATE=1u<<7,
    GHOSTOS_RIGHT_REVOKE=1u<<8,GHOSTOS_RIGHT_CONTROL=1u<<9,GHOSTOS_RIGHT_DMA_READ=1u<<10,GHOSTOS_RIGHT_DMA_WRITE=1u<<11,
    GHOSTOS_RIGHT_DEBUG=1u<<12,GHOSTOS_RIGHT_ALL=(1u<<13)-1 };

bool ghostos_capability_handle_from_raw(uint64_t raw,ghostos_capability_handle *out);
bool ghostos_capability_rights_valid(uint16_t rights);
bool ghostos_capability_range_new(uint64_t start,uint64_t length,ghostos_capability_range *out);
bool ghostos_capability_range_contains(ghostos_capability_range outer,ghostos_capability_range inner);
bool ghostos_capability_range_overlaps(ghostos_capability_range a,ghostos_capability_range b);
ghostos_capability_object ghostos_capability_object_make(ghostos_capability_object_kind kind,uint8_t scope,uint32_t value32,uint64_t value0,uint64_t value1);
bool ghostos_capability_object_equal(ghostos_capability_object a,ghostos_capability_object b);
void ghostos_capability_space_init(ghostos_capability_space *space,size_t capacity,ghostos_capability_event_hook events,void *event_context);
size_t ghostos_capability_capacity(const ghostos_capability_space *space);
ghostos_capability_error ghostos_capability_mint_root(ghostos_capability_space *space,uint32_t owner,ghostos_capability_object object,uint16_t rights,ghostos_capability_handle *out);
ghostos_capability_error ghostos_capability_mint_untyped(ghostos_capability_space *space,uint32_t owner,ghostos_capability_range range,uint16_t rights,ghostos_capability_handle *out);
ghostos_capability_error ghostos_capability_mint_mmio(ghostos_capability_space *space,uint32_t owner,ghostos_capability_range range,uint16_t rights,ghostos_capability_handle *out);
ghostos_capability_error ghostos_capability_retype_memory(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle source,uint32_t new_owner,uint32_t region,ghostos_capability_range range,uint16_t rights,ghostos_capability_handle *out);
ghostos_capability_error ghostos_capability_delegate(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle source,uint32_t new_owner,uint16_t rights,ghostos_capability_handle *out);
ghostos_capability_error ghostos_capability_drop_rights(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,uint16_t rights,uint16_t *remaining);
ghostos_capability_error ghostos_capability_authorize(const ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_capability_object object,uint16_t required);
ghostos_capability_error ghostos_capability_authorize_mapping(const ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,uint32_t region,bool writable,bool executable);
ghostos_capability_error ghostos_capability_inspect(const ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_capability_info *out);
bool ghostos_quota_policy_valid(ghostos_quota_policy policy);
ghostos_quota_policy ghostos_quota_policy_default(void);
ghostos_capability_error ghostos_capability_configure_quota(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_quota_policy policy);
ghostos_capability_error ghostos_capability_quota_policy(const ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_quota_policy *out);
ghostos_capability_error ghostos_capability_quota_usage(const ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_quota_usage *out);
ghostos_capability_error ghostos_capability_consume_quota(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_capability_quota_resource resource,uint64_t now_us,uint64_t amount,ghostos_quota_result *out);
ghostos_capability_error ghostos_capability_refund_quota(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_capability_quota_resource resource,uint64_t amount);
ghostos_capability_error ghostos_capability_release_memory(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,uint64_t amount);
ghostos_capability_error ghostos_capability_get_links(const ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,ghostos_capability_links *out);
ghostos_capability_error ghostos_capability_revoke(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle authority,size_t *revoked);
ghostos_capability_error ghostos_capability_revoke_with_hook(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle authority,ghostos_capability_revoke_hook hook,void *context,size_t *revoked);
ghostos_capability_error ghostos_capability_revoke_remote_memory(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle authority,uint64_t resource,size_t *revoked);
ghostos_capability_error ghostos_capability_delete(ghostos_capability_space *space,uint32_t caller,ghostos_capability_handle handle,size_t *revoked);
size_t ghostos_capability_used(const ghostos_capability_space *space);
size_t ghostos_capability_entries(const ghostos_capability_space *space,ghostos_capability_entry_callback callback,void *context);
bool ghostos_capability_check_invariants(const ghostos_capability_space *space);

#endif
