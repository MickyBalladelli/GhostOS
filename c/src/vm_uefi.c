#include "ghostos/vm_uefi.h"

static bool range_fits(size_t length, size_t offset, size_t count) {
    return offset <= length && count <= length - offset;
}
static uint16_t le16(const uint8_t *p) { return (uint16_t)(p[0] | (uint16_t)p[1] << 8); }
static uint32_t le32(const uint8_t *p) {
    uint32_t value = 0;
    for (size_t i = 0; i < 4; ++i) value |= (uint32_t)p[i] << (i * 8);
    return value;
}
static uint64_t le64(const uint8_t *p) {
    uint64_t value = 0;
    for (size_t i = 0; i < 8; ++i) value |= (uint64_t)p[i] << (i * 8);
    return value;
}
static void put_le(uint8_t *p, uint64_t value, size_t length) {
    for (size_t i = 0; i < length; ++i) p[i] = (uint8_t)(value >> (i * 8));
}
static void zero_bytes(uint8_t *p, size_t length) { for (size_t i = 0; i < length; ++i) p[i] = 0; }

bool ghostos_vm_uefi_pe_section_at(const uint8_t *bytes, size_t length, const ghostos_vm_uefi_pe *pe, size_t index, ghostos_vm_uefi_pe_section *output) {
    if (index >= pe->section_count || index > (SIZE_MAX - pe->sections_start) / 40) return false;
    size_t offset = pe->sections_start + index * 40;
    if (!range_fits(length, offset, 40)) return false;
    uint32_t address = le32(bytes + offset + 12);
    uint32_t raw_size = le32(bytes + offset + 16);
    size_t raw_offset = le32(bytes + offset + 20);
    if (address > pe->size_of_image || raw_size > pe->size_of_image - address) return false;
    /* A zero-size section does not validate its raw pointer. */
    if (raw_size != 0 && !range_fits(length, raw_offset, raw_size)) return false;
    *output = (ghostos_vm_uefi_pe_section){ address, raw_offset, raw_size };
    return true;
}

uint32_t ghostos_vm_uefi_pe_parse(const uint8_t *bytes, size_t length, ghostos_vm_uefi_pe *output) {
    if (length < 0x40 || bytes[0] != 'M' || bytes[1] != 'Z') return 1;
    size_t pe_offset = le32(bytes + 0x3c);
    if (!range_fits(length, pe_offset, 24)) return 1;
    const uint8_t *pe = bytes + pe_offset;
    if (pe[0] != 'P' || pe[1] != 'E' || pe[2] != 0 || pe[3] != 0) return 1;
    const uint8_t *coff = pe + 4;
    if (le16(coff) != 0x8664) return 2;
    size_t sections = le16(coff + 2), optional_size = le16(coff + 16);
    size_t optional_offset = pe_offset + 24;
    if (optional_size < 112 || !range_fits(length, optional_offset, optional_size)) return 1;
    const uint8_t *optional = bytes + optional_offset;
    if (le16(optional) != 0x20b) return 2;
    ghostos_vm_uefi_pe image = {0};
    image.entry_rva = le32(optional + 16);
    image.image_base = le64(optional + 24);
    image.size_of_image = le32(optional + 56);
    if (image.size_of_image == 0 || image.entry_rva >= image.size_of_image) return 1;
    if (le32(optional + 108) > 5) {
        if (optional_size < 160) return 1;
        image.reloc_rva = le32(optional + 152);
        image.reloc_size = le32(optional + 156);
        if (image.reloc_size != 0 && (image.reloc_rva > image.size_of_image ||
            image.reloc_size > image.size_of_image - image.reloc_rva)) return 1;
    }
    image.sections_start = optional_offset + optional_size;
    image.section_count = sections;
    for (size_t i = 0; i < sections; ++i) {
        ghostos_vm_uefi_pe_section section;
        if (!ghostos_vm_uefi_pe_section_at(bytes, length, &image, i, &section)) return 1;
    }
    *output = image;
    return 0;
}

static bool address_add(uint64_t a, uint64_t b, bool checked, uint64_t *out) {
    if (checked && b > UINT64_MAX - a) return false;
    *out = a + b;
    return true;
}
static uint64_t read_value(const ghostos_vm_uefi_io *io, uint64_t address, uint32_t width) {
    uint64_t value = 0;
    return io->read_value(io->context, address, width, &value) ? value : 0;
}

uint32_t ghostos_vm_uefi_relocate(uint64_t base, uint64_t delta, uint32_t rva, uint32_t size, bool checked, const ghostos_vm_uefi_io *io) {
    uint64_t offset = 0;
    while (offset + 8 <= (uint64_t)size) {
        uint64_t address, field;
        if (!address_add(base, rva, checked, &address) || !address_add(address, offset, checked, &address)) return 5;
        uint64_t page = read_value(io, address, 4);
        if (!address_add(address, 4, checked, &field)) return 5;
        uint64_t block_size = read_value(io, field, 4);
        if (block_size < 8 || block_size > (uint64_t)size - offset) return 1;
        uint64_t count = (block_size - 8) / 2;
        for (uint64_t i = 0; i < count; ++i) {
            if (!address_add(address, 8, checked, &field) || !address_add(field, i * 2, checked, &field)) return 5;
            uint64_t entry = read_value(io, field, 2);
            uint64_t target;
            /* Preserve target arithmetic even for ignored relocation types. */
            if (!address_add(base, page, checked, &target) || !address_add(target, entry & 0xfff, checked, &target)) return 5;
            uint32_t type = (uint32_t)((entry >> 12) & 0xf);
            if (type == 10) {
                uint64_t value = read_value(io, target, 8) + delta;
                (void)io->write_value(io->context, target, 8, value);
            } else if (type == 3) {
                uint32_t value = (uint32_t)(read_value(io, target, 4) + delta);
                (void)io->write_value(io->context, target, 4, value);
            }
        }
        offset += block_size;
    }
    return 0;
}

