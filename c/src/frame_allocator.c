#include "ghostos/frame_allocator.h"

#define EARLY_ALLOCATION_FLOOR UINT64_C(16777216)

typedef struct { uint64_t start, length; } range;

static bool end_of(range r, uint64_t *end) {
    if (UINT64_MAX - r.start < r.length) return false;
    *end = r.start + r.length;
    return true;
}

static uint64_t align_up(uint64_t value) {
    if (UINT64_MAX - value < GHOSTOS_FRAME_SIZE - 1) return UINT64_MAX & ~(GHOSTOS_FRAME_SIZE - 1);
    return (value + GHOSTOS_FRAME_SIZE - 1) & ~(GHOSTOS_FRAME_SIZE - 1);
}

static bool contains(range outer, range inner) {
    uint64_t outer_end, inner_end;
    return inner.start >= outer.start && end_of(outer, &outer_end) && end_of(inner, &inner_end) && inner_end <= outer_end;
}

static bool initial_free(ghostos_early_frame_allocator *a, range r) {
    for (size_t i = 0; i < GHOSTOS_MAX_MEMORY_REGIONS; ++i) {
        if (!a->free_ranges[i].used) {
            a->free_ranges[i] = (ghostos_free_frame_range){r.start, r.length, true};
            ++a->free_count;
            return true;
        }
    }
    return false;
}

void ghostos_frame_allocator_init(ghostos_early_frame_allocator *a, const ghostos_memory_region *regions, size_t count) {
    for (size_t i = 0; i < GHOSTOS_MAX_MEMORY_REGIONS; ++i) a->free_ranges[i].used = false;
    for (size_t i = 0; i < GHOSTOS_MAX_OWNED_FRAME_RANGES; ++i) a->owned_ranges[i].used = false;
    a->free_count = 0;
    a->owned_count = 0;
    if (!regions) return;
    if (count > GHOSTOS_MAX_MEMORY_REGIONS) count = GHOSTOS_MAX_MEMORY_REGIONS;
    for (size_t i = 0; i < count; ++i) {
        const ghostos_memory_region *r = &regions[i];
        if (r->kind != GHOSTOS_MEMORY_USABLE || r->length < GHOSTOS_FRAME_SIZE || UINT64_MAX - r->start < r->length) continue;
        uint64_t end = r->start + r->length;
        uint64_t preferred = align_up(r->start > EARLY_ALLOCATION_FLOOR ? r->start : EARLY_ALLOCATION_FLOOR);
        uint64_t fallback = align_up(r->start);
        uint64_t start = preferred <= end && GHOSTOS_FRAME_SIZE <= end - preferred ? preferred : fallback;
        if (start <= end && GHOSTOS_FRAME_SIZE <= end - start) (void)initial_free(a, (range){start, end - start});
    }
}

static bool can_insert_owned(const ghostos_early_frame_allocator *a, range r, uint32_t owner) {
    uint64_t end;
    if (!end_of(r, &end)) return false;
    for (size_t i = 0; i < GHOSTOS_MAX_OWNED_FRAME_RANGES; ++i) {
        const ghostos_owned_frame_range *e = &a->owned_ranges[i];
        if (!e->used || e->owner != owner) continue;
        uint64_t e_end;
        if (end_of((range){e->start,e->length}, &e_end) && (e_end == r.start || end == e->start)) return true;
    }
    for (size_t i = 0; i < GHOSTOS_MAX_OWNED_FRAME_RANGES; ++i) if (!a->owned_ranges[i].used) return true;
    return false;
}

