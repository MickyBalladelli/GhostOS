#ifndef GHOSTOS_VOLUME_VALIDATE_H
#define GHOSTOS_VOLUME_VALIDATE_H
#include "ghostos/volume_mutation.h"
/* Full reachable-root validation, independently for live and pinned roots.
   Uses mutation marked/pending arrays and data_owners, all sized block_count.
   No heap/recursion. Scratch arrays and payload change; storage does not.
   Results: 0 valid, 1 corrupt/configuration, 5 insufficient scratch.
   Local key order is checked; separators remain hints, as in the source.
   Unreachable block graph structure is not checked. */
int ghostos_volume_validate(const ghostos_volume_mutation *volume, uint64_t next_checkpoint);
#endif
