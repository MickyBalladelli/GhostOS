#include "ghostos/vm_ahci.h"
#include <stdlib.h>
#include <string.h>

#define CAP UINT32_C(0x80020301)
#define GHC_AE (UINT32_C(1) << 31)
#define GHC_IE UINT32_C(2)
#define CMD_CR (UINT32_C(1) << 15)
#define MAX_SECTORS (16u * 1024u * 1024u / 512u)

struct ghostos_vm_ahci {
    uint64_t clb, fb;
    uint32_t ghc, host_is, port_is, ie, cmd, tfd, sctl, serr, sact, ci, pending;
    bool checked;
};
static uint32_t get32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}
static uint64_t get64(const uint8_t *p) { return get32(p) | (uint64_t)get32(p + 4) << 32; }
static void word(uint8_t *p, size_t index, uint16_t v) {
    p[index * 2] = (uint8_t)v; p[index * 2 + 1] = (uint8_t)(v >> 8);
}
ghostos_vm_ahci *ghostos_vm_ahci_new(bool checked) {
    ghostos_vm_ahci *a = calloc(1, sizeof(*a));
    if (a) a->checked = checked;
    return a;
}
void ghostos_vm_ahci_free(ghostos_vm_ahci *a) { free(a); }
void ghostos_vm_ahci_reset(ghostos_vm_ahci *a) {
    bool checked = a->checked;
    memset(a, 0, sizeof(*a)); a->checked = checked;
}
bool ghostos_vm_ahci_pending(const ghostos_vm_ahci *a) { return a->pending != 0; }
static uint32_t read_host(const ghostos_vm_ahci *a, uint32_t off) {
    switch (off) {
        case 0: return CAP;
        case 4: return a->ghc;
        case 8: return a->host_is;
        case 0xc: return 1;
        case 0x10: return 0x00010200;
        default: return 0;
    }
}
static uint32_t read_port(const ghostos_vm_ahci *a, uint32_t off) {
    switch (off) {
        case 0: return (uint32_t)a->clb;
        case 4: return (uint32_t)(a->clb >> 32);
        case 8: return (uint32_t)a->fb;
        case 0xc: return (uint32_t)(a->fb >> 32);
        case 0x10: return a->port_is;
        case 0x14: return a->ie;
        case 0x18: return a->cmd;
        case 0x20: return a->tfd;
        case 0x24: return 0x101;
        case 0x28: return 0x123;
        case 0x2c: return a->sctl;
        case 0x30: return a->serr;
        case 0x34: return a->sact;
        case 0x38: return a->ci;
        default: return 0;
    }
}
uint32_t ghostos_vm_ahci_read(const ghostos_vm_ahci *a, uint64_t address, uint8_t size, uint64_t *value) {
    if (size != 4) return GHOSTOS_VM_AHCI_SIZE;
    uint32_t off = (uint32_t)(address & 0xfff);
    *value = off < 0x100 ? read_host(a, off) : off < 0x180 ? read_port(a, off - 0x100) : 0;
    return 0;
}
uint32_t ghostos_vm_ahci_write(ghostos_vm_ahci *a, uint64_t address, uint8_t size, uint32_t value) {
    if (size != 4) return GHOSTOS_VM_AHCI_SIZE;
    uint32_t off = (uint32_t)(address & 0xfff);
    if (off < 0x100) {
        if (off == 4) {
            if (value & 1) ghostos_vm_ahci_reset(a);
            else a->ghc = (a->ghc & ~(GHC_AE | GHC_IE)) | (value & (GHC_AE | GHC_IE));
        } else if (off == 8) { a->host_is &= ~value; a->port_is &= ~value; }
    } else if (off < 0x180) {
        switch (off - 0x100) {
            case 0: a->clb = (a->clb & ~UINT64_C(0xffffffff)) | value; break;
            case 4: a->clb = (a->clb & UINT32_MAX) | (uint64_t)value << 32; break;
            case 8: a->fb = (a->fb & ~UINT64_C(0xffffffff)) | value; break;
            case 0xc: a->fb = (a->fb & UINT32_MAX) | (uint64_t)value << 32; break;
            case 0x10: a->port_is &= ~value; a->host_is &= ~value; break;
            case 0x14: a->ie = value; break;
            case 0x18: a->cmd = (a->cmd & CMD_CR) | (value & ~CMD_CR); break;
            case 0x2c: a->sctl = value & 0xfff; break;
            case 0x30: a->serr &= ~value; break;
            case 0x34: a->sact = value; break;
            case 0x38: a->pending |= value & ~a->ci; a->ci |= value; break;
            default: break;
        }
    }
    return 0;
}

