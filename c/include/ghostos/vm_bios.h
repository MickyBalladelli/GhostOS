#ifndef GHOSTOS_VM_BIOS_H
#define GHOSTOS_VM_BIOS_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
typedef struct {
    uint64_t rax, rbx, rcx, rdx, rsi, rdi, rflags;
    uint16_t ds, es;
} ghostos_vm_bios_registers;
typedef struct {
    bool (*read)(void *, uint64_t, uint8_t *, size_t);
    bool (*write)(void *, uint64_t, const uint8_t *, size_t);
    void *context;
} ghostos_vm_bios_io;
void ghostos_vm_bios_rom(uint8_t output[65536]);
void ghostos_vm_bios_post_tables(uint8_t ivt[1024], uint8_t bda[256]);
/* Image NULL means absent; non-NULL and zero length means present but empty.
 * Preserve legacy CHS/EDD quirks, including the EDD low-byte count and
 * segment/offset positions, and ignored CHS DMA-write failures. */
void ghostos_vm_bios_service(uint8_t vector, ghostos_vm_bios_registers *registers,
    const uint8_t *image, size_t image_length, uint64_t memory_size,
    const ghostos_vm_bios_io *io);
#endif
