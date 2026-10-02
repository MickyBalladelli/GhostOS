#include "ghostos/vm_nvme.h"
#include "ghostos/vm_ahci.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

/* In-memory adapters replace the original temporary eight-sector images. */
typedef struct { uint8_t memory[0x20000], disk[4096]; bool attached; } fixture;
static bool read_memory(void *raw, uint64_t address, uint8_t *out, size_t length) {
    fixture *f = raw;
    if (address > sizeof(f->memory) || length > sizeof(f->memory) - address) return false;
    memcpy(out, f->memory + address, length); return true;
}
static bool write_memory(void *raw, uint64_t address, const uint8_t *bytes, size_t length) {
    fixture *f = raw;
    if (address > sizeof(f->memory) || length > sizeof(f->memory) - address) return false;
    memcpy(f->memory + address, bytes, length); return true;
}
static bool validate_dma(void *raw, uint64_t address, size_t length, uint64_t alignment) {
    fixture *f = raw;
    return length && (!alignment || address % alignment == 0)
        && address <= sizeof(f->memory) && length <= sizeof(f->memory) - address;
}
static bool sector_count(void *raw, uint64_t *count) {
    fixture *f = raw; if (!f->attached) return false; *count = 8; return true;
}
static bool read_sector(void *raw, uint64_t sector, uint8_t out[512]) {
    fixture *f = raw; if (!f->attached || sector >= 8) return false;
    memcpy(out, f->disk + sector * 512, 512); return true;
}
static bool write_sector(void *raw, uint64_t sector, const uint8_t bytes[512]) {
    fixture *f = raw; if (!f->attached || sector >= 8) return false;
    memcpy(f->disk + sector * 512, bytes, 512); return true;
}
static bool flush_disk(void *raw) { return ((fixture *)raw)->attached; }
static ghostos_vm_storage_io adapters(fixture *f) {
    return (ghostos_vm_storage_io){read_memory, write_memory, validate_dma, sector_count,
        read_sector, write_sector, flush_disk, NULL, f};
}
static void put32(uint8_t *p, uint32_t value) {
    for (size_t i = 0; i < 4; ++i) p[i] = (uint8_t)(value >> (i * 8));
}
static void command(uint8_t cmd[64], uint8_t opcode, uint64_t buffer, uint64_t sector, uint16_t count) {
    memset(cmd, 0, 64); cmd[0] = opcode; put32(cmd + 4, 1);
    for (size_t i = 0; i < 8; ++i) cmd[8 + i] = (uint8_t)(buffer >> (i * 8));
    put32(cmd + 40, (uint32_t)sector);
    put32(cmd + 44, ((uint32_t)(count - 1) << 16) | (uint32_t)(sector >> 32));
}

/* Port of controller_registers_identify_and_io_round_trip. */
static void nvme_round_trip(void) {
    fixture f = {.attached = true}; ghostos_vm_storage_io io = adapters(&f);
    ghostos_vm_nvme *n = ghostos_vm_nvme_new(true); assert(n);
    uint64_t value;
    assert(ghostos_vm_nvme_read(n, 8, 4, &value) == 0 && value == 0x00010300);
    assert(ghostos_vm_nvme_read(n, 0, 8, &value) == 0 && (value & (UINT64_C(1) << 37)));
    uint8_t cmd[64], payload[512]; memset(payload, 0x5a, sizeof(payload));
    memcpy(f.memory + 0x2000, payload, sizeof(payload));
    uint32_t status;
    command(cmd, 1, 0x2000, 1, 1);
    assert(ghostos_vm_nvme_command(n, &io, cmd, &status) == 0 && status == 0);
    memset(f.memory + 0x2000, 0, sizeof(payload));
    command(cmd, 2, 0x2000, 1, 1);
    assert(ghostos_vm_nvme_command(n, &io, cmd, &status) == 0 && status == 0);
    assert(memcmp(f.memory + 0x2000, payload, sizeof(payload)) == 0);
    command(cmd, 6, 0x3000, 0, 1);
    assert(ghostos_vm_nvme_admin(n, &io, cmd, &status) == 0 && status == 0);
    const uint8_t eight[8] = {8}; assert(memcmp(f.memory + 0x3000, eight, 8) == 0);
    ghostos_vm_nvme_free(n);
}

/* Port of invalid_namespace_and_bounds_fail_cleanly. */
static void nvme_bounds(void) {
    fixture f = {0}; ghostos_vm_storage_io io = adapters(&f);
    ghostos_vm_nvme *n = ghostos_vm_nvme_new(true); assert(n);
    uint8_t cmd[64]; uint32_t status; uint64_t value;
    command(cmd, 2, 0x2000, 0, 1); put32(cmd + 4, 2);
    assert(ghostos_vm_nvme_command(n, &io, cmd, &status) == 0 && status == 11);
    f.attached = true; command(cmd, 2, 0x2000, 8, 1);
    assert(ghostos_vm_nvme_command(n, &io, cmd, &status) == 0 && status == 128);
    assert(ghostos_vm_nvme_read(n, 8, 2, &value) == GHOSTOS_VM_NVME_SIZE);
    ghostos_vm_nvme_reset(n);
    assert(sector_count(&f, &value) && value == 8);
    ghostos_vm_nvme_free(n);
}

/* Port of identification_and_dma_round_trip. */
static void ahci_round_trip(void) {
    fixture f = {.attached = true}; ghostos_vm_storage_io io = adapters(&f);
    ghostos_vm_ahci *a = ghostos_vm_ahci_new(true); assert(a);
    uint64_t count; assert(sector_count(&f, &count) && count == 8);
    uint8_t identify[512]; ghostos_vm_ahci_identify(count, identify);
    assert(memcmp(identify + 46, "GhostOS Virtual Disk", 20) == 0);
    uint8_t sector[512]; memset(sector, 0xa5, sizeof(sector));
    memcpy(f.memory + 0x1000, sector, sizeof(sector));
    ghostos_vm_ahci_prd prd = {0x1000, 512};
    assert(ghostos_vm_ahci_transfer(a, &io, 2, 1, &prd, 1, true) == 0);
    prd.address = 0x2000;
    assert(ghostos_vm_ahci_transfer(a, &io, 2, 1, &prd, 1, false) == 0);
    assert(memcmp(f.memory + 0x2000, sector, sizeof(sector)) == 0);
    ghostos_vm_ahci_free(a);
}

/* Port of register_access_rejects_bad_size_and_reset_keeps_disk. */
static void ahci_reset(void) {
    fixture f = {.attached = true};
    ghostos_vm_ahci *a = ghostos_vm_ahci_new(true); assert(a);
    uint64_t value;
    assert(ghostos_vm_ahci_read(a, 0, 2, &value) == GHOSTOS_VM_AHCI_SIZE);
    assert(ghostos_vm_ahci_read(a, 0, 4, &value) == 0 && value == 0x80020301);
    ghostos_vm_ahci_reset(a);
    assert(sector_count(&f, &value) && value == 8);
    assert(!ghostos_vm_ahci_pending(a));
    ghostos_vm_ahci_free(a);
}

int main(void) {
    nvme_round_trip(); nvme_bounds(); ahci_round_trip(); ahci_reset();
    puts("C VM storage contracts passed"); return 0;
}
