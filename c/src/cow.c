#include "ghostos/cow.h"

static bool valid_range(ghostos_physical_range r) {
    return r.start % GHOSTOS_FRAME_SIZE == 0 && r.length != 0 &&
        r.length % GHOSTOS_FRAME_SIZE == 0 && UINT64_MAX - r.start >= r.length;
}

static size_t find_page(const ghostos_cow_manager *m, uint64_t frame) {
    for (size_t i = 0; i < GHOSTOS_MAX_COW_PAGES; ++i)
        if (m->pages[i].used && m->pages[i].frame == frame) return i;
    return GHOSTOS_MAX_COW_PAGES;
}

static size_t free_pages(const ghostos_cow_manager *m) {
    size_t count = 0;
    for (size_t i = 0; i < GHOSTOS_MAX_COW_PAGES; ++i) count += !m->pages[i].used;
    return count;
}

void ghostos_cow_init(ghostos_cow_manager *m) {
    for (size_t i = 0; i < GHOSTOS_MAX_COW_PAGES; ++i) m->pages[i].used = false;
}

ghostos_cow_error ghostos_cow_register(ghostos_cow_manager *m, uint32_t owner, ghostos_physical_range r) {
    if (!valid_range(r)) return GHOSTOS_COW_INVALID_RANGE;
    uint64_t count = r.length / GHOSTOS_FRAME_SIZE;
    if (count > GHOSTOS_MAX_COW_PAGES || free_pages(m) < (size_t)count) return GHOSTOS_COW_CAPACITY;
    for (uint64_t i = 0; i < count; ++i)
        if (find_page(m, r.start + i * GHOSTOS_FRAME_SIZE) != GHOSTOS_MAX_COW_PAGES) return GHOSTOS_COW_ALREADY_TRACKED;
    for (uint64_t i = 0; i < count; ++i) {
        size_t slot = 0;
        while (m->pages[slot].used) ++slot;
        m->pages[slot] = (ghostos_cow_page){r.start + i * GHOSTOS_FRAME_SIZE, owner, 1, true};
    }
    return GHOSTOS_COW_OK;
}

ghostos_cow_error ghostos_cow_share(ghostos_cow_manager *m, uint32_t owner, ghostos_physical_range r) {
    if (!valid_range(r)) return GHOSTOS_COW_INVALID_RANGE;
    uint64_t count = r.length / GHOSTOS_FRAME_SIZE;
    for (uint64_t i = 0; i < count; ++i) {
        size_t slot = find_page(m, r.start + i * GHOSTOS_FRAME_SIZE);
        if (slot == GHOSTOS_MAX_COW_PAGES) return GHOSTOS_COW_NOT_TRACKED;
        if (m->pages[slot].owner != owner) return GHOSTOS_COW_INVALID_OWNER;
        if (m->pages[slot].references == UINT32_MAX) return GHOSTOS_COW_CAPACITY;
    }
    for (uint64_t i = 0; i < count; ++i) ++m->pages[find_page(m, r.start + i * GHOSTOS_FRAME_SIZE)].references;
    return GHOSTOS_COW_OK;
}

ghostos_cow_error ghostos_cow_write_fault(ghostos_cow_manager *m, uint32_t owner, uint64_t frame, ghostos_early_frame_allocator *a, ghostos_cow_copy_page_fn copy, void *context, ghostos_cow_write_result *out) {
    if (frame % GHOSTOS_FRAME_SIZE) return GHOSTOS_COW_INVALID_RANGE;
    size_t slot = find_page(m, frame);
    if (slot == GHOSTOS_MAX_COW_PAGES) return GHOSTOS_COW_NOT_TRACKED;
    if (m->pages[slot].references == 1) {
        if (out) *out = (ghostos_cow_write_result){GHOSTOS_COW_WRITE_EXCLUSIVE, frame, frame};
        return GHOSTOS_COW_OK;
    }
    size_t new_slot = 0;
    while (new_slot < GHOSTOS_MAX_COW_PAGES && m->pages[new_slot].used) ++new_slot;
    if (new_slot == GHOSTOS_MAX_COW_PAGES) return GHOSTOS_COW_CAPACITY;
    uint64_t new_frame;
    if (ghostos_frame_allocate_for(a, owner, &new_frame) != GHOSTOS_ALLOC_OK) return GHOSTOS_COW_ALLOCATION;
    if (!copy || !copy(context, frame, new_frame)) {
        if (ghostos_frame_reclaim(a, owner, new_frame) != GHOSTOS_RECLAIM_OK) return GHOSTOS_COW_RECLAIM_FAILED;
        return GHOSTOS_COW_COPY_FAILED;
    }
    --m->pages[slot].references;
    m->pages[new_slot] = (ghostos_cow_page){new_frame, owner, 1, true};
    if (out) *out = (ghostos_cow_write_result){GHOSTOS_COW_WRITE_COPIED, frame, new_frame};
    return GHOSTOS_COW_OK;
}

ghostos_cow_error ghostos_cow_release(ghostos_cow_manager *m, ghostos_physical_range r, ghostos_early_frame_allocator *a, uint64_t *reclaimed) {
    if (!valid_range(r)) return GHOSTOS_COW_INVALID_RANGE;
    uint64_t count = r.length / GHOSTOS_FRAME_SIZE;
    for (uint64_t i = 0; i < count; ++i)
        if (find_page(m, r.start + i * GHOSTOS_FRAME_SIZE) == GHOSTOS_MAX_COW_PAGES) return GHOSTOS_COW_NOT_TRACKED;
    uint64_t removed = 0;
    for (uint64_t i = 0; i < count; ++i) {
        size_t slot = find_page(m, r.start + i * GHOSTOS_FRAME_SIZE);
        ghostos_cow_page *p = &m->pages[slot];
        if (p->references == 1 && ghostos_frame_reclaim(a, p->owner, p->frame) != GHOSTOS_RECLAIM_OK) {
            if (reclaimed) *reclaimed = removed;
            return GHOSTOS_COW_RECLAIM_FAILED;
        }
        if (--p->references == 0) { p->used = false; ++removed; }
    }
    if (reclaimed) *reclaimed = removed;
    return GHOSTOS_COW_OK;
}

bool ghostos_cow_info(const ghostos_cow_manager *m, uint64_t frame, ghostos_cow_page_info *out) {
    size_t slot = find_page(m, frame);
    if (slot == GHOSTOS_MAX_COW_PAGES) return false;
    if (out) *out = (ghostos_cow_page_info){m->pages[slot].frame, m->pages[slot].owner, m->pages[slot].references};
    return true;
}
