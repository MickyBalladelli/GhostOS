#include "ghostos/cpu_topology.h"

bool ghostos_cpu_id(uint8_t raw, uint8_t *cpu) {
    if (raw >= GHOSTOS_MAX_CPUS) return false;
    if (cpu) *cpu = raw;
    return true;
}

ghostos_cpu_mask ghostos_cpu_mask_empty(void) { return (ghostos_cpu_mask){{0,0}}; }
ghostos_cpu_mask ghostos_cpu_mask_from_cpu(uint8_t cpu) {
    ghostos_cpu_mask m = {{0,0}};
    if (cpu < GHOSTOS_MAX_CPUS) m.words[cpu / 64] = UINT64_C(1) << (cpu % 64);
    return m;
}
ghostos_cpu_mask ghostos_cpu_mask_union(ghostos_cpu_mask a, ghostos_cpu_mask b) { return (ghostos_cpu_mask){{a.words[0]|b.words[0],a.words[1]|b.words[1]}}; }
ghostos_cpu_mask ghostos_cpu_mask_difference(ghostos_cpu_mask a, ghostos_cpu_mask b) { return (ghostos_cpu_mask){{a.words[0]&~b.words[0],a.words[1]&~b.words[1]}}; }
bool ghostos_cpu_mask_contains(ghostos_cpu_mask m,uint8_t cpu) { return cpu<GHOSTOS_MAX_CPUS&&(m.words[cpu/64]&(UINT64_C(1)<<(cpu%64)))!=0; }
bool ghostos_cpu_mask_intersects(ghostos_cpu_mask a,ghostos_cpu_mask b) { return (a.words[0]&b.words[0])||(a.words[1]&b.words[1]); }
bool ghostos_cpu_mask_is_empty(ghostos_cpu_mask m) { return m.words[0]==0&&m.words[1]==0; }

void ghostos_cpu_topology_init(ghostos_cpu_topology *t,size_t capacity) {
    t->capacity=capacity<GHOSTOS_MAX_CPUS?capacity:GHOSTOS_MAX_CPUS;
    t->count=0;
    for(size_t i=0;i<GHOSTOS_MAX_CPUS;i++)t->records[i]=(ghostos_cpu_record){0,0,GHOSTOS_CPU_ABSENT};
}

static bool has_hardware_id(const ghostos_cpu_topology*t,uint32_t hardware_id) {
    for(size_t i=0;i<t->count;i++)if(t->records[i].state!=GHOSTOS_CPU_ABSENT&&t->records[i].hardware_id==hardware_id)return true;
    return false;
}

static ghostos_cpu_topology_error add_cpu(ghostos_cpu_topology*t,uint32_t hardware_id,ghostos_cpu_startup_state state,uint8_t*cpu) {
    if(has_hardware_id(t,hardware_id))return GHOSTOS_CPU_DUPLICATE_HARDWARE_ID;
    if(t->count>=t->capacity||t->count>=GHOSTOS_MAX_CPUS)return GHOSTOS_CPU_CAPACITY;
    uint8_t id=(uint8_t)t->count;
    t->records[t->count]=(ghostos_cpu_record){id,hardware_id,state};
    ++t->count;
    if(cpu)*cpu=id;
    return GHOSTOS_CPU_TOPOLOGY_OK;
}

ghostos_cpu_topology_error ghostos_cpu_topology_add_bootstrap(ghostos_cpu_topology*t,uint32_t hardware_id,uint8_t*cpu) {
    return add_cpu(t,hardware_id,GHOSTOS_CPU_STARTED,cpu);
}

ghostos_cpu_topology_error ghostos_cpu_topology_request(ghostos_cpu_topology*t,uint32_t hardware_id,uint8_t*cpu) {
    /* Rust's unwrap_or eagerly evaluates add(), so an already known hardware ID
       returns DuplicateHardwareId before its record can be transitioned. */
    if(has_hardware_id(t,hardware_id))return GHOSTOS_CPU_DUPLICATE_HARDWARE_ID;
    uint8_t id;
    ghostos_cpu_topology_error e=add_cpu(t,hardware_id,GHOSTOS_CPU_ABSENT,&id);
    if(e!=GHOSTOS_CPU_TOPOLOGY_OK)return e;
    ghostos_cpu_record*r=&t->records[id];
    if(r->state!=GHOSTOS_CPU_ABSENT)return GHOSTOS_CPU_INVALID_TRANSITION;
    r->state=GHOSTOS_CPU_REQUESTED;
    if(cpu)*cpu=id;
    return GHOSTOS_CPU_TOPOLOGY_OK;
}

