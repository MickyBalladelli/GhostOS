#ifndef GHOSTOS_PARTITION_H
#define GHOSTOS_PARTITION_H

#include <stdbool.h>
#include <stdint.h>

typedef enum {
    GHOSTOS_CORE_PARTITION_OK = 0,
    GHOSTOS_CORE_PARTITION_EMPTY_MASK,
    GHOSTOS_CORE_PARTITION_OFFLINE_CORE,
    GHOSTOS_CORE_PARTITION_NO_HOUSEKEEPING_CORE
} ghostos_core_partition_error;

typedef struct {
    uint64_t online[2];
    uint64_t isolated[2];
} ghostos_core_partition;

void ghostos_core_partition_init(ghostos_core_partition *partition);
void ghostos_core_partition_get_online(const ghostos_core_partition *partition, uint64_t words[2]);
void ghostos_core_partition_get_isolated(const ghostos_core_partition *partition, uint64_t words[2]);
void ghostos_core_partition_get_housekeeping(const ghostos_core_partition *partition, uint64_t words[2]);
bool ghostos_core_partition_is_online(const ghostos_core_partition *partition, uint8_t cpu);
bool ghostos_core_partition_is_isolated(const ghostos_core_partition *partition, uint8_t cpu);
bool ghostos_core_partition_accepts_kernel_work(const ghostos_core_partition *partition, uint8_t cpu);
bool ghostos_core_partition_accepts_timer(const ghostos_core_partition *partition, uint8_t cpu);
bool ghostos_core_partition_accepts_ipc(const ghostos_core_partition *partition, uint8_t cpu);
ghostos_core_partition_error ghostos_core_partition_set_online(
    ghostos_core_partition *partition, const uint64_t online[2]);
ghostos_core_partition_error ghostos_core_partition_isolate(
    ghostos_core_partition *partition, const uint64_t cpus[2]);
void ghostos_core_partition_release(ghostos_core_partition *partition, const uint64_t cpus[2]);

#endif
