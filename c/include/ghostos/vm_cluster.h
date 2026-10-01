#ifndef GHOSTOS_VM_CLUSTER_H
#define GHOSTOS_VM_CLUSTER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_CLUSTER_MAX_NODES 1000
#define GHOSTOS_CLUSTER_MAX_PACKETS 4096
#define GHOSTOS_CLUSTER_MAX_TRACE 8192
#define GHOSTOS_CLUSTER_MAX_PARTITIONS 4096
#define GHOSTOS_CLUSTER_MAX_FAULTS 1024
#define GHOSTOS_CLUSTER_MAX_SHARED_MEMORY (64u * 1024u * 1024u)
#define GHOSTOS_CLUSTER_PAGE_SIZE UINT64_C(4096)

typedef enum {
    GHOSTOS_CLUSTER_OK = 0, GHOSTOS_CLUSTER_INVALID_ID,
    GHOSTOS_CLUSTER_INVALID_CONFIG, GHOSTOS_CLUSTER_CAPACITY,
    GHOSTOS_CLUSTER_DUPLICATE_NODE, GHOSTOS_CLUSTER_UNKNOWN_NODE,
    GHOSTOS_CLUSTER_NODE_NOT_RUNNING, GHOSTOS_CLUSTER_INVALID_SHARED_SIZE,
    GHOSTOS_CLUSTER_INVALID_SHARED_RANGE, GHOSTOS_CLUSTER_SHARED_UNAVAILABLE,
    GHOSTOS_CLUSTER_SHARED_CORRUPT, GHOSTOS_CLUSTER_STALE_EPOCH,
    GHOSTOS_CLUSTER_NO_FAULT, GHOSTOS_CLUSTER_BUFFER_TOO_SMALL,
    GHOSTOS_CLUSTER_VM_ERROR
} ghostos_cluster_error;

typedef struct { uint32_t raw; } ghostos_cluster_node_id;
typedef enum { GHOSTOS_CLUSTER_RUNNING, GHOSTOS_CLUSTER_ISOLATED,
    GHOSTOS_CLUSTER_FAILED, GHOSTOS_CLUSTER_STOPPED } ghostos_cluster_node_state;
typedef struct {
    uint64_t latency_ticks;
    uint8_t loss_percent, duplicate_percent;
    bool reorder;
    uint64_t seed;
} ghostos_cluster_network_config;
typedef enum { GHOSTOS_CLUSTER_QUEUED, GHOSTOS_CLUSTER_DROPPED,
    GHOSTOS_CLUSTER_PARTITIONED } ghostos_cluster_network_result;
typedef struct {
    ghostos_cluster_network_result result;
    uint8_t copies;
    uint64_t deliver_at;
} ghostos_cluster_network_outcome;
typedef struct {
    uint64_t tick;
    ghostos_cluster_node_id source, target;
    ghostos_cluster_network_outcome outcome;
} ghostos_cluster_network_trace;
typedef struct {
    bool used;
    ghostos_cluster_node_id source, target;
    uint64_t deliver_at, sequence;
    size_t length;
    uint8_t payload[1500];
} ghostos_cluster_packet;
typedef struct {
    ghostos_cluster_network_config config;
    uint64_t tick, random_state, next_sequence;
    ghostos_cluster_packet in_flight[GHOSTOS_CLUSTER_MAX_PACKETS];
    ghostos_cluster_packet delivered[GHOSTOS_CLUSTER_MAX_PACKETS];
    size_t delivered_count;
    struct { ghostos_cluster_node_id left, right; } partitions[GHOSTOS_CLUSTER_MAX_PARTITIONS];
    size_t partition_count;
    ghostos_cluster_network_trace trace[GHOSTOS_CLUSTER_MAX_TRACE];
    size_t trace_count;
} ghostos_cluster_network;

typedef struct { size_t size; uint64_t epoch; bool present; } ghostos_cluster_shared_device;
typedef struct { ghostos_cluster_node_id node; size_t offset, length; uint64_t epoch; } ghostos_cluster_shared_mapping;
typedef struct {
    uint8_t *bytes;
    size_t size;
    uint64_t epoch;
    bool present, corrupted;
    ghostos_cluster_node_id nodes[GHOSTOS_CLUSTER_MAX_NODES];
    size_t node_count;
    ghostos_cluster_node_id failed[GHOSTOS_CLUSTER_MAX_NODES];
    size_t failed_count;
} ghostos_cluster_shared_memory;

typedef enum { GHOSTOS_CLUSTER_WORKLOAD_IPC, GHOSTOS_CLUSTER_WORKLOAD_FILESYSTEM_COMMIT,
    GHOSTOS_CLUSTER_WORKLOAD_MEMORY_FETCH, GHOSTOS_CLUSTER_WORKLOAD_INFERENCE,
    GHOSTOS_CLUSTER_WORKLOAD_MEMBERSHIP_CHANGE } ghostos_cluster_workload;
