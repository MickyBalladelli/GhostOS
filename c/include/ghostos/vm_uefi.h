#ifndef GHOSTOS_VM_UEFI_H
#define GHOSTOS_VM_UEFI_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Results: 0 success, 1 invalid image, 2 unsupported image, 3 out of memory,
 * 4 guest write failed, 5 checked arithmetic overflow (host panic adapter). */
typedef struct {
    uint64_t image_base;
    uint32_t entry_rva, size_of_image, reloc_rva, reloc_size;
    size_t sections_start, section_count;
} ghostos_vm_uefi_pe;
typedef struct {
    uint32_t virtual_address;
    size_t raw_offset, raw_length;
} ghostos_vm_uefi_pe_section;
typedef struct { uint64_t base, size; } ghostos_vm_uefi_image_range;
typedef struct {
    uint32_t virtual_address;
    const uint8_t *bytes;
    size_t length;
} ghostos_vm_uefi_section;
typedef struct {
    bool (*read_value)(void *, uint64_t address, uint32_t width, uint64_t *value);
    bool (*write_value)(void *, uint64_t address, uint32_t width, uint64_t value);
    bool (*write_phys)(void *, uint64_t address, const uint8_t *, size_t);
    void *context;
} ghostos_vm_uefi_io;

uint32_t ghostos_vm_uefi_pe_parse(const uint8_t *, size_t, ghostos_vm_uefi_pe *);
bool ghostos_vm_uefi_pe_section_at(const uint8_t *, size_t, const ghostos_vm_uefi_pe *, size_t index, ghostos_vm_uefi_pe_section *);
/* Callbacks and section/range bytes are borrowed synchronously. Virtual value
 * reads default to zero on failure; relocation write failures stay ignored. */
uint32_t ghostos_vm_uefi_map_pe(const ghostos_vm_uefi_pe *, const ghostos_vm_uefi_section *, size_t,
    uint64_t requested, uint64_t memory_size, const ghostos_vm_uefi_image_range *, size_t,
    bool checked, const ghostos_vm_uefi_io *);
uint32_t ghostos_vm_uefi_relocate(uint64_t base, uint64_t delta, uint32_t rva, uint32_t size,
    bool checked, const ghostos_vm_uefi_io *);
/* Returns descriptor count. NULL output is a size query. An undersized output
 * is untouched. Each descriptor is 48 bytes; image order ties remain stable. */
size_t ghostos_vm_uefi_memory_map(uint64_t memory_size, const ghostos_vm_uefi_image_range *, size_t,
    uint8_t *output, size_t descriptor_capacity);
void ghostos_vm_uefi_service_stub(uint64_t id, uint8_t output[8]);
void ghostos_vm_uefi_table_header(uint64_t signature, uint8_t output[24]);
/* Preserve the VM's existing RSDP byte layout, including legacy field offsets. */
void ghostos_vm_uefi_rsdp(uint8_t output[36]);

typedef struct {
    uint64_t map_size, buffer, map_key, descriptor_size, descriptor_version;
} ghostos_vm_uefi_map_args;
/* These return the original 64-bit EFI status codes. Descriptor physical
 * writes are checked; metadata value-write errors deliberately stay ignored. */
uint64_t ghostos_vm_uefi_get_memory_map(const ghostos_vm_uefi_map_args *, const uint8_t *descriptors,
    size_t count, size_t *map_key, const ghostos_vm_uefi_io *);
uint64_t ghostos_vm_uefi_exit_boot_services(bool *active, bool valid_handle, size_t map_key, uint64_t supplied_key);
bool ghostos_vm_uefi_is_boot_service(uint64_t id);
#endif
