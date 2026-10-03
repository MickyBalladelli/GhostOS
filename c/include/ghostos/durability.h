#ifndef GHOSTOS_DURABILITY_H
#define GHOSTOS_DURABILITY_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#define GHOSTOS_DURABILITY_CONTRACT_VERSION 1

enum ghostos_crash_domain {
    GHOSTOS_CRASH_SYNFS, GHOSTOS_CRASH_STORAGE, GHOSTOS_CRASH_PACKAGE_ACTIVATION,
    GHOSTOS_CRASH_CONFIGURATION, GHOSTOS_CRASH_COMPILER_JOB, GHOSTOS_CRASH_UPDATE_RECOVERY
};
enum ghostos_crash_boundary {
    GHOSTOS_BOUNDARY_FLUSH, GHOSTOS_BOUNDARY_JOURNAL_RECORD,
    GHOSTOS_BOUNDARY_MANIFEST_SLOT, GHOSTOS_BOUNDARY_RENAME,
    GHOSTOS_BOUNDARY_CAPABILITY_CHANGE, GHOSTOS_BOUNDARY_SERVICE_RESTART
};
enum ghostos_durability_event_kind {
    GHOSTOS_APPLICATION_WRITE, GHOSTOS_SYNFS_WRITE, GHOSTOS_RENAME,
    GHOSTOS_COMMIT, GHOSTOS_STORAGE_DAEMON_WRITE, GHOSTOS_CACHE_FLUSH,
    GHOSTOS_BLOCK_DATA_WRITE, GHOSTOS_BLOCK_COMMIT_RECORD,
    GHOSTOS_BLOCK_FLUSH, GHOSTOS_SYNC_ACKNOWLEDGED,
    GHOSTOS_POWER_LOSS, GHOSTOS_RECOVERED
};
enum ghostos_durability_error {
    GHOSTOS_DURABILITY_OK, GHOSTOS_TRACE_FULL, GHOSTOS_MISSING_STEP,
    GHOSTOS_INVALID_ORDER, GHOSTOS_RECOVERED_VOLATILE
};
/* Stable wire-independent representation; kind uses the values above. */
typedef struct {
    uint64_t transaction;
    uint32_t kind;
    uint32_t reserved;
} ghostos_durability_event;

typedef struct {
    uint32_t layer, write, flush, sync, rename, commit, recovery;
} ghostos_durability_layer_contract;
/* Layers: application, SynFs, storage daemon, cache, block device, recovery.
 * Guarantees: volatile=0, published=1, durable=2, recoverable=3. */
extern const ghostos_durability_layer_contract ghostos_durability_contract[6];
typedef bool (*ghostos_interruption_checkpoint)(void *, uint32_t, uint32_t);
bool ghostos_no_interruption(void *context, uint32_t domain, uint32_t boundary);
uint32_t ghostos_durability_record(ghostos_durability_event *events,
    size_t capacity, size_t *length, ghostos_durability_event event);
uint32_t ghostos_durability_verify(const ghostos_durability_event *events,
    size_t length, uint64_t *failed_transaction);

#endif
