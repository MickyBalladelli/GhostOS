#ifndef GHOSTOS_BOOT_DIAGNOSTICS_H
#define GHOSTOS_BOOT_DIAGNOSTICS_H

#include "ghostos/status.h"

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_BOOT_DIAGNOSTIC_BYTES 56

typedef enum { GHOSTOS_BOOT_KERNEL_ENTRY=1,GHOSTOS_BOOT_INFO_VALIDATED=2,GHOSTOS_BOOT_MEMORY_READY=3,
    GHOSTOS_BOOT_ARCHITECTURE_READY=4,GHOSTOS_BOOT_HARDWARE_READY=5,GHOSTOS_BOOT_STORAGE_READY=6,
    GHOSTOS_BOOT_SERVICES_READY=7,GHOSTOS_BOOT_USER_HANDOFF=8 } ghostos_boot_stage;
typedef enum { GHOSTOS_BOOT_IN_PROGRESS=1,GHOSTOS_BOOT_FAILED=2,GHOSTOS_BOOT_SUCCEEDED=3 } ghostos_boot_state;
typedef struct { uint64_t id; ghostos_boot_stage stage; ghostos_boot_state state; ghostos_status status; bool interrupted; } ghostos_boot_attempt;
typedef struct { ghostos_boot_attempt current, last_failure; bool has_last_failure; uint64_t failure_count; } ghostos_boot_diagnostics;
typedef bool (*ghostos_boot_diagnostic_load)(void *context,uint8_t *bytes,size_t capacity,size_t *length);
typedef void (*ghostos_boot_diagnostic_save)(void *context,const uint8_t *bytes,size_t length);

bool ghostos_boot_stage_from_raw(uint8_t raw,ghostos_boot_stage *out);
const char *ghostos_boot_stage_name(ghostos_boot_stage stage);
ghostos_boot_diagnostics ghostos_boot_diagnostics_initial(uint64_t attempt_id);
void ghostos_boot_diagnostics_begin(const ghostos_boot_diagnostics *previous,bool has_previous,ghostos_boot_diagnostics *next,ghostos_boot_attempt *reported,bool *has_reported);
void ghostos_boot_diagnostics_checkpoint(ghostos_boot_diagnostics *diagnostics,ghostos_boot_stage stage);
void ghostos_boot_diagnostics_fail(ghostos_boot_diagnostics *diagnostics,ghostos_status status);
void ghostos_boot_diagnostics_complete(ghostos_boot_diagnostics *diagnostics);
bool ghostos_boot_diagnostics_encode(const ghostos_boot_diagnostics *diagnostics,uint8_t *destination,size_t capacity,size_t *length);
bool ghostos_boot_diagnostics_decode(const uint8_t *source,size_t length,ghostos_boot_diagnostics *diagnostics);
void ghostos_boot_diagnostics_set_store(ghostos_boot_diagnostic_load load,ghostos_boot_diagnostic_save save,void *context);
bool ghostos_boot_diagnostic_begin(ghostos_boot_attempt *reported);
void ghostos_boot_diagnostic_checkpoint(ghostos_boot_stage stage);
void ghostos_boot_diagnostic_fail(ghostos_status status);
void ghostos_boot_diagnostic_complete(void);
bool ghostos_boot_diagnostic_last_failure(ghostos_boot_attempt *failure);

#endif
