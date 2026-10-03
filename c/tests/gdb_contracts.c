#include "ghostos/gdb.h"
#include <assert.h>
#include <string.h>
static uint8_t memory[8] = {1, 2, 3, 4, 5, 6, 7, 8};
static int permit_read(void *context, uint64_t token, uint8_t operation) {
    (void)context;
    return token == 9 && operation == 0;
}
static int read_memory(void *context, uint64_t address, uint8_t *bytes, size_t length, size_t *transferred) {
    size_t count = length < sizeof memory ? length : sizeof memory;
    size_t i;
    (void)context;
    (void)address;
    for (i = 0; i < count; ++i) bytes[i] = memory[i];
    *transferred = count;
    return 0;
}
static void frame(const char *payload, uint8_t *output, size_t *length) {
    uint8_t checksum = 0;
    size_t i;
    output[0] = '$';
    for (i = 0; payload[i]; ++i) {
        output[1 + i] = (uint8_t)payload[i];
        checksum = (uint8_t)(checksum + (uint8_t)payload[i]);
    }
    output[1 + i] = '#';
    output[2 + i] = (uint8_t)((checksum >> 4) < 10 ? '0' + (checksum >> 4) : 'a' + (checksum >> 4) - 10);
    output[3 + i] = (uint8_t)((checksum & 0x0f) < 10 ? '0' + (checksum & 0x0f) : 'a' + (checksum & 0x0f) - 10);
    *length = i + 4;
}
static void partial_memory_read_and_denied_write(void) {
    ghostos_gdb stub;
    ghostos_gdb_runtime runtime = {0, permit_read, read_memory, 0, 11};
    uint8_t packet[16];
    uint8_t output[64];
    size_t length = 0;
    int written;
    ghostos_gdb_init(&stub, 9, runtime);
    frame("m0,4", packet, &length);
    assert(ghostos_gdb_ingest(&stub, packet, 3, output, sizeof output) == 0);
    written = ghostos_gdb_ingest(&stub, packet + 3, length - 3, output, sizeof output);
    assert(written == 13);
    assert(memcmp(output, "+$01020304#0a", 13) == 0);
    frame("M0,1:ff", packet, &length);
    written = ghostos_gdb_ingest(&stub, packet, length, output, sizeof output);
    assert(written == 8);
    assert(memcmp(output, "+$E03#a8", 8) == 0);
}
int main(void) {
    partial_memory_read_and_denied_write();
    return 0;
}