void ghostos_vm_ahci_identify(uint64_t sectors, uint8_t id[512]) {
    if (sectors > 0x0fffffff) sectors = 0x0fffffff;
    memset(id, 0, 512);
    word(id, 0, 0x40); word(id, 1, 16383); word(id, 3, 16); word(id, 6, 63);
    memcpy(id + 20, "SYNOSVM00001", 12);
    memcpy(id + 46, "GhostOS Virtual Disk", 20);
    word(id, 47, 0x8001); word(id, 49, 0x0f00); word(id, 50, 0x4000);
    word(id, 60, (uint16_t)sectors); word(id, 61, (uint16_t)(sectors >> 16));
    word(id, 62, 7); word(id, 63, 7); word(id, 80, 0x7e); word(id, 83, 0x4000);
    for (size_t i = 0; i < 4; ++i) word(id, 100 + i, (uint16_t)(sectors >> (i * 16)));
    word(id, 255, 0xa5a5);
}
uint32_t ghostos_vm_ahci_transfer(ghostos_vm_ahci *a, const ghostos_vm_storage_io *io, uint64_t lba,
    size_t count, const ghostos_vm_ahci_prd *prds, size_t prd_count, bool to_disk) {
    uint64_t sectors;
    if (!io->sector_count(io->context, &sectors)) return GHOSTOS_VM_AHCI_DISK;
    if (count > MAX_SECTORS) return GHOSTOS_VM_AHCI_DMA;
    uint8_t sector[512] = {0};
    size_t index = 0, expected = count * 512;
    for (size_t i = 0; i < prd_count; ++i) {
        size_t remaining = prds[i].length;
        uint64_t address = prds[i].address;
        while (remaining && index < expected) {
            size_t n = remaining < 512 ? remaining : 512, off = index % 512;
            /* Preserve the old slice-panic ordering, without C out-of-bounds. */
            if (to_disk) {
                if (off + n > 512) return GHOSTOS_VM_AHCI_LEGACY_BOUNDS;
                if (!io->read_memory(io->context, address, sector + off, n)) return GHOSTOS_VM_AHCI_DMA;
                if (off + n == 512) {
                    if (a->checked && lba > UINT64_MAX - index / 512) return GHOSTOS_VM_AHCI_LEGACY_OVERFLOW;
                    if (!io->write_sector(io->context, lba + index / 512, sector)) return GHOSTOS_VM_AHCI_DISK;
                }
            } else {
                if (!off) {
                    if (a->checked && lba > UINT64_MAX - index / 512) return GHOSTOS_VM_AHCI_LEGACY_OVERFLOW;
                    if (!io->read_sector(io->context, lba + index / 512, sector)) return GHOSTOS_VM_AHCI_DISK;
                }
                if (off + n > 512) return GHOSTOS_VM_AHCI_LEGACY_BOUNDS;
                if (!io->write_memory(io->context, address, sector + off, n)) return GHOSTOS_VM_AHCI_DMA;
            }
            if (address > UINT64_MAX - n) return GHOSTOS_VM_AHCI_DMA;
            address += n; remaining -= n; index += n;
        }
        if (index >= expected) break;
    }
    return index == expected ? 0 : GHOSTOS_VM_AHCI_DMA;
}
static uint32_t write_fis(const ghostos_vm_ahci *a, const ghostos_vm_storage_io *io, uint8_t type, uint64_t offset) {
    uint8_t fis[28] = {0}; fis[0] = type; fis[1] = 2; fis[2] = 0x50; fis[12] = 1;
    if (a->fb > UINT64_MAX - offset || !io->write_memory(io->context, a->fb + offset, fis, sizeof(fis))) return GHOSTOS_VM_AHCI_DMA;
    return 0;
}
static uint32_t process_slot(ghostos_vm_ahci *a, const ghostos_vm_storage_io *io, size_t slot, uint32_t *extra) {
    *extra = 0;
    if (a->clb % 1024 || a->fb % 256 || a->clb > UINT64_MAX - slot * 32) return GHOSTOS_VM_AHCI_DMA;
    uint8_t header[32] = {0}, cfis[64] = {0};
    if (!io->read_memory(io->context, a->clb + slot * 32, header, sizeof(header))) return GHOSTOS_VM_AHCI_DMA;
    uint32_t dw0 = get32(header);
    size_t prdtl = dw0 >> 16;
    if (!prdtl || prdtl > 256 || (dw0 & 32)) return GHOSTOS_VM_AHCI_DMA;
    uint64_t ctba = get64(header + 8);
    if (ctba % 128 || !io->read_memory(io->context, ctba, cfis, sizeof(cfis)) || cfis[0] != 0x27) return GHOSTOS_VM_AHCI_DMA;
    uint8_t command = cfis[2];
    size_t count = (size_t)cfis[12] | (size_t)cfis[13] << 8;
    uint64_t lba = (uint64_t)cfis[4] | (uint64_t)cfis[5] << 8 | (uint64_t)cfis[6] << 16
        | (uint64_t)cfis[8] << 24 | (uint64_t)cfis[9] << 32 | (uint64_t)cfis[10] << 40 | (uint64_t)cfis[11] << 48;
    ghostos_vm_ahci_prd prds[256];
    for (size_t i = 0; i < prdtl; ++i) {
        uint8_t prd[16] = {0};
        uint64_t offset = 0x80 + i * 16;
        if (ctba > UINT64_MAX - offset || !io->read_memory(io->context, ctba + offset, prd, sizeof(prd))) return GHOSTOS_VM_AHCI_DMA;
        prds[i] = (ghostos_vm_ahci_prd){get64(prd), (get32(prd + 12) & 0x003fffff) + 1};
    }
    if (command == 0xec) {
        uint8_t identify[512]; uint64_t sectors = 0;
        (void)io->sector_count(io->context, &sectors);
        ghostos_vm_ahci_identify(sectors, identify);
        size_t done = 0;
        for (size_t i = 0; i < prdtl; ++i) {
            size_t n = prds[i].length < 512 - done ? prds[i].length : 512 - done;
            if (!io->write_memory(io->context, prds[i].address, identify + done, n)) return GHOSTOS_VM_AHCI_DMA;
            done += n;
            if (done >= 512) break;
        }
        if (done != 512) return GHOSTOS_VM_AHCI_DMA;
        uint32_t result = write_fis(a, io, 0x34, 0x40);
        if (result) return result;
        result = write_fis(a, io, 0x5f, 0x20);
        if (!result) *extra = 32;
        return result;
    }
    bool read = command == 0x20 || command == 0x24 || command == 0xc8 || command == 0x25;
    bool write = command == 0x30 || command == 0x34 || command == 0xca || command == 0x35;
    if (read || write) {
        if (!count) count = 256;
        if (count > MAX_SECTORS) return GHOSTOS_VM_AHCI_DMA;
        uint32_t result = ghostos_vm_ahci_transfer(a, io, lba, count, prds, prdtl, write);
        if (result) return result;
        result = write_fis(a, io, 0x34, 0x40);
        if (!result && read) *extra = 32;
        return result;
    }
    if (command == 0xe7 || command == 0xea || command == 0xef || command == 0xc6) {
        if ((command == 0xe7 || command == 0xea) && !io->flush_disk(io->context)) return GHOSTOS_VM_AHCI_DISK;
        return write_fis(a, io, 0x34, 0x40);
    }
    return GHOSTOS_VM_AHCI_DMA;
}
uint32_t ghostos_vm_ahci_poll(ghostos_vm_ahci *a, const ghostos_vm_storage_io *io) {
    uint32_t issued = a->pending, raise = 0;
    a->pending = 0;
    if (!issued) return 0;
    for (size_t slot = 0; slot < 32; ++slot) {
        if (!(issued & (UINT32_C(1) << slot))) continue;
        uint32_t extra, result = process_slot(a, io, slot, &extra);
        if (result == GHOSTOS_VM_AHCI_LEGACY_BOUNDS || result == GHOSTOS_VM_AHCI_LEGACY_OVERFLOW) return result;
        if (result) { a->tfd = 1; a->serr |= 1; raise |= 1; }
        else raise |= extra | 1;
    }
    a->ci &= ~issued; a->host_is |= raise; a->port_is |= raise;
    if ((a->ghc & GHC_AE) && (a->ghc & GHC_IE) && (a->port_is & a->ie) && io->interrupt) io->interrupt(io->context);
    return 0;
}
