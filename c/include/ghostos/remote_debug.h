#ifndef GHOSTOS_REMOTE_DEBUG_H
#define GHOSTOS_REMOTE_DEBUG_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Open: 0 accepted, 1 invalid capability, 2 target mismatch.
 * Operation: read=0, write=1, control=2. Rights include DEBUG plus READ or WRITE. */
#define GHOSTOS_REMOTE_DEBUG_WIRE_BYTES 192u
int ghostos_remote_debug_open(const uint8_t *wire, size_t length, uint64_t target,
    uint64_t *resource, uint64_t *nonce);
uint16_t ghostos_remote_debug_rights(uint8_t operation);
bool ghostos_remote_debug_permit(uint64_t token, uint64_t nonce, bool authorized);
#endif
