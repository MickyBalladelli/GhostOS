#include "ghostos/volume_device.h"
static int append(uint64_t *log, size_t log_capacity, size_t *log_count, uint64_t block) {
    if (*log_count == log_capacity) return 3;
    log[*log_count] = block;
    *log_count += 1;
    return 0;
}
int ghostos_volume_device_flush(ghostos_volume *volume, uint64_t generation, uint32_t version, uint32_t root,
    uint64_t next_checkpoint, uint64_t next_object_id, size_t block_count, uint64_t *log, size_t log_capacity,
    size_t *log_count, int fail_at, bool fail_flush, bool interrupted, uint64_t *sequence) {
    uint64_t base, next_sequence;
    uint8_t bank;
    size_t index, writes = 0;
    *log_count = 0;
    if (!next_checkpoint || !next_object_id || (root && !generation)) return 1;
    if (volume->sequence == UINT64_MAX) return 2;
    next_sequence = volume->sequence + 1;
    bank = (uint8_t)(1u - volume->active);
    base = (uint64_t)bank * (uint64_t)(block_count + 2u);
    if (fail_at == (int)writes) return 3;
    if (append(log, log_capacity, log_count, base + 1)) return 3;
    writes += 1;
    for (index = 0; index < block_count; ++index) {
        if (fail_at == (int)writes) return 3;
        if (append(log, log_capacity, log_count, base + 2u + (uint64_t)index)) return 3;
        writes += 1;
    }
    if (fail_at == (int)writes) return 3;
    if (append(log, log_capacity, log_count, base)) return 3;
    if (fail_flush) return 3;
    if (append(log, log_capacity, log_count, UINT64_MAX)) return 3;
    volume->banks[bank].valid = true;
    volume->banks[bank].sequence = next_sequence;
    volume->banks[bank].generation = generation;
    volume->banks[bank].version = version;
    volume->sequence = next_sequence;
    volume->active = bank;
    *sequence = next_sequence;
    if (interrupted) return 4;
    return 0;
}
