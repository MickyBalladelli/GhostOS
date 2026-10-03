#ifndef GHOSTOS_GDB_H
#define GHOSTOS_GDB_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Ingest: 0 means the packet is still partial. A positive value is the
 * response length. Negative values are -1 too small, -2 corrupt, -3 invalid,
 * and -4 runtime failure. Operations: read=0, write=1, control=2. */
#define GHOSTOS_GDB_PACKET_BYTES 512u
typedef int (*ghostos_gdb_permit_fn)(void *context, uint64_t token, uint8_t operation);
typedef int (*ghostos_gdb_memory_fn)(void *context, uint64_t address, uint8_t *bytes, size_t length, size_t *transferred);
typedef struct {
    void *context;
    ghostos_gdb_permit_fn permit;
    ghostos_gdb_memory_fn read_memory;
    ghostos_gdb_memory_fn write_memory;
    uint8_t stop_signal;
} ghostos_gdb_runtime;
typedef struct {
    ghostos_gdb_runtime runtime;
    uint64_t token;
    uint8_t payload[GHOSTOS_GDB_PACKET_BYTES];
    size_t payload_len;
    uint8_t checksum, received_checksum, input_state;
    bool detached;
} ghostos_gdb;
void ghostos_gdb_init(ghostos_gdb *stub, uint64_t token, ghostos_gdb_runtime runtime);
int ghostos_gdb_ingest(ghostos_gdb *stub, const uint8_t *input, size_t input_len, uint8_t *output, size_t output_len);
#endif