static void insert_owned(ghostos_early_frame_allocator *a, range r, uint32_t owner) {
    uint64_t re = r.start + r.length;
    size_t left = GHOSTOS_MAX_OWNED_FRAME_RANGES, right = left;
    for (size_t i = 0; i < GHOSTOS_MAX_OWNED_FRAME_RANGES; ++i) {
        ghostos_owned_frame_range *e = &a->owned_ranges[i];
        uint64_t ee;
        if (e->used && e->owner == owner && end_of((range){e->start,e->length}, &ee) && ee == r.start) left = i;
        if (e->used && e->owner == owner && e->start == re) right = i;
    }
    if (left != GHOSTOS_MAX_OWNED_FRAME_RANGES && right != GHOSTOS_MAX_OWNED_FRAME_RANGES && left != right) {
        uint64_t eend = a->owned_ranges[right].start + a->owned_ranges[right].length;
        a->owned_ranges[left].length = eend - a->owned_ranges[left].start;
        a->owned_ranges[right].used = false;
        --a->owned_count;
    } else if (left != GHOSTOS_MAX_OWNED_FRAME_RANGES) {
        a->owned_ranges[left].length = re - a->owned_ranges[left].start;
    } else if (right != GHOSTOS_MAX_OWNED_FRAME_RANGES) {
        a->owned_ranges[right].length += r.length;
        a->owned_ranges[right].start = r.start;
    } else {
        for (size_t i = 0; i < GHOSTOS_MAX_OWNED_FRAME_RANGES; ++i) if (!a->owned_ranges[i].used) {
            a->owned_ranges[i] = (ghostos_owned_frame_range){r.start,r.length,owner,true};
            ++a->owned_count;
            return;
        }
    }
}

ghostos_allocation_error ghostos_frame_allocate_range(ghostos_early_frame_allocator *a, uint32_t owner, size_t frame_count, ghostos_physical_range *out) {
    if (!frame_count || (uint64_t)frame_count > UINT64_MAX / GHOSTOS_FRAME_SIZE) return GHOSTOS_ALLOC_EXHAUSTED;
    uint64_t length = (uint64_t)frame_count * GHOSTOS_FRAME_SIZE;
    for (size_t i = 0; i < GHOSTOS_MAX_MEMORY_REGIONS; ++i) {
        ghostos_free_frame_range *f = &a->free_ranges[i];
        if (!f->used || f->length < length) continue;
        range allocation = {f->start,length};
        if (!can_insert_owned(a, allocation, owner)) return GHOSTOS_ALLOC_EXHAUSTED;
        if (f->length == length) { f->used = false; --a->free_count; }
        else { f->start += length; f->length -= length; }
        insert_owned(a, allocation, owner);
        if (out) *out = (ghostos_physical_range){allocation.start,allocation.length};
        return GHOSTOS_ALLOC_OK;
    }
    return GHOSTOS_ALLOC_EXHAUSTED;
}

ghostos_allocation_error ghostos_frame_allocate_for(ghostos_early_frame_allocator *a, uint32_t owner, uint64_t *frame) {
    ghostos_physical_range r;
    if (ghostos_frame_allocate_range(a, owner, 1, &r) != GHOSTOS_ALLOC_OK) return GHOSTOS_ALLOC_EXHAUSTED;
    if (frame) *frame = r.start;
    return GHOSTOS_ALLOC_OK;
}

ghostos_allocation_error ghostos_frame_allocate(ghostos_early_frame_allocator *a, uint64_t *frame) {
    return ghostos_frame_allocate_for(a, 0, frame);
}

ghostos_quota_error ghostos_frame_allocate_quota(ghostos_early_frame_allocator *a, uint64_t now, const ghostos_frame_quota *q, uint64_t *frame, uint64_t *retry) {
    if (!q || !q->consume) return GHOSTOS_QUOTA_INVALID_CAPABILITY;
    uint64_t delay = 0;
    ghostos_quota_decision d = q->consume(q->context, 0, now, GHOSTOS_FRAME_SIZE, &delay);
    if (d == GHOSTOS_QUOTA_DECISION_THROTTLED) { if (retry) *retry = delay; return GHOSTOS_QUOTA_THROTTLED; }
    if (d == GHOSTOS_QUOTA_DECISION_REJECTED) return GHOSTOS_QUOTA_REJECTED;
    if (ghostos_frame_allocate_for(a, 0, frame) == GHOSTOS_ALLOC_OK) return GHOSTOS_QUOTA_OK;
    if (q->refund) q->refund(q->context, 0, GHOSTOS_FRAME_SIZE);
    return GHOSTOS_QUOTA_EXHAUSTED;
}

