#include "ghostos/vm_cluster.h"
#include <string.h>

static bool same(ghostos_cluster_node_id a, ghostos_cluster_node_id b) { return a.raw == b.raw; }
static bool pair_same(ghostos_cluster_node_id a, ghostos_cluster_node_id b,
    ghostos_cluster_node_id c, ghostos_cluster_node_id d) {
    return (same(a, c) && same(b, d)) || (same(a, d) && same(b, c));
}
static ghostos_cluster_network_outcome result(ghostos_cluster_network_result kind) {
    ghostos_cluster_network_outcome out = { kind, 0, 0 };
    return out;
}
static bool partitioned(const ghostos_cluster_network *n, ghostos_cluster_node_id a, ghostos_cluster_node_id b) {
    for (size_t i = 0; i < n->partition_count; ++i)
        if (pair_same(a, b, n->partitions[i].left, n->partitions[i].right)) return true;
    return false;
}
static void trace(ghostos_cluster_network *n, ghostos_cluster_node_id src,
    ghostos_cluster_node_id dst, ghostos_cluster_network_outcome outcome) {
    if (n->trace_count < GHOSTOS_CLUSTER_MAX_TRACE)
        n->trace[n->trace_count++] = (ghostos_cluster_network_trace){n->tick, src, dst, outcome};
}
static uint8_t sample(ghostos_cluster_network *n) {
    n->random_state ^= n->random_state << 7;
    n->random_state ^= n->random_state >> 9;
    n->random_state ^= n->random_state << 8;
    return (uint8_t)(n->random_state % 100);
}

