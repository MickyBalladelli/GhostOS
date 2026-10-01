#include "ghostos/boot_protocol.h"

const ghostos_memory_region ghostos_memory_region_empty = {0, 0, GHOSTOS_MEMORY_RESERVED, 0};
const ghostos_framebuffer_info ghostos_framebuffer_empty = {0};

bool ghostos_boot_method_from_raw(uint32_t raw, ghostos_boot_method *out) {
    if ((raw != GHOSTOS_BOOT_BIOS && raw != GHOSTOS_BOOT_UEFI) || out == NULL) return false;
    *out = raw;
    return true;
}
bool ghostos_memory_kind_from_raw(uint32_t raw, ghostos_memory_kind *out) {
    if (raw < GHOSTOS_MEMORY_USABLE || raw > GHOSTOS_MEMORY_FRAMEBUFFER || out == NULL) return false;
    *out = raw;
    return true;
}
uint64_t ghostos_memory_region_end(ghostos_memory_region region) {
    return region.start > UINT64_MAX - region.length ? UINT64_MAX : region.start + region.length;
}
bool ghostos_memory_region_is_valid(ghostos_memory_region region) {
    return region.length != 0 && (region.start != 0 || region.kind == GHOSTOS_MEMORY_RESERVED) &&
        region.start <= UINT64_MAX - region.length &&
        region.kind >= GHOSTOS_MEMORY_USABLE && region.kind <= GHOSTOS_MEMORY_FRAMEBUFFER;
}
bool ghostos_framebuffer_is_valid(ghostos_framebuffer_info fb) {
    if (fb.address == 0 && fb.size == 0 && fb.width == 0 && fb.height == 0 && fb.stride == 0 && fb.pixel_format == 0) return true;
    if (fb.address == 0 || fb.size == 0 || fb.width == 0 || fb.height == 0 || fb.stride < fb.width ||
        (fb.pixel_format != GHOSTOS_FRAMEBUFFER_PIXEL_RGB && fb.pixel_format != GHOSTOS_FRAMEBUFFER_PIXEL_BGR)) return false;
    uint64_t pixels = (uint64_t)fb.stride * fb.height;
    return pixels <= UINT64_MAX / 4 && pixels * 4 <= fb.size;
}
ghostos_boot_info ghostos_boot_info_empty(ghostos_boot_method method) {
    ghostos_boot_info info = {0};
    info.magic = GHOSTOS_BOOT_INFO_MAGIC;
    info.version = GHOSTOS_BOOT_INFO_VERSION;
    info.method = method;
    for (size_t i = 0; i < GHOSTOS_MAX_MEMORY_REGIONS; ++i) info.memory_regions[i] = ghostos_memory_region_empty;
    return info;
}
bool ghostos_boot_info_push_region(ghostos_boot_info *info, ghostos_memory_region region) {
    if (info == NULL || info->memory_region_count >= GHOSTOS_MAX_MEMORY_REGIONS || !ghostos_memory_region_is_valid(region)) return false;
    if (info->memory_region_count != 0 && region.start < ghostos_memory_region_end(info->memory_regions[info->memory_region_count - 1])) return false;
    info->memory_regions[info->memory_region_count++] = region;
    return true;
}
bool ghostos_boot_info_regions(const ghostos_boot_info *info, const ghostos_memory_region **regions, size_t *count) {
    if (info == NULL || regions == NULL || count == NULL || info->memory_region_count > GHOSTOS_MAX_MEMORY_REGIONS) return false;
    *regions = info->memory_regions;
    *count = info->memory_region_count;
    return true;
}
bool ghostos_boot_info_is_valid(const ghostos_boot_info *info) {
    if (info == NULL || info->magic != GHOSTOS_BOOT_INFO_MAGIC || info->version != GHOSTOS_BOOT_INFO_VERSION ||
        info->memory_region_count > GHOSTOS_MAX_MEMORY_REGIONS ||
        (info->method != GHOSTOS_BOOT_BIOS && info->method != GHOSTOS_BOOT_UEFI) ||
        !ghostos_framebuffer_is_valid(info->framebuffer)) return false;
    for (size_t i = 0; i < info->memory_region_count; ++i) {
        if (!ghostos_memory_region_is_valid(info->memory_regions[i])) return false;
        if (i != 0 && info->memory_regions[i].start < ghostos_memory_region_end(info->memory_regions[i - 1])) return false;
    }
    return true;
}