ghostos_quota_error ghostos_frame_allocate_capability(ghostos_early_frame_allocator *a, uint32_t caller, uint64_t authority, uint64_t now, const ghostos_frame_quota *q, uint64_t *frame, uint64_t *retry) {
    if (!q || !q->authorize || !q->authorize(q->context, caller, authority) || !q->consume) return GHOSTOS_QUOTA_INVALID_CAPABILITY;
    uint64_t delay = 0;
    ghostos_quota_decision d = q->consume(q->context, caller, now, GHOSTOS_FRAME_SIZE, &delay);
    if (d == GHOSTOS_QUOTA_DECISION_THROTTLED) { if (retry) *retry = delay; return GHOSTOS_QUOTA_THROTTLED; }
    if (d == GHOSTOS_QUOTA_DECISION_REJECTED) return GHOSTOS_QUOTA_REJECTED;
    if (ghostos_frame_allocate_for(a, caller, frame) == GHOSTOS_ALLOC_OK) return GHOSTOS_QUOTA_OK;
    if (q->refund) q->refund(q->context, caller, GHOSTOS_FRAME_SIZE);
    return GHOSTOS_QUOTA_EXHAUSTED;
}

static bool can_insert_free(const ghostos_early_frame_allocator *a, range r) {
    uint64_t re;
    if (!end_of(r, &re)) return false;
    for (size_t i=0;i<GHOSTOS_MAX_MEMORY_REGIONS;i++) {
        const ghostos_free_frame_range *f=&a->free_ranges[i];
        uint64_t fe;
        if(f->used&&end_of((range){f->start,f->length},&fe)&&(fe==r.start||re==f->start))return true;
    }
    for(size_t i=0;i<GHOSTOS_MAX_MEMORY_REGIONS;i++)if(!a->free_ranges[i].used)return true;
    return false;
}

static void insert_free(ghostos_early_frame_allocator *a, range r) {
    uint64_t re=r.start+r.length; size_t left=GHOSTOS_MAX_MEMORY_REGIONS,right=left;
    for(size_t i=0;i<GHOSTOS_MAX_MEMORY_REGIONS;i++){
        ghostos_free_frame_range*f=&a->free_ranges[i];uint64_t fe;
        if(f->used&&end_of((range){f->start,f->length},&fe)&&fe==r.start)left=i;
        if(f->used&&f->start==re)right=i;
    }
    if(left!=GHOSTOS_MAX_MEMORY_REGIONS&&right!=GHOSTOS_MAX_MEMORY_REGIONS&&left!=right){uint64_t e=a->free_ranges[right].start+a->free_ranges[right].length;a->free_ranges[left].length=e-a->free_ranges[left].start;a->free_ranges[right].used=false;--a->free_count;}
    else if(left!=GHOSTOS_MAX_MEMORY_REGIONS)a->free_ranges[left].length=re-a->free_ranges[left].start;
    else if(right!=GHOSTOS_MAX_MEMORY_REGIONS){uint64_t e=a->free_ranges[right].start+a->free_ranges[right].length;a->free_ranges[right].start=r.start;a->free_ranges[right].length=e-r.start;}
    else (void)initial_free(a,r);
}