uint32_t ghostos_vm_uefi_map_pe(const ghostos_vm_uefi_pe *image, const ghostos_vm_uefi_section *sections, size_t count,
    uint64_t requested, uint64_t memory_size, const ghostos_vm_uefi_image_range *loaded, size_t loaded_count,
    bool checked, const ghostos_vm_uefi_io *io) {
    if ((uint64_t)image->size_of_image > UINT64_MAX - requested) return 3;
    uint64_t end = requested + image->size_of_image;
    if (requested % 4096 != 0 || end > memory_size) return 3;
    for (size_t i = 0; i < loaded_count; ++i) {
        uint64_t loaded_end;
        if (!address_add(loaded[i].base, loaded[i].size, checked, &loaded_end)) return 5;
        if (requested < loaded_end && loaded[i].base < end) return 3;
    }
    const uint8_t zeros[4096] = {0};
    for (uint64_t offset = 0; offset < image->size_of_image;) {
        uint64_t remaining = image->size_of_image - offset;
        size_t length = remaining < sizeof(zeros) ? (size_t)remaining : sizeof(zeros);
        if (!io->write_phys(io->context, requested + offset, zeros, length)) return 4;
        offset += length;
    }
    for (size_t i = 0; i < count; ++i) {
        if (sections[i].length == 0) continue;
        uint64_t destination;
        if (!address_add(requested, sections[i].virtual_address, checked, &destination)) return 5;
        if (!io->write_phys(io->context, destination, sections[i].bytes, sections[i].length)) return 4;
    }
    if (requested != image->image_base) {
        if (image->reloc_size == 0) return 1;
        return ghostos_vm_uefi_relocate(requested, requested - image->image_base, image->reloc_rva, image->reloc_size, checked, io);
    }
    return 0;
}

static void memory_descriptor(uint8_t *out, uint32_t type, uint64_t start, uint64_t length, uint64_t attributes) {
    zero_bytes(out, 48);
    put_le(out, type, 4);
    put_le(out + 8, start, 8);
    put_le(out + 16, start, 8);
    put_le(out + 24, length / 4096, 8);
    put_le(out + 32, attributes, 8);
}
static size_t sorted_image(const ghostos_vm_uefi_image_range *images, size_t count, size_t rank) {
    /* No allocation: stable ranking keeps equal bases in original order. */
    for (size_t i = 0; i < count; ++i) {
        size_t before = 0;
        for (size_t j = 0; j < count; ++j)
            if (images[j].base < images[i].base || (images[j].base == images[i].base && j < i)) ++before;
        if (before == rank) return i;
    }
    return 0;
}
static size_t encode_map(uint64_t memory_size, const ghostos_vm_uefi_image_range *images, size_t count, uint8_t *out) {
    size_t used = 0;
#define EMIT(type, start, length, attr) do { if (out != NULL) memory_descriptor(out + used * 48, type, start, length, attr); ++used; } while (0)
    EMIT(7, 0, 0xa0000, 0xf);
    EMIT(0, 0xa0000, 0x60000, 0);
    EMIT(1, 0x100000, 0x2000000 - 0x100000, 0xf);
    uint64_t cursor = 0x2000000;
    for (size_t rank = 0; rank < count; ++rank) {
        const ghostos_vm_uefi_image_range *image = &images[sorted_image(images, count, rank)];
        if (image->base > cursor) { EMIT(7, cursor, image->base - cursor, 0xf); }
        EMIT(2, image->base, image->size, 0xf);
        uint64_t end = image->size > UINT64_MAX - image->base ? UINT64_MAX : image->base + image->size;
        if (cursor < end) cursor = end;
    }
    if (cursor < memory_size) { EMIT(7, cursor, memory_size - cursor, 0xf); }
#undef EMIT
    return used;
}
size_t ghostos_vm_uefi_memory_map(uint64_t memory_size, const ghostos_vm_uefi_image_range *images, size_t count, uint8_t *output, size_t capacity) {
    size_t needed = encode_map(memory_size, images, count, NULL);
    if (output != NULL && capacity >= needed) (void)encode_map(memory_size, images, count, output);
    return needed;
}

void ghostos_vm_uefi_service_stub(uint64_t id, uint8_t output[8]) {
    output[0] = 0xb8; put_le(output + 1, (uint32_t)id, 4);
    output[5] = 0xcd; output[6] = 0xe0; output[7] = 0xc3;
}
void ghostos_vm_uefi_table_header(uint64_t signature, uint8_t output[24]) {
    zero_bytes(output, 24);
    put_le(output, signature, 8); put_le(output + 8, 0x2000f, 4); put_le(output + 12, 24, 4);
}
void ghostos_vm_uefi_rsdp(uint8_t output[36]) {
    static const uint8_t signature[8] = {'R', 'S', 'D', ' ', 'P', 'T', 'R', ' '};
    zero_bytes(output, 36);
    for (size_t i = 0; i < 8; ++i) output[i] = signature[i];
    output[8] = 2; output[15] = 36;
    uint8_t sum = 0;
    for (size_t i = 0; i < 20; ++i) sum = (uint8_t)(sum + output[i]);
    output[9] = (uint8_t)(0u - sum);
    sum = 0;
    for (size_t i = 0; i < 36; ++i) sum = (uint8_t)(sum + output[i]);
    output[32] = (uint8_t)(0u - sum);
}