bool ghostos_cluster_node_id_from_raw(uint32_t raw, ghostos_cluster_node_id *out) {
    if (!out || raw == 0 || raw > GHOSTOS_CLUSTER_MAX_NODES) return false;
    out->raw = raw;
    return true;
}
ghostos_cluster_network_config ghostos_cluster_network_config_default(void) {
    ghostos_cluster_network_config c = {1, 0, 0, false, UINT64_C(0x53594e4f53434c55)};
    return c;
}
ghostos_cluster_error ghostos_cluster_network_init(ghostos_cluster_network *n, ghostos_cluster_network_config c) {
    if (!n) return GHOSTOS_CLUSTER_INVALID_CONFIG;
    if (c.loss_percent > 100 || c.duplicate_percent > 100) return GHOSTOS_CLUSTER_INVALID_CONFIG;
    memset(n, 0, sizeof(*n)); n->config = c; n->random_state = c.seed ? c.seed : 1;
    return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_network_outcome ghostos_cluster_network_send(ghostos_cluster_network *n,
    ghostos_cluster_node_id src, ghostos_cluster_node_id dst, const uint8_t *data, size_t length) {
    if (!n || length > sizeof(n->in_flight[0].payload) || (length && !data)) return result(GHOSTOS_CLUSTER_DROPPED);
    if (partitioned(n, src, dst)) { ghostos_cluster_network_outcome o = result(GHOSTOS_CLUSTER_PARTITIONED); trace(n, src, dst, o); return o; }
    if (sample(n) < n->config.loss_percent) { ghostos_cluster_network_outcome o = result(GHOSTOS_CLUSTER_DROPPED); trace(n, src, dst, o); return o; }
    uint8_t copies = sample(n) < n->config.duplicate_percent ? 2 : 1;
    size_t free_count = 0;
    for (size_t i = 0; i < GHOSTOS_CLUSTER_MAX_PACKETS; ++i) if (!n->in_flight[i].used) ++free_count;
    if (free_count < copies) { ghostos_cluster_network_outcome o = result(GHOSTOS_CLUSTER_DROPPED); trace(n, src, dst, o); return o; }
    n->next_sequence++;
    uint64_t when = UINT64_MAX - n->tick < n->config.latency_ticks ? UINT64_MAX : n->tick + n->config.latency_ticks;
    for (uint8_t c = 0; c < copies; ++c) for (size_t i = 0; i < GHOSTOS_CLUSTER_MAX_PACKETS; ++i) {
        ghostos_cluster_packet *p = &n->in_flight[i];
        if (!p->used) { p->used = true; p->source = src; p->target = dst; p->length = length;
            p->deliver_at = when; p->sequence = n->next_sequence; if (length) memcpy(p->payload, data, length); break; }
    }
    ghostos_cluster_network_outcome o = {GHOSTOS_CLUSTER_QUEUED, copies, when}; trace(n, src, dst, o); return o;
}
void ghostos_cluster_network_advance(ghostos_cluster_network *n, uint64_t ticks) {
    if (!n) return;
    while (ticks--) {
        if (n->tick != UINT64_MAX) ++n->tick;
        size_t ready_start = n->delivered_count;
        for (size_t i = 0; i < GHOSTOS_CLUSTER_MAX_PACKETS; ++i) {
            ghostos_cluster_packet *p = &n->in_flight[i];
            if (!p->used || p->deliver_at > n->tick || n->delivered_count == GHOSTOS_CLUSTER_MAX_PACKETS) continue;
            size_t j = n->delivered_count++;
            while (j && n->delivered[j - 1].sequence > p->sequence) { n->delivered[j] = n->delivered[j - 1]; --j; }
            n->delivered[j] = *p; p->used = false;
        }
        if (n->config.reorder && !(n->tick & 1))
            for (size_t i = 0, count = n->delivered_count - ready_start; i < count / 2; ++i) { ghostos_cluster_packet t=n->delivered[ready_start+i]; n->delivered[ready_start+i]=n->delivered[ready_start+count-1-i]; n->delivered[ready_start+count-1-i]=t; }
    }
}
size_t ghostos_cluster_network_receive(ghostos_cluster_network *n, ghostos_cluster_node_id target,
    ghostos_cluster_packet *out, size_t capacity) {
    if (!n || (!out && capacity)) return 0;
    size_t written=0, kept=0;
    for (size_t i=0; i<n->delivered_count; ++i) {
        if (same(n->delivered[i].target,target) && written<capacity) out[written++]=n->delivered[i];
        else n->delivered[kept++]=n->delivered[i];
    }
    n->delivered_count=kept; return written;
}
void ghostos_cluster_network_partition(ghostos_cluster_network *n, ghostos_cluster_node_id a, ghostos_cluster_node_id b) {
    if (!n || partitioned(n,a,b) || n->partition_count==GHOSTOS_CLUSTER_MAX_PARTITIONS) return;
    n->partitions[n->partition_count].left=a; n->partitions[n->partition_count++].right=b;
}
void ghostos_cluster_network_reconnect(ghostos_cluster_network *n, ghostos_cluster_node_id a, ghostos_cluster_node_id b) {
    if (!n) return; size_t kept=0;
    for (size_t i=0;i<n->partition_count;++i) if (!pair_same(a,b,n->partitions[i].left,n->partitions[i].right)) n->partitions[kept++]=n->partitions[i];
    n->partition_count=kept;
}
void ghostos_cluster_network_clear_trace(ghostos_cluster_network *n) { if (n) n->trace_count=0; }
size_t ghostos_cluster_network_pending(const ghostos_cluster_network *n) {
    if (!n) return 0; size_t count=n->delivered_count;
    for(size_t i=0;i<GHOSTOS_CLUSTER_MAX_PACKETS;++i) count += n->in_flight[i].used; return count;
}

ghostos_cluster_error ghostos_cluster_shared_init(ghostos_cluster_shared_memory *m, uint8_t *storage, size_t size) {
    if (!m || !storage || !size || size%GHOSTOS_CLUSTER_PAGE_SIZE || size>GHOSTOS_CLUSTER_MAX_SHARED_MEMORY) return GHOSTOS_CLUSTER_INVALID_SHARED_SIZE;
    memset(m,0,sizeof(*m)); m->bytes=storage; m->size=size; m->epoch=1; m->present=true; memset(storage,0,size); return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_shared_device ghostos_cluster_shared_discover(const ghostos_cluster_shared_memory *m) {
    ghostos_cluster_shared_device d={m?m->size:0,m?m->epoch:0,m&&m->present}; return d;
}
static bool contains(const ghostos_cluster_node_id *ids,size_t count,ghostos_cluster_node_id id) { for(size_t i=0;i<count;++i) if(same(ids[i],id)) return true; return false; }
void ghostos_cluster_shared_register(ghostos_cluster_shared_memory *m, ghostos_cluster_node_id id) {
    if(!m) return; if(!contains(m->nodes,m->node_count,id)&&m->node_count<GHOSTOS_CLUSTER_MAX_NODES)m->nodes[m->node_count++]=id; ghostos_cluster_shared_restore_node(m,id);
}
void ghostos_cluster_shared_fail(ghostos_cluster_shared_memory *m, ghostos_cluster_node_id id) {
    if(m&&!contains(m->failed,m->failed_count,id)&&m->failed_count<GHOSTOS_CLUSTER_MAX_NODES)m->failed[m->failed_count++]=id;
}
void ghostos_cluster_shared_restore_node(ghostos_cluster_shared_memory *m, ghostos_cluster_node_id id) {
    if(!m)return; size_t k=0; for(size_t i=0;i<m->failed_count;++i)if(!same(m->failed[i],id))m->failed[k++]=m->failed[i];m->failed_count=k;
}
static ghostos_cluster_error check_shared(const ghostos_cluster_shared_memory *m, ghostos_cluster_node_id id,size_t off,size_t len) {
    if(!m||!m->present)return GHOSTOS_CLUSTER_SHARED_UNAVAILABLE;
    if(!contains(m->nodes,m->node_count,id))return GHOSTOS_CLUSTER_UNKNOWN_NODE;
    if(contains(m->failed,m->failed_count,id))return GHOSTOS_CLUSTER_NODE_NOT_RUNNING;
    if(off>m->size||len>m->size-off)return GHOSTOS_CLUSTER_INVALID_SHARED_RANGE; return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_error ghostos_cluster_shared_map(const ghostos_cluster_shared_memory *m,ghostos_cluster_node_id id,size_t off,size_t len,ghostos_cluster_shared_mapping *out) {
    if(!out)return GHOSTOS_CLUSTER_INVALID_SHARED_RANGE; ghostos_cluster_error e=check_shared(m,id,off,len); if(e)return e; *out=(ghostos_cluster_shared_mapping){id,off,len,m->epoch};return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_error ghostos_cluster_shared_read(const ghostos_cluster_shared_memory *m,ghostos_cluster_node_id id,size_t off,uint8_t *out,size_t len) {
    ghostos_cluster_error e=check_shared(m,id,off,len);if(e)return e;if(m->corrupted)return GHOSTOS_CLUSTER_SHARED_CORRUPT;if(len&&!out)return GHOSTOS_CLUSTER_INVALID_SHARED_RANGE;memcpy(out,m->bytes+off,len);return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_error ghostos_cluster_shared_write(ghostos_cluster_shared_memory *m,ghostos_cluster_node_id id,size_t off,const uint8_t *data,size_t len) {
    ghostos_cluster_error e=check_shared(m,id,off,len);if(e)return e;if(m->corrupted)return GHOSTOS_CLUSTER_SHARED_CORRUPT;if(len&&!data)return GHOSTOS_CLUSTER_INVALID_SHARED_RANGE;memcpy(m->bytes+off,data,len);return GHOSTOS_CLUSTER_OK;
}
void ghostos_cluster_shared_hot_remove(ghostos_cluster_shared_memory *m){if(m){m->present=false;if(m->epoch!=UINT64_MAX)++m->epoch;}}
void ghostos_cluster_shared_restore(ghostos_cluster_shared_memory *m){if(m){m->present=true;m->corrupted=false;if(m->epoch!=UINT64_MAX)++m->epoch;}}
void ghostos_cluster_shared_corrupt(ghostos_cluster_shared_memory *m){if(m)m->corrupted=true;}
void ghostos_cluster_shared_repair(ghostos_cluster_shared_memory *m){if(m){m->corrupted=false;if(m->epoch!=UINT64_MAX)++m->epoch;}}

ghostos_cluster_error ghostos_vm_cluster_init(ghostos_vm_cluster *c,ghostos_cluster_network_config config,uint8_t *storage,size_t size) {
    if(!c)return GHOSTOS_CLUSTER_INVALID_CONFIG;memset(c,0,sizeof(*c));ghostos_cluster_error e=ghostos_cluster_network_init(&c->network,config);if(e)return e;
    e=ghostos_cluster_shared_init(&c->shared_memory,storage,size);if(e)return e;c->epoch=1;return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_node *ghostos_vm_cluster_node(ghostos_vm_cluster *c,ghostos_cluster_node_id id){if(!c)return NULL;for(size_t i=0;i<c->node_count;++i)if(same(c->nodes[i].id,id))return &c->nodes[i];return NULL;}
ghostos_cluster_error ghostos_vm_cluster_add_node(ghostos_vm_cluster *c,ghostos_cluster_node_id id,void *ctx,ghostos_cluster_error(*run)(void*,uint64_t,uint64_t*)) {
    if(!c)return GHOSTOS_CLUSTER_UNKNOWN_NODE;if(ghostos_vm_cluster_node(c,id))return GHOSTOS_CLUSTER_DUPLICATE_NODE;if(c->node_count==GHOSTOS_CLUSTER_MAX_NODES)return GHOSTOS_CLUSTER_CAPACITY;
    c->nodes[c->node_count++]=(ghostos_cluster_node){true,id,GHOSTOS_CLUSTER_RUNNING,0,0,ctx,run};ghostos_cluster_shared_register(&c->shared_memory,id);return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_status ghostos_vm_cluster_status(const ghostos_vm_cluster *c){ghostos_cluster_status s={0,0,0,false};if(!c)return s;s.epoch=c->epoch;s.members=c->node_count;for(size_t i=0;i<c->node_count;++i)s.running+=c->nodes[i].state==GHOSTOS_CLUSTER_RUNNING;s.quorum=s.running>=s.members/2+1;return s;}
ghostos_cluster_error ghostos_vm_cluster_heartbeat(ghostos_vm_cluster *c,ghostos_cluster_node_id id,ghostos_cluster_heartbeat *out){ghostos_cluster_node*n=ghostos_vm_cluster_node(c,id);if(!n)return GHOSTOS_CLUSTER_UNKNOWN_NODE;if(n->state!=GHOSTOS_CLUSTER_RUNNING)return GHOSTOS_CLUSTER_NODE_NOT_RUNNING;if(!out)return GHOSTOS_CLUSTER_INVALID_CONFIG;if(n->heartbeat_sequence!=UINT64_MAX)++n->heartbeat_sequence;*out=(ghostos_cluster_heartbeat){id,n->heartbeat_sequence,c->epoch,c->network.tick};return GHOSTOS_CLUSTER_OK;}
ghostos_cluster_error ghostos_vm_cluster_observe_heartbeat(ghostos_vm_cluster*c,ghostos_cluster_node_id target,ghostos_cluster_heartbeat h){ghostos_cluster_node*n=ghostos_vm_cluster_node(c,target);if(!n)return GHOSTOS_CLUSTER_UNKNOWN_NODE;if(n->state!=GHOSTOS_CLUSTER_RUNNING)return GHOSTOS_CLUSTER_NODE_NOT_RUNNING;if(h.epoch!=c->epoch)return GHOSTOS_CLUSTER_STALE_EPOCH;for(size_t i=0;i<c->observed_count;++i)if(same(c->observed[i].target,target)&&same(c->observed[i].source,h.source)){if(h.sequence>c->observed[i].sequence)c->observed[i].sequence=h.sequence;return GHOSTOS_CLUSTER_OK;}if(c->observed_count==sizeof(c->observed)/sizeof(c->observed[0]))return GHOSTOS_CLUSTER_CAPACITY;c->observed[c->observed_count].target=target;c->observed[c->observed_count].source=h.source;c->observed[c->observed_count++].sequence=h.sequence;return GHOSTOS_CLUSTER_OK;}
ghostos_cluster_error ghostos_vm_cluster_run_node(ghostos_vm_cluster*c,ghostos_cluster_node_id id,uint64_t steps,uint64_t*out){ghostos_cluster_node*n=ghostos_vm_cluster_node(c,id);if(!n)return GHOSTOS_CLUSTER_UNKNOWN_NODE;if(n->state!=GHOSTOS_CLUSTER_RUNNING)return GHOSTOS_CLUSTER_NODE_NOT_RUNNING;if(!n->run_vm)return GHOSTOS_CLUSTER_VM_ERROR;uint64_t done=0;ghostos_cluster_error e=n->run_vm(n->vm_context,steps,&done);if(e)return e;n->executed_steps=UINT64_MAX-n->executed_steps<done?UINT64_MAX:n->executed_steps+done;if(out)*out=done;return GHOSTOS_CLUSTER_OK;}
ghostos_cluster_error ghostos_vm_cluster_send(ghostos_vm_cluster*c,ghostos_cluster_node_id src,ghostos_cluster_node_id dst,const uint8_t*p,size_t len,ghostos_cluster_network_outcome*out){ghostos_cluster_node*a=ghostos_vm_cluster_node(c,src),*b=ghostos_vm_cluster_node(c,dst);if(!a||!b)return GHOSTOS_CLUSTER_UNKNOWN_NODE;if(a->state!=GHOSTOS_CLUSTER_RUNNING)return GHOSTOS_CLUSTER_NODE_NOT_RUNNING;if(b->state!=GHOSTOS_CLUSTER_RUNNING)return GHOSTOS_CLUSTER_NODE_NOT_RUNNING;if(out)*out=ghostos_cluster_network_send(&c->network,src,dst,p,len);return GHOSTOS_CLUSTER_OK;}
/* fault kind: 0 partition, 1 reconnect, 2 isolate, 3 fail, 4 recover, 5 kill-during */
ghostos_cluster_error ghostos_vm_cluster_fault(ghostos_vm_cluster*c,unsigned kind,ghostos_cluster_node_id a,ghostos_cluster_node_id b,ghostos_cluster_workload workload){
    if(!c)return GHOSTOS_CLUSTER_UNKNOWN_NODE;ghostos_cluster_node*n;
    if(kind<=1){if(!ghostos_vm_cluster_node(c,a)||!ghostos_vm_cluster_node(c,b))return GHOSTOS_CLUSTER_UNKNOWN_NODE;if(kind==0)ghostos_cluster_network_partition(&c->network,a,b);else ghostos_cluster_network_reconnect(&c->network,a,b);return GHOSTOS_CLUSTER_OK;}
    n=ghostos_vm_cluster_node(c,a);if(!n)return GHOSTOS_CLUSTER_UNKNOWN_NODE;
    if(kind==2)n->state=GHOSTOS_CLUSTER_ISOLATED;
    else if(kind==3||kind==5){n->state=GHOSTOS_CLUSTER_FAILED;ghostos_cluster_shared_fail(&c->shared_memory,a);if(c->epoch!=UINT64_MAX)++c->epoch;if(kind==5){if(c->fault_count==GHOSTOS_CLUSTER_MAX_FAULTS)return GHOSTOS_CLUSTER_CAPACITY;c->faults[c->fault_count++]=(ghostos_cluster_fault_record){a,workload,false};}}
    else if(kind==4){n->state=GHOSTOS_CLUSTER_RUNNING;ghostos_cluster_shared_restore_node(&c->shared_memory,a);ghostos_cluster_shared_register(&c->shared_memory,a);if(c->epoch!=UINT64_MAX)++c->epoch;}
    else return GHOSTOS_CLUSTER_INVALID_CONFIG;return GHOSTOS_CLUSTER_OK;
}
ghostos_cluster_error ghostos_vm_cluster_recover_last_fault(ghostos_vm_cluster*c){if(!c||!c->fault_count)return GHOSTOS_CLUSTER_NO_FAULT;ghostos_cluster_fault_record*f=&c->faults[c->fault_count-1];if(f->recovered)return GHOSTOS_CLUSTER_OK;ghostos_cluster_node*n=ghostos_vm_cluster_node(c,f->node);if(!n)return GHOSTOS_CLUSTER_UNKNOWN_NODE;n->state=GHOSTOS_CLUSTER_RUNNING;ghostos_cluster_shared_restore_node(&c->shared_memory,f->node);ghostos_cluster_shared_register(&c->shared_memory,f->node);if(c->epoch!=UINT64_MAX)++c->epoch;f->recovered=true;return GHOSTOS_CLUSTER_OK;}
ghostos_cluster_scale_evidence ghostos_vm_cluster_scale(const ghostos_vm_cluster*c){ghostos_cluster_scale_evidence e={0};if(!c)return e;e.nodes=e.discovered_nodes=c->node_count;e.heartbeat_messages=c->network.trace_count;e.control_plane_traffic_bytes=c->node_count*sizeof(ghostos_cluster_node_id)+e.heartbeat_messages*28;e.control_plane_memory_bytes=c->node_count*sizeof(ghostos_cluster_node_id)+c->node_count*sizeof(ghostos_cluster_node)+c->observed_count*(2*sizeof(ghostos_cluster_node_id)+sizeof(uint64_t))+c->fault_count*sizeof(ghostos_cluster_fault_record);e.convergence_ticks=c->network.tick;return e;}
