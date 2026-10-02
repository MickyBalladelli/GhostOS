#include "ghostos/vm_bios.h"
#include <string.h>

void ghostos_vm_bios_rom(uint8_t output[65536]) {
    memset(output, 0, 65536);
    const uint8_t entry[] = {0xfa,0x66,0x31,0xc0,0x8e,0xd8,0x8e,0xc0,0x8e,0xd0,0x66,0xbc,0,0x7c,0xfb,0xcd,0x19,0xf4};
    const uint8_t reset[] = {0xea,0x5b,0xe0,0,0xf0};
    memcpy(output + 0xe05b, entry, sizeof(entry));
    memcpy(output + 0xfff0, reset, sizeof(reset));
    output[0xfffe] = 0x55; output[0xffff] = 0xaa;
}

void ghostos_vm_bios_post_tables(uint8_t ivt[1024], uint8_t bda[256]) {
    for (size_t i = 0; i < 1024; i += 4) {
        ivt[i] = 0x53; ivt[i + 1] = 0xff; ivt[i + 2] = 0; ivt[i + 3] = 0xf0;
    }
    memset(bda, 0, 256);
    bda[0x80] = 3; bda[0x81] = 0xf8; bda[0x10] = 0x21;
    bda[0x13] = 0x80; bda[0x14] = 2; bda[0x49] = 3;
    bda[0x4a] = 80; bda[0x4c] = 0xa0; bda[0x4d] = 0xf;
}

static void ah(ghostos_vm_bios_registers *r, uint8_t value) {
    r->rax = (r->rax & ~UINT64_C(0xff00)) | ((uint64_t)value << 8);
}
static void fail(ghostos_vm_bios_registers *r, uint8_t value) { r->rflags |= 1; ah(r, value); }
static uint64_t le(const uint8_t *bytes, unsigned count) {
    uint64_t value = 0;
    for (unsigned i = 0; i < count; ++i) value |= (uint64_t)bytes[i] << (i * 8);
    return value;
}
static void put(uint8_t *bytes, uint64_t value, unsigned count) {
    for (unsigned i = 0; i < count; ++i) bytes[i] = (uint8_t)(value >> (i * 8));
}

static void disk_read(ghostos_vm_bios_registers *r, const uint8_t *image,
    size_t length, bool edd, const ghostos_vm_bios_io *io) {
    if (!image) { fail(r, 0x80); return; }
    uint64_t lba, count, destination;
    if (edd) {
        uint8_t header[16];
        uint64_t dap = ((uint64_t)r->ds << 4) + (r->rsi & 0xffff);
        if (!io->read(io->context, dap, header, 16) || header[0] < 0x10) { fail(r, 1); return; }
        count = header[2];
        destination = (le(header + 4, 2) << 4) + le(header + 6, 2);
        lba = le(header + 8, 8);
    } else {
        uint64_t cylinder = ((r->rcx >> 8) & 0xff) | ((r->rcx & 0xc0) << 2);
        uint64_t sector = r->rcx & 0x3f;
        lba = (cylinder * 16 + ((r->rdx >> 8) & 0xff)) * 63 + (sector ? sector - 1 : 0);
        count = r->rax & 0xff;
        destination = ((uint64_t)r->es << 4) + (r->rbx & 0xffff);
    }
    uint64_t available = length / 512;
    uint64_t remaining = available > lba ? available - lba : 0;
    if (count > remaining) count = remaining;
    if (!count) { fail(r, 2); return; }
    bool written = io->write(io->context, destination, image + (size_t)lba * 512, (size_t)count * 512);
    if (edd && !written) { fail(r, 9); return; }
    r->rflags &= ~UINT64_C(1); ah(r, 0);
}

static void e820(ghostos_vm_bios_registers *r, uint64_t memory, const ghostos_vm_bios_io *io) {
    uint64_t size = r->rcx & UINT32_MAX, index = r->rbx & UINT32_MAX;
    if ((r->rdx & UINT32_MAX) != UINT32_C(0x534d4150) || size < 20) { fail(r, 0x86); return; }
    if (index >= 3) { r->rbx = 0; ah(r, 0); return; }
    uint64_t base = index == 0 ? 0 : index == 1 ? 0xe0000 : 0x100000;
    uint64_t length = index == 0 ? (memory < 0xe0000 ? memory : 0xe0000) :
        index == 1 ? 0x20000 : (memory > 0x100000 ? memory - 0x100000 : 0);
    uint8_t entry[24];
    put(entry, base, 8); put(entry + 8, length, 8);
    put(entry + 16, index == 1 ? 2 : 1, 4); put(entry + 20, 1, 4);
    uint64_t destination = ((uint64_t)r->es << 4) + (r->rdi & 0xffff);
    if (!io->write(io->context, destination, entry, size < 24 ? (size_t)size : 24)) { fail(r, 0x86); return; }
    r->rbx = index + 1; ah(r, 0); r->rflags &= ~UINT64_C(1);
}

void ghostos_vm_bios_service(uint8_t vector, ghostos_vm_bios_registers *r,
    const uint8_t *image, size_t length, uint64_t memory_size, const ghostos_vm_bios_io *io) {
    uint8_t function = (uint8_t)(r->rax >> 8);
    if (vector == 0x16) {
        if (function == 0) { r->rax &= ~UINT64_C(0xffff); r->rflags |= 1u << 6; }
        else if (function == 1) r->rflags |= 1u << 6;
        else r->rax &= ~UINT64_C(0xff);
        return;
    }
    if (vector != 0x13 && vector != 0x15) return;
    r->rflags &= ~UINT64_C(1);
    if (vector == 0x15) {
        switch (function) {
            case 0x20: case 0x86: case 0xc0: ah(r, 0); break;
            case 0x88: {
                uint64_t kb = (memory_size > 0x100000 ? memory_size - 0x100000 : 0) / 1024;
                if (kb > 0xffff) kb = 0xffff;
                r->rax = (r->rax & ~UINT64_C(0xffff)) | kb;
                break;
            }
            case 0xe8: case 0xe9: e820(r, memory_size, io); break;
            default: fail(r, 0x86); break;
        }
        return;
    }
    switch (function) {
        case 0: case 1: case 0x0e: ah(r, 0); break;
        case 2: disk_read(r, image, length, false, io); break;
        case 8:
            ah(r, 0); r->rdx = (r->rdx & ~UINT64_C(0xffff)) | 0x0f01;
            r->rcx = (r->rcx & ~UINT64_C(0xffff)) | 62; break;
        case 0x15: {
            ah(r, 3); uint64_t sectors = length / 512;
            r->rcx = (r->rcx & ~UINT64_C(0xffff)) | (sectors & 0xffff);
            r->rdx = (r->rdx & ~UINT64_C(0xffff)) | ((sectors >> 16) & 0xffff); break;
        }
        case 0x41:
            if (!(r->rdx & 0x80)) { fail(r, 1); break; }
            ah(r, 0x30); r->rbx = (r->rbx & ~UINT64_C(0xffff)) | 0xaa55;
            r->rcx = (r->rcx & ~UINT64_C(0xffff)) | 3; break;
        case 0x42: disk_read(r, image, length, true, io); break;
        default: fail(r, 1); break;
    }
}
