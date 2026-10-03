#include "ghostos/inference_journal.h"
static void store_be32(uint8_t *bytes, uint32_t value) {
    bytes[0] = (uint8_t)(value >> 24);
    bytes[1] = (uint8_t)(value >> 16);
    bytes[2] = (uint8_t)(value >> 8);
    bytes[3] = (uint8_t)value;
}
static void store_be64(uint8_t *bytes, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) bytes[i] = (uint8_t)(value >> (8 * (7 - i)));
}
static uint32_t load_be32(const uint8_t *bytes) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) | ((uint32_t)bytes[2] << 8) | bytes[3];
}
static uint64_t load_be64(const uint8_t *bytes) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value = (value << 8) | bytes[i];
    return value;
}
static uint32_t crc32(const uint8_t *bytes, size_t length) {
    uint32_t crc = 0xffffffffu;
    size_t i;
    for (i = 0; i < length; ++i) {
        int bit;
        crc ^= bytes[i];
        for (bit = 0; bit < 8; ++bit) {
            uint32_t mask = 0u - (crc & 1u);
            crc = (crc >> 1) ^ (0xedb88320u & mask);
        }
    }
    return ~crc;
}
static void encode_record(const ghostos_inference_record *record, uint8_t *bytes) {
    size_t i;
    uint32_t checksum;
    for (i = 0; i < GHOSTOS_INFERENCE_RECOVERY_BYTES; ++i) bytes[i] = 0;
    bytes[0] = 'S'; bytes[1] = 'Y'; bytes[2] = 'N'; bytes[3] = 'R'; bytes[4] = 1;
    store_be64(bytes + 8, record->request);
    store_be64(bytes + 16, record->model);
    store_be64(bytes + 24, record->kv_cache);
    store_be64(bytes + 32, record->next_token);
    store_be64(bytes + 40, record->rng_state);
    store_be64(bytes + 48, record->epoch);
    store_be32(bytes + 56, record->primary);
    store_be32(bytes + 60, record->replica);
    checksum = crc32(bytes, 64);
    store_be32(bytes + 64, checksum);
}
static void write_checkpoint(const ghostos_inference_record *record, ghostos_inference_checkpoint *checkpoint) {
    checkpoint->primary = record->primary;
    checkpoint->replica = record->replica;
    checkpoint->epoch = record->epoch;
    encode_record(record, checkpoint->bytes);
}
static int node_bit(uint32_t node, uint64_t *bit) {
    if (!node || node > 64) return 3;
    *bit = 1ull << (node - 1);
    return 0;
}
static int valid_slot(const ghostos_inference_journal *entries, size_t capacity, uint64_t handle, size_t *slot) {
    uint32_t generation = (uint32_t)(handle >> 32);
    size_t index = (uint32_t)handle;
    if (index >= capacity) return 2;
    if (!generation || !entries[index].occupied || entries[index].generation != generation) return 4;
    *slot = index;
    return 0;
}
static bool same_request(const ghostos_inference_journal *entry, uint64_t request) {
    if (!entry->occupied) return false;
    if (entry->has_committed && entry->committed.request == request) return true;
    return entry->has_pending && entry->pending.request == request;
}
int ghostos_inference_journal_begin(ghostos_inference_journal *entries, size_t capacity, uint64_t request, uint64_t model,
    uint64_t kv_cache, uint32_t primary, uint32_t replica, uint64_t rng_state, uint64_t *handle,
    ghostos_inference_checkpoint *checkpoint) {
    size_t slot = 0, i;
    uint32_t generation;
    bool found = false;
    if (primary == replica) return 1;
    for (i = 0; i < capacity; ++i) if (same_request(&entries[i], request)) return 2;
    for (i = 0; i < capacity; ++i) if (!entries[i].occupied) { slot = i; found = true; break; }
    if (!found) return 3;
    generation = entries[slot].generation + 1;
    if (!generation) generation = 1;
    entries[slot].occupied = true;
    entries[slot].generation = generation;
    entries[slot].state = 0;
    entries[slot].failed_nodes = 0;
    entries[slot].has_committed = false;
    entries[slot].has_pending = true;
    entries[slot].pending.request = request;
    entries[slot].pending.model = model;
    entries[slot].pending.kv_cache = kv_cache;
    entries[slot].pending.next_token = 0;
    entries[slot].pending.rng_state = rng_state;
    entries[slot].pending.epoch = 1;
    entries[slot].pending.primary = primary;
    entries[slot].pending.replica = replica;
    *handle = ((uint64_t)generation << 32) | slot;
    write_checkpoint(&entries[slot].pending, checkpoint);
    return 0;
}
int ghostos_inference_journal_acknowledge(ghostos_inference_journal *entries, size_t capacity, uint64_t handle,
    uint64_t epoch, bool primary_acknowledged, bool replica_acknowledged, uint8_t *state) {
    size_t slot = 0;
    uint64_t primary_bit = 0, replica_bit = 0;
    int status = valid_slot(entries, capacity, handle, &slot);
    ghostos_inference_record pending;
    if (status) return status;
    if (!entries[slot].has_pending) return 5;
    pending = entries[slot].pending;
    if (pending.epoch != epoch) return 5;
    if (!primary_acknowledged || !replica_acknowledged) return 1;
    status = node_bit(pending.primary, &primary_bit);
    if (!status) status = node_bit(pending.replica, &replica_bit);
    if (status) return status;
    entries[slot].committed = pending;
    entries[slot].has_committed = true;
    entries[slot].has_pending = false;
    entries[slot].state = (entries[slot].failed_nodes & primary_bit) || (entries[slot].failed_nodes & replica_bit) ? 2 : 1;
    *state = entries[slot].state;
    return 0;
}
int ghostos_inference_journal_prepare(ghostos_inference_journal *entries, size_t capacity, uint64_t handle,
    uint64_t next_token, uint64_t rng_state, ghostos_inference_checkpoint *checkpoint) {
    size_t slot = 0;
    ghostos_inference_record pending;
    int status = valid_slot(entries, capacity, handle, &slot);
    if (status) return status;
    if (entries[slot].has_pending || !entries[slot].has_committed) return 5;
    if (next_token < entries[slot].committed.next_token) return 5;
    pending = entries[slot].committed;
    pending.next_token = next_token;
    pending.rng_state = rng_state;
    pending.epoch = pending.epoch == UINT64_MAX ? UINT64_MAX : pending.epoch + 1;
    entries[slot].pending = pending;
    entries[slot].has_pending = true;
    entries[slot].state = 0;
    write_checkpoint(&pending, checkpoint);
    return 0;
}
int ghostos_inference_journal_fail(ghostos_inference_journal *entries, size_t capacity, uint64_t handle, uint32_t failed,
    uint32_t *journal_node) {
    size_t slot = 0;
    uint64_t bit = 0, primary_bit = 0, replica_bit = 0;
    bool primary_failed, replica_failed;
    int status = valid_slot(entries, capacity, handle, &slot);
    if (status) return status;
    status = node_bit(failed, &bit);
    if (status) return status;
    entries[slot].failed_nodes |= bit;
    entries[slot].has_pending = false;
    if (!entries[slot].has_committed) return 1;
    status = node_bit(entries[slot].committed.primary, &primary_bit);
    if (!status) status = node_bit(entries[slot].committed.replica, &replica_bit);
    if (status) return status;
    primary_failed = (entries[slot].failed_nodes & primary_bit) != 0;
    replica_failed = (entries[slot].failed_nodes & replica_bit) != 0;
    if (!primary_failed) *journal_node = entries[slot].committed.primary;
    else if (!replica_failed) *journal_node = entries[slot].committed.replica;
    else return 1;
    entries[slot].state = 2;
    return 0;
}
int ghostos_inference_journal_repair(ghostos_inference_journal *entries, size_t capacity, uint64_t handle,
    uint32_t replacement, ghostos_inference_checkpoint *checkpoint) {
    size_t slot = 0;
    uint64_t primary_bit = 0, replica_bit = 0, replacement_bit = 0;
    uint32_t survivor;
    bool primary_failed, replica_failed;
    ghostos_inference_record pending;
    int status = valid_slot(entries, capacity, handle, &slot);
    if (status) return status;
    if (entries[slot].has_pending || !entries[slot].has_committed) return 5;
    status = node_bit(entries[slot].committed.primary, &primary_bit);
    if (!status) status = node_bit(entries[slot].committed.replica, &replica_bit);
    if (status) return status;
    primary_failed = (entries[slot].failed_nodes & primary_bit) != 0;
    replica_failed = (entries[slot].failed_nodes & replica_bit) != 0;
    if (!primary_failed) survivor = entries[slot].committed.primary;
    else if (!replica_failed) survivor = entries[slot].committed.replica;
    else return 1;
    status = node_bit(replacement, &replacement_bit);
    if (status) return status;
    if (replacement == survivor || (entries[slot].failed_nodes & replacement_bit) != 0) return 1;
    pending = entries[slot].committed;
    pending.epoch = pending.epoch == UINT64_MAX ? UINT64_MAX : pending.epoch + 1;
    pending.primary = survivor;
    pending.replica = replacement;
    entries[slot].pending = pending;
    entries[slot].has_pending = true;
    entries[slot].state = 0;
    write_checkpoint(&pending, checkpoint);
    return 0;
}
int ghostos_inference_recovery_decode(const uint8_t *bytes, size_t length, uint64_t *rng_state) {
    ghostos_inference_record record;
    uint8_t check[GHOSTOS_INFERENCE_RECOVERY_BYTES];
    if (length != GHOSTOS_INFERENCE_RECOVERY_BYTES || bytes[0] != 'S' || bytes[1] != 'Y' || bytes[2] != 'N' || bytes[3] != 'R' || bytes[4] != 1)
        return 6;
    record.request = load_be64(bytes + 8);
    record.model = load_be64(bytes + 16);
    record.kv_cache = load_be64(bytes + 24);
    record.next_token = load_be64(bytes + 32);
    record.rng_state = load_be64(bytes + 40);
    record.epoch = load_be64(bytes + 48);
    record.primary = load_be32(bytes + 56);
    record.replica = load_be32(bytes + 60);
    if (!record.request || !(record.model >> 32) || !(record.kv_cache >> 32) || !record.primary || !record.replica ||
        record.primary == record.replica || !record.epoch)
        return 6;
    encode_record(&record, check);
    if (load_be32(bytes + 64) != load_be32(check + 64)) return 6;
    *rng_state = record.rng_state;
    return 0;
}
