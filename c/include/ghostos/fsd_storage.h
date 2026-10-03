#ifndef GHOSTOS_FSD_STORAGE_H
#define GHOSTOS_FSD_STORAGE_H
#include "ghostos/fsd_open.h"
#include "ghostos/volume_mutation.h"
enum { GHOSTOS_FSD_STORAGE_NO_SPACE = GHOSTOS_FSD_HANDLES_STORAGE_FULL };
/* Install as fsd_open_state.write_empty with a validated
   ghostos_volume_mutation as context. Publishes in-memory C tree/reader state;
   durability remains a separate explicit sync operation. */
int ghostos_fsd_storage_write_empty(void *context, ghostos_volume_reader *reader,
    const uint8_t *path, size_t path_length);
ghostos_status ghostos_fsd_storage_status(int result);
#endif
