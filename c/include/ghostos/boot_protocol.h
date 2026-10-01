#ifndef GHOSTOS_BOOT_PROTOCOL_H
#define GHOSTOS_BOOT_PROTOCOL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_BOOT_INFO_MAGIC UINT64_C(0x53594e4f53424f4f)
#define GHOSTOS_BOOT_INFO_VERSION 1u
#define GHOSTOS_MAX_MEMORY_REGIONS 128u
#define GHOSTOS_FRAMEBUFFER_PIXEL_RGB 1u
#define GHOSTOS_FRAMEBUFFER_PIXEL_BGR 2u
#define GHOSTOS_PERSISTENCE_PORT 0x5400u
#define GHOSTOS_PERSISTENCE_PORT_SIZE 12u
#define GHOSTOS_PERSISTENCE_COMMAND_PORT GHOSTOS_PERSISTENCE_PORT
#define GHOSTOS_PERSISTENCE_LENGTH_PORT (GHOSTOS_PERSISTENCE_PORT + 4u)
#define GHOSTOS_PERSISTENCE_DATA_PORT (GHOSTOS_PERSISTENCE_PORT + 8u)
#define GHOSTOS_PERSISTENCE_LOAD 1u
#define GHOSTOS_PERSISTENCE_SAVE 2u
#define GHOSTOS_PERSISTENCE_FLUSH 3u
#define GHOSTOS_PERSISTENCE_MAX_BYTES (60u * 1024u)

typedef uint32_t ghostos_boot_method;
enum { GHOSTOS_BOOT_BIOS = 1, GHOSTOS_BOOT_UEFI = 2 };
typedef uint32_t ghostos_memory_kind;
enum {
    GHOSTOS_MEMORY_USABLE = 1, GHOSTOS_MEMORY_RESERVED, GHOSTOS_MEMORY_ACPI_RECLAIMABLE,
    GHOSTOS_MEMORY_ACPI_NON_VOLATILE, GHOSTOS_MEMORY_BOOTLOADER, GHOSTOS_MEMORY_KERNEL,
    GHOSTOS_MEMORY_FRAMEBUFFER
};
typedef struct {
    uint64_t start, length;
    ghostos_memory_kind kind;
    uint32_t attributes;
} ghostos_memory_region;
typedef struct {
    uint64_t address, size;
    uint32_t width, height, stride, pixel_format;
} ghostos_framebuffer_info;
typedef struct {
    _Alignas(16) uint64_t magic;
    uint32_t version;
    ghostos_boot_method method;
    uint64_t physical_address_offset, rsdp_address;
    ghostos_framebuffer_info framebuffer;
    size_t memory_region_count;
    ghostos_memory_region memory_regions[GHOSTOS_MAX_MEMORY_REGIONS];
} ghostos_boot_info;

_Static_assert(sizeof(ghostos_memory_region) == 24, "boot memory region ABI");
_Static_assert(sizeof(ghostos_framebuffer_info) == 32, "boot framebuffer ABI");
_Static_assert(_Alignof(ghostos_boot_info) == 16, "boot handoff alignment");
#if SIZE_MAX == UINT64_MAX
_Static_assert(offsetof(ghostos_boot_info, memory_regions) == 72, "64-bit boot region offset");
_Static_assert(sizeof(ghostos_boot_info) == 3152, "64-bit boot handoff size");
#endif

extern const ghostos_memory_region ghostos_memory_region_empty;
extern const ghostos_framebuffer_info ghostos_framebuffer_empty;
bool ghostos_boot_method_from_raw(uint32_t raw, ghostos_boot_method *out);
bool ghostos_memory_kind_from_raw(uint32_t raw, ghostos_memory_kind *out);
uint64_t ghostos_memory_region_end(ghostos_memory_region region);
bool ghostos_memory_region_is_valid(ghostos_memory_region region);
bool ghostos_framebuffer_is_valid(ghostos_framebuffer_info framebuffer);
ghostos_boot_info ghostos_boot_info_empty(ghostos_boot_method method);
bool ghostos_boot_info_push_region(ghostos_boot_info *info, ghostos_memory_region region);
bool ghostos_boot_info_regions(const ghostos_boot_info *info, const ghostos_memory_region **regions, size_t *count);
bool ghostos_boot_info_is_valid(const ghostos_boot_info *info);
#endif
