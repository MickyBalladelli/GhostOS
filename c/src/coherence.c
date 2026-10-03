#include "ghostos/coherence.h"
enum { GHOSTOS_COHERENCE_PAGE = 4096 };
static int node_bit(uint32_t node, uint64_t *bit) {
    if (!node || node > 64) return 3;
    *bit = 1ull << (node - 1);
    return 0;
}
static uint32_t first_node(uint64_t nodes) {
    uint32_t bit = 0;
    if (!nodes) return 0;
    while ((nodes & 1ull) == 0) { nodes >>= 1; bit += 1; }
    return bit + 1;
}
static int entry(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, size_t *index) {
    size_t i;
    for (i = 0; i < capacity; ++i) if (pages[i].page_address == page_address) { *index = i; return 0; }
    for (i = 0; i < capacity; ++i) if (pages[i].page_address == UINT64_MAX) {
        pages[i].page_address = page_address;
        *index = i;
        return 0;
    }
    return 3;
}
void ghostos_coherence_init(ghostos_coherence_page *pages, size_t capacity) {
    size_t i;
    for (i = 0; i < capacity; ++i) {
        pages[i].page_address = UINT64_MAX;
        pages[i].sharers = 0;
        pages[i].owner = 0;
        pages[i].lease_owner = 0;
        pages[i].lease_epoch = 0;
        pages[i].lease_expires_at_us = 0;
        pages[i].state = 0;
        pages[i].lease_mode = 0;
        pages[i].has_owner = false;
        pages[i].has_lease = false;
    }
}
int ghostos_coherence_grant(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, uint32_t owner,
    uint8_t mode, uint32_t epoch, uint64_t expires_at_us) {
    size_t index = 0;
    int status;
    if (page_address % GHOSTOS_COHERENCE_PAGE != 0 || !expires_at_us) return 2;
    status = entry(pages, capacity, page_address, &index);
    if (status) return status;
    pages[index].has_lease = true;
    pages[index].lease_owner = owner;
    pages[index].lease_mode = mode;
    pages[index].lease_epoch = epoch;
    pages[index].lease_expires_at_us = expires_at_us;
    return 0;
}
int ghostos_coherence_arrive(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, uint32_t node, bool writable) {
    size_t index = 0;
    uint64_t bit = 0;
    int status = entry(pages, capacity, page_address, &index);
    if (status) return status;
    if (writable) {
        pages[index].state = 2;
        pages[index].has_owner = true;
        pages[index].owner = node;
        pages[index].sharers = 0;
        return 0;
    }
    status = node_bit(node, &bit);
    if (status) return status;
    pages[index].state = 1;
    pages[index].sharers |= bit;
    return 0;
}
int ghostos_coherence_begin_fault(ghostos_coherence_page *pages, size_t capacity, uint32_t requester, uint64_t virtual_address,
    uint8_t access, bool reserved_bit, uint64_t now_us, uint8_t *action, bool *writable, uint64_t *nodes) {
    size_t index = 0;
    uint64_t page_address = virtual_address & ~(uint64_t)(GHOSTOS_COHERENCE_PAGE - 1);
    uint64_t bit = 0;
    int status;
    bool valid_local;
    if (reserved_bit || access == 2) return 1;
    status = entry(pages, capacity, page_address, &index);
    if (status) return status;
    if (pages[index].has_lease && now_us >= pages[index].lease_expires_at_us) {
        pages[index].has_lease = false;
        pages[index].state = 0;
        pages[index].has_owner = false;
        pages[index].sharers = 0;
    }
    valid_local = pages[index].has_lease && pages[index].lease_owner == requester && now_us < pages[index].lease_expires_at_us;
    if (!valid_local) {
        *action = 1;
        *writable = access == 1;
        *nodes = 0;
        return 0;
    }
    status = node_bit(requester, &bit);
    if (status) return status;
    if (access == 1) {
        uint64_t others = pages[index].sharers & ~bit;
        if (others) {
            *action = 3;
            *writable = true;
            *nodes = others;
            return 0;
        }
        if (pages[index].state == 2 && pages[index].has_owner && pages[index].owner == requester) {
            *action = 0;
            *writable = true;
            *nodes = 0;
            return 0;
        }
    } else if ((pages[index].sharers & bit) != 0) {
        *action = 0;
        *writable = false;
        *nodes = 0;
        return 0;
    }
    {
        uint32_t source = pages[index].has_owner ? pages[index].owner : first_node(pages[index].sharers);
        if (source && source != requester) {
            *action = 2;
            *writable = access == 1;
            *nodes = source;
            return 0;
        }
    }
    *action = 0;
    *writable = access == 1;
    *nodes = 0;
    return 0;
}
int ghostos_coherence_invalidation_complete(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, uint32_t writer) {
    size_t index = 0;
    int status = entry(pages, capacity, page_address, &index);
    if (status) return status;
    if (!pages[index].has_lease) return 4;
    if (pages[index].lease_owner != writer || pages[index].lease_mode != 1) return 5;
    pages[index].state = 2;
    pages[index].has_owner = true;
    pages[index].owner = writer;
    pages[index].sharers = 0;
    return 0;
}
int ghostos_coherence_fail_node(ghostos_coherence_page *pages, size_t capacity, uint32_t node, size_t *changed) {
    uint64_t bit = 0;
    size_t i;
    int status = node_bit(node, &bit);
    *changed = 0;
    if (status) return status;
    for (i = 0; i < capacity; ++i) {
        bool affected;
        if (pages[i].page_address == UINT64_MAX) continue;
        affected = (pages[i].has_owner && pages[i].owner == node) || (pages[i].sharers & bit) != 0 ||
            (pages[i].has_lease && pages[i].lease_owner == node);
        if (!affected) continue;
        pages[i].sharers &= ~bit;
        if (pages[i].has_owner && pages[i].owner == node) {
            pages[i].has_owner = false;
            pages[i].state = pages[i].sharers ? 1 : 0;
        }
        if (pages[i].has_lease && pages[i].lease_owner == node) pages[i].has_lease = false;
        *changed += 1;
    }
    return 0;
}
int ghostos_coherence_first(const ghostos_coherence_page *pages, size_t capacity, bool *has_lease) {
    size_t i;
    for (i = 0; i < capacity; ++i) if (pages[i].page_address != UINT64_MAX) { *has_lease = pages[i].has_lease; return 0; }
    return 4;
}
