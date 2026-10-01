#include "ghostos/partition.h"

static bool empty(const uint64_t words[2]) {
    return !words[0] && !words[1];
}

static void difference(const uint64_t left[2], const uint64_t right[2], uint64_t out[2]) {
    out[0] = left[0] & ~right[0];
    out[1] = left[1] & ~right[1];
}

static void union_mask(const uint64_t left[2], const uint64_t right[2], uint64_t out[2]) {
    out[0] = left[0] | right[0];
    out[1] = left[1] | right[1];
}

void ghostos_core_partition_init(ghostos_core_partition *partition) {
    if (!partition) return;
    *partition = (ghostos_core_partition){{1, 0}, {0, 0}};
}

void ghostos_core_partition_get_online(const ghostos_core_partition *partition, uint64_t words[2]) {
    if (!words) return;
    if (partition) { words[0] = partition->online[0]; words[1] = partition->online[1]; }
    else words[0] = words[1] = 0;
}

void ghostos_core_partition_get_isolated(const ghostos_core_partition *partition, uint64_t words[2]) {
    if (!words) return;
    if (partition) { words[0] = partition->isolated[0]; words[1] = partition->isolated[1]; }
    else words[0] = words[1] = 0;
}

void ghostos_core_partition_get_housekeeping(const ghostos_core_partition *partition, uint64_t words[2]) {
    if (!words) return;
    if (partition) difference(partition->online, partition->isolated, words);
    else words[0] = words[1] = 0;
}

static bool contains(const uint64_t words[2], uint8_t cpu) {
    return words[cpu / 64] & (UINT64_C(1) << (cpu % 64));
}

bool ghostos_core_partition_is_online(const ghostos_core_partition *partition, uint8_t cpu) {
    return partition && cpu < 128 && contains(partition->online, cpu);
}

bool ghostos_core_partition_is_isolated(const ghostos_core_partition *partition, uint8_t cpu) {
    return partition && cpu < 128 && contains(partition->isolated, cpu);
}

bool ghostos_core_partition_accepts_kernel_work(const ghostos_core_partition *partition, uint8_t cpu) {
    return partition && cpu < 128 && contains(partition->online, cpu) && !contains(partition->isolated, cpu);
}

bool ghostos_core_partition_accepts_timer(const ghostos_core_partition *partition, uint8_t cpu) {
    return ghostos_core_partition_accepts_kernel_work(partition, cpu);
}

bool ghostos_core_partition_accepts_ipc(const ghostos_core_partition *partition, uint8_t cpu) {
    return ghostos_core_partition_accepts_kernel_work(partition, cpu);
}

ghostos_core_partition_error ghostos_core_partition_set_online(
    ghostos_core_partition *partition, const uint64_t online[2]) {
    if (!partition || !online) return GHOSTOS_CORE_PARTITION_EMPTY_MASK;
    if (empty(online)) return GHOSTOS_CORE_PARTITION_EMPTY_MASK;
    uint64_t offline_isolated[2];
    difference(partition->isolated, online, offline_isolated);
    if (!empty(offline_isolated)) return GHOSTOS_CORE_PARTITION_OFFLINE_CORE;
    partition->online[0] = online[0];
    partition->online[1] = online[1];
    return GHOSTOS_CORE_PARTITION_OK;
}

ghostos_core_partition_error ghostos_core_partition_isolate(
    ghostos_core_partition *partition, const uint64_t cpus[2]) {
    if (!partition || !cpus || empty(cpus)) return GHOSTOS_CORE_PARTITION_EMPTY_MASK;
    uint64_t offline[2];
    difference(cpus, partition->online, offline);
    if (!empty(offline)) return GHOSTOS_CORE_PARTITION_OFFLINE_CORE;
    uint64_t isolated[2], housekeeping[2];
    union_mask(partition->isolated, cpus, isolated);
    difference(partition->online, isolated, housekeeping);
    if (empty(housekeeping)) return GHOSTOS_CORE_PARTITION_NO_HOUSEKEEPING_CORE;
    partition->isolated[0] = isolated[0];
    partition->isolated[1] = isolated[1];
    return GHOSTOS_CORE_PARTITION_OK;
}

void ghostos_core_partition_release(ghostos_core_partition *partition, const uint64_t cpus[2]) {
    if (!partition || !cpus) return;
    difference(partition->isolated, cpus, partition->isolated);
}