ghostos_reclaim_error ghostos_frame_reclaim_range(ghostos_early_frame_allocator *a,uint32_t owner,ghostos_physical_range input){
    if(input.start%GHOSTOS_FRAME_SIZE||!input.length||input.length%GHOSTOS_FRAME_SIZE||UINT64_MAX-input.start<input.length)return GHOSTOS_RECLAIM_INVALID_RANGE;
    range r={input.start,input.length};size_t ix=GHOSTOS_MAX_OWNED_FRAME_RANGES;
    for(size_t i=0;i<GHOSTOS_MAX_OWNED_FRAME_RANGES;i++){ghostos_owned_frame_range*e=&a->owned_ranges[i];if(e->used&&e->owner==owner&&contains((range){e->start,e->length},r)){ix=i;break;}}
    if(ix==GHOSTOS_MAX_OWNED_FRAME_RANGES)return GHOSTOS_RECLAIM_NOT_OWNED;
    ghostos_owned_frame_range current=a->owned_ranges[ix];uint64_t end=current.start+current.length,re=r.start+r.length;
    bool split=r.start>current.start&&re<end;
    if(!can_insert_free(a,r))return GHOSTOS_RECLAIM_CAPACITY;
    if(split){bool empty=false;for(size_t i=0;i<GHOSTOS_MAX_OWNED_FRAME_RANGES;i++)if(!a->owned_ranges[i].used)empty=true;if(!empty)return GHOSTOS_RECLAIM_CAPACITY;}
    if(r.start==current.start&&re==end){a->owned_ranges[ix].used=false;--a->owned_count;}
    else if(r.start==current.start){a->owned_ranges[ix].start=re;a->owned_ranges[ix].length=end-re;}
    else if(re==end)a->owned_ranges[ix].length=r.start-current.start;
    else if(split){a->owned_ranges[ix].length=r.start-current.start;for(size_t i=0;i<GHOSTOS_MAX_OWNED_FRAME_RANGES;i++)if(!a->owned_ranges[i].used){a->owned_ranges[i]=(ghostos_owned_frame_range){re,end-re,owner,true};++a->owned_count;break;}}
    insert_free(a,r);return GHOSTOS_RECLAIM_OK;
}

ghostos_reclaim_error ghostos_frame_reclaim(ghostos_early_frame_allocator *a,uint32_t owner,uint64_t frame){return ghostos_frame_reclaim_range(a,owner,(ghostos_physical_range){frame,GHOSTOS_FRAME_SIZE});}
ghostos_reclaim_error ghostos_frame_reclaim_quota(ghostos_early_frame_allocator*a,uint32_t owner,ghostos_physical_range r,const ghostos_frame_quota*q){ghostos_reclaim_error e=ghostos_frame_reclaim_range(a,owner,r);if(e==GHOSTOS_RECLAIM_OK&&q&&q->release&&!q->release(q->context,owner,r.length))return GHOSTOS_RECLAIM_ACCESS_DENIED;return e;}
ghostos_reclaim_error ghostos_frame_reclaim_capability(ghostos_early_frame_allocator*a,uint32_t caller,uint64_t authority,ghostos_physical_range r,const ghostos_frame_quota*q){if(!q||!q->authorize||!q->authorize(q->context,caller,authority))return GHOSTOS_RECLAIM_ACCESS_DENIED;ghostos_reclaim_error e=ghostos_frame_reclaim_range(a,caller,r);if(e==GHOSTOS_RECLAIM_OK&&q->release&&!q->release(q->context,caller,r.length))return GHOSTOS_RECLAIM_ACCESS_DENIED;return e;}
bool ghostos_frame_owner_of(const ghostos_early_frame_allocator*a,uint64_t frame,uint32_t*owner){if(frame%GHOSTOS_FRAME_SIZE)return false;range f={frame,GHOSTOS_FRAME_SIZE};for(size_t i=0;i<GHOSTOS_MAX_OWNED_FRAME_RANGES;i++){const ghostos_owned_frame_range*e=&a->owned_ranges[i];if(e->used&&contains((range){e->start,e->length},f)){if(owner)*owner=e->owner;return true;}}return false;}
uint64_t ghostos_frame_available(const ghostos_early_frame_allocator*a){uint64_t n=0;for(size_t i=0;i<GHOSTOS_MAX_MEMORY_REGIONS;i++)if(a->free_ranges[i].used)n+=a->free_ranges[i].length/GHOSTOS_FRAME_SIZE;return n;}
uint64_t ghostos_frame_owned(const ghostos_early_frame_allocator*a,uint32_t owner){uint64_t n=0;for(size_t i=0;i<GHOSTOS_MAX_OWNED_FRAME_RANGES;i++)if(a->owned_ranges[i].used&&a->owned_ranges[i].owner==owner)n+=a->owned_ranges[i].length/GHOSTOS_FRAME_SIZE;return n;}
