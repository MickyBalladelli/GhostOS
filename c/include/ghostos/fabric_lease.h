#ifndef GHOSTOS_FABRIC_LEASE_H
#define GHOSTOS_FABRIC_LEASE_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 device not found, 2 busy, 3 alignment, 4 invalid range,
   5 capacity, 6 not owner, 7 expired, 8 lease not found.
   Access: read=0, write=1, execute=2. Rights: read=1, write=2, read+write=3. */
typedef struct {
    bool occupied;
    uint32_t generation;
    uint32_t pool;
    uint64_t owner, start, length, expires_at_us;
    uint8_t rights;
} ghostos_fabric_lease;
int ghostos_fabric_lease_allocate(ghostos_fabric_lease *leases, size_t capacity, bool pool_present, bool draining,
    uint32_t pool, uint64_t pool_start, uint64_t pool_length, uint64_t owner, uint64_t length, uint64_t alignment,
    uint8_t rights, uint64_t now_us, uint64_t duration_us, uint64_t *handle, uint64_t *start);
int ghostos_fabric_lease_largest_free(ghostos_fabric_lease *leases, size_t capacity, bool pool_present, uint32_t pool,
    uint64_t pool_start, uint64_t pool_length, uint64_t alignment, uint64_t now_us, uint64_t *start, uint64_t *length);
int ghostos_fabric_lease_authorize(const ghostos_fabric_lease *leases, size_t capacity, uint64_t handle, uint64_t owner,
    uint64_t address, uint8_t access, uint64_t now_us);
#endif