typedef struct { ghostos_cluster_node_id node; ghostos_cluster_workload workload; bool recovered; } ghostos_cluster_fault_record;
typedef struct {
    ghostos_cluster_node_id source;
    uint64_t sequence, epoch, tick;
} ghostos_cluster_heartbeat;
typedef struct { uint64_t epoch; size_t members, running; bool quorum; } ghostos_cluster_status;
typedef struct {
    size_t nodes, discovered_nodes, heartbeat_messages;
    size_t control_plane_traffic_bytes, control_plane_memory_bytes;
    uint64_t convergence_ticks;
} ghostos_cluster_scale_evidence;
typedef struct {
    bool used;
    ghostos_cluster_node_id id;
    ghostos_cluster_node_state state;
    uint64_t executed_steps, heartbeat_sequence;
    void *vm_context;
    ghostos_cluster_error (*run_vm)(void *context, uint64_t steps, uint64_t *executed);
} ghostos_cluster_node;
typedef struct {
    ghostos_cluster_node nodes[GHOSTOS_CLUSTER_MAX_NODES];
    size_t node_count;
    ghostos_cluster_network network;
    ghostos_cluster_shared_memory shared_memory;
    uint64_t epoch;
    struct { ghostos_cluster_node_id target, source; uint64_t sequence; } observed[GHOSTOS_CLUSTER_MAX_NODES * 4];
    size_t observed_count;
    ghostos_cluster_fault_record faults[GHOSTOS_CLUSTER_MAX_FAULTS];
    size_t fault_count;
} ghostos_vm_cluster;

bool ghostos_cluster_node_id_from_raw(uint32_t raw, ghostos_cluster_node_id *out);
ghostos_cluster_network_config ghostos_cluster_network_config_default(void);
ghostos_cluster_error ghostos_cluster_network_init(ghostos_cluster_network *network, ghostos_cluster_network_config config);
ghostos_cluster_network_outcome ghostos_cluster_network_send(ghostos_cluster_network *network,
    ghostos_cluster_node_id source, ghostos_cluster_node_id target, const uint8_t *payload, size_t length);
void ghostos_cluster_network_advance(ghostos_cluster_network *network, uint64_t ticks);
size_t ghostos_cluster_network_receive(ghostos_cluster_network *network, ghostos_cluster_node_id target,
    ghostos_cluster_packet *out, size_t capacity);
void ghostos_cluster_network_partition(ghostos_cluster_network *network, ghostos_cluster_node_id left, ghostos_cluster_node_id right);
void ghostos_cluster_network_reconnect(ghostos_cluster_network *network, ghostos_cluster_node_id left, ghostos_cluster_node_id right);
void ghostos_cluster_network_clear_trace(ghostos_cluster_network *network);
size_t ghostos_cluster_network_pending(const ghostos_cluster_network *network);

ghostos_cluster_error ghostos_cluster_shared_init(ghostos_cluster_shared_memory *memory, uint8_t *storage, size_t size);
ghostos_cluster_shared_device ghostos_cluster_shared_discover(const ghostos_cluster_shared_memory *memory);
void ghostos_cluster_shared_register(ghostos_cluster_shared_memory *memory, ghostos_cluster_node_id node);
void ghostos_cluster_shared_fail(ghostos_cluster_shared_memory *memory, ghostos_cluster_node_id node);
void ghostos_cluster_shared_restore_node(ghostos_cluster_shared_memory *memory, ghostos_cluster_node_id node);
ghostos_cluster_error ghostos_cluster_shared_map(const ghostos_cluster_shared_memory *memory,
    ghostos_cluster_node_id node, size_t offset, size_t length, ghostos_cluster_shared_mapping *out);
ghostos_cluster_error ghostos_cluster_shared_read(const ghostos_cluster_shared_memory *memory,
    ghostos_cluster_node_id node, size_t offset, uint8_t *out, size_t length);
ghostos_cluster_error ghostos_cluster_shared_write(ghostos_cluster_shared_memory *memory,
    ghostos_cluster_node_id node, size_t offset, const uint8_t *data, size_t length);
void ghostos_cluster_shared_hot_remove(ghostos_cluster_shared_memory *memory);
void ghostos_cluster_shared_restore(ghostos_cluster_shared_memory *memory);
void ghostos_cluster_shared_corrupt(ghostos_cluster_shared_memory *memory);
void ghostos_cluster_shared_repair(ghostos_cluster_shared_memory *memory);

ghostos_cluster_error ghostos_vm_cluster_init(ghostos_vm_cluster *cluster, ghostos_cluster_network_config config,
    uint8_t *shared_storage, size_t shared_size);
ghostos_cluster_error ghostos_vm_cluster_add_node(ghostos_vm_cluster *cluster, ghostos_cluster_node_id id,
    void *vm_context, ghostos_cluster_error (*run_vm)(void *, uint64_t, uint64_t *));
ghostos_cluster_node *ghostos_vm_cluster_node(ghostos_vm_cluster *cluster, ghostos_cluster_node_id id);
ghostos_cluster_status ghostos_vm_cluster_status(const ghostos_vm_cluster *cluster);
ghostos_cluster_error ghostos_vm_cluster_heartbeat(ghostos_vm_cluster *cluster, ghostos_cluster_node_id source,
    ghostos_cluster_heartbeat *out);
ghostos_cluster_error ghostos_vm_cluster_observe_heartbeat(ghostos_vm_cluster *cluster,
    ghostos_cluster_node_id target, ghostos_cluster_heartbeat heartbeat);
ghostos_cluster_error ghostos_vm_cluster_run_node(ghostos_vm_cluster *cluster,
    ghostos_cluster_node_id id, uint64_t steps, uint64_t *executed);
ghostos_cluster_error ghostos_vm_cluster_send(ghostos_vm_cluster *cluster, ghostos_cluster_node_id source,
    ghostos_cluster_node_id target, const uint8_t *payload, size_t length, ghostos_cluster_network_outcome *out);
ghostos_cluster_error ghostos_vm_cluster_fault(ghostos_vm_cluster *cluster, unsigned kind,
    ghostos_cluster_node_id left, ghostos_cluster_node_id right, ghostos_cluster_workload workload);
ghostos_cluster_error ghostos_vm_cluster_recover_last_fault(ghostos_vm_cluster *cluster);
ghostos_cluster_scale_evidence ghostos_vm_cluster_scale(const ghostos_vm_cluster *cluster);

#endif
