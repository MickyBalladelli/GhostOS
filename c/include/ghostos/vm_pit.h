#ifndef GHOSTOS_VM_PIT_H
#define GHOSTOS_VM_PIT_H

#include <stdbool.h>
#include <stdint.h>

#define GHOSTOS_VM_PIT_FREQUENCY_HZ UINT64_C(1193182)

typedef struct {
    uint16_t reload, count, latched_count;
    uint8_t mode, access, latched_status, wr_bytes, rd_bytes;
    bool bcd, gate, output, running, null_count;
    bool has_latched_count, has_latched_status;
} ghostos_vm_pit_channel;

typedef struct {
    ghostos_vm_pit_channel channels[3];
    uint64_t last_ns;
    bool has_last_ns;
} ghostos_vm_pit;

/* Port results: 0 success, 1 unsupported size, 2 invalid address. */
void ghostos_vm_pit_init(ghostos_vm_pit *pit);
uint8_t ghostos_vm_pit_read(ghostos_vm_pit *pit, uint16_t port,
    uint8_t size, uint64_t *value);
uint8_t ghostos_vm_pit_write(ghostos_vm_pit *pit, uint16_t port,
    uint64_t value, uint8_t size);
/* True means channel zero emitted a pulse; the host routes the IRQ. */
bool ghostos_vm_pit_advance(ghostos_vm_pit *pit, uint64_t now_ns);

#endif
