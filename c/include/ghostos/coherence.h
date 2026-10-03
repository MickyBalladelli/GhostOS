#ifndef GHOSTOS_COHERENCE_H
#define GHOSTOS_COHERENCE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid address, 2 invalid range, 3 capacity,
   4 expired lease, 5 not owner.
   Access: read=0, write=1, execute=2. Mode: protected read=0, exclusive=1.
   State: invalid=0, shared=1, exclusive=2.
   Action: map local=0, acquire lease=1, fetch=2, invalidate=3. */
typedef struct {
    uint64_t page_address, sharers;
    uint32_t owner, lease_owner, lease_epoch;
    uint64_t lease_expires_at_us;
    uint8_t state, lease_mode;
    bool has_owner, has_lease;
} ghostos_coherence_page;
void ghostos_coherence_init(ghostos_coherence_page *pages, size_t capacity);
int ghostos_coherence_grant(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, uint32_t owner,
    uint8_t mode, uint32_t epoch, uint64_t expires_at_us);
int ghostos_coherence_arrive(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, uint32_t node, bool writable);
int ghostos_coherence_begin_fault(ghostos_coherence_page *pages, size_t capacity, uint32_t requester, uint64_t virtual_address,
    uint8_t access, bool reserved_bit, uint64_t now_us, uint8_t *action, bool *writable, uint64_t *nodes);
int ghostos_coherence_invalidation_complete(ghostos_coherence_page *pages, size_t capacity, uint64_t page_address, uint32_t writer);
int ghostos_coherence_fail_node(ghostos_coherence_page *pages, size_t capacity, uint32_t node, size_t *changed);
int ghostos_coherence_first(const ghostos_coherence_page *pages, size_t capacity, bool *has_lease);
#endif