static ghostos_cpu_topology_error get_record_mut(ghostos_cpu_topology*t,uint8_t cpu,ghostos_cpu_record**out) {
    if(cpu>=t->count||t->records[cpu].state==GHOSTOS_CPU_ABSENT)return GHOSTOS_CPU_INVALID;
    *out=&t->records[cpu];
    return GHOSTOS_CPU_TOPOLOGY_OK;
}

ghostos_cpu_topology_error ghostos_cpu_topology_mark_started(ghostos_cpu_topology*t,uint8_t cpu) {
    ghostos_cpu_record*r;ghostos_cpu_topology_error e=get_record_mut(t,cpu,&r);if(e)return e;
    if(r->state!=GHOSTOS_CPU_REQUESTED&&r->state!=GHOSTOS_CPU_STARTED)return GHOSTOS_CPU_INVALID_TRANSITION;
    r->state=GHOSTOS_CPU_STARTED;return GHOSTOS_CPU_TOPOLOGY_OK;
}
ghostos_cpu_topology_error ghostos_cpu_topology_mark_failed(ghostos_cpu_topology*t,uint8_t cpu) {
    ghostos_cpu_record*r;ghostos_cpu_topology_error e=get_record_mut(t,cpu,&r);if(e)return e;
    if(r->state!=GHOSTOS_CPU_REQUESTED)return GHOSTOS_CPU_INVALID_TRANSITION;
    r->state=GHOSTOS_CPU_FAILED;return GHOSTOS_CPU_TOPOLOGY_OK;
}
ghostos_cpu_topology_error ghostos_cpu_topology_record(const ghostos_cpu_topology*t,uint8_t cpu,ghostos_cpu_record*out) {
    if(cpu>=t->count||t->records[cpu].state==GHOSTOS_CPU_ABSENT)return GHOSTOS_CPU_INVALID;
    if(out)*out=t->records[cpu];return GHOSTOS_CPU_TOPOLOGY_OK;
}
ghostos_cpu_mask ghostos_cpu_topology_online(const ghostos_cpu_topology*t) {
    ghostos_cpu_mask m={{0,0}};for(size_t i=0;i<t->count;i++)if(t->records[i].state==GHOSTOS_CPU_STARTED)m=ghostos_cpu_mask_union(m,ghostos_cpu_mask_from_cpu(t->records[i].id));return m;
}
size_t ghostos_cpu_topology_online_count(const ghostos_cpu_topology*t) {
    size_t n=0;for(size_t i=0;i<t->count;i++)if(t->records[i].state==GHOSTOS_CPU_STARTED)++n;return n;
}
size_t ghostos_cpu_topology_records(const ghostos_cpu_topology*t,ghostos_cpu_record*out,size_t capacity) {
    size_t n=0;for(size_t i=0;i<t->count;i++)if(t->records[i].state!=GHOSTOS_CPU_ABSENT){if(out&&n<capacity)out[n]=t->records[i];++n;}return n;
}

void ghostos_per_cpu_interrupt_init(ghostos_per_cpu_interrupt_state*s,uint8_t cpu) {
    s->cpu=cpu;s->interrupt_depth=0;s->enabled=false;s->isolated=false;s->pending_ipi=ghostos_cpu_mask_empty();
}
void ghostos_per_cpu_interrupt_enter(ghostos_per_cpu_interrupt_state*s) {
    if(s->interrupt_depth<UINT16_MAX)++s->interrupt_depth;s->enabled=false;
}
void ghostos_per_cpu_interrupt_exit(ghostos_per_cpu_interrupt_state*s) {
    if(s->interrupt_depth>0)--s->interrupt_depth;s->enabled=s->interrupt_depth==0;
}
void ghostos_per_cpu_queue_ipi(ghostos_per_cpu_interrupt_state*s,uint8_t source) {
    s->pending_ipi=ghostos_cpu_mask_union(s->pending_ipi,ghostos_cpu_mask_from_cpu(source));
}
bool ghostos_per_cpu_take_ipi(ghostos_per_cpu_interrupt_state*s,uint8_t source) {
    if(!ghostos_cpu_mask_contains(s->pending_ipi,source))return false;
    s->pending_ipi=ghostos_cpu_mask_difference(s->pending_ipi,ghostos_cpu_mask_from_cpu(source));return true;
}
