#include "ghostos/vm_disk_image.h"
#include <stdlib.h>
#include <string.h>

#define OFFSET_MASK UINT64_C(0x00fffffffffffe00)
struct ghostos_vm_disk_image {
    uint32_t format, cluster_bits, l2_bits, l1_size, l2_entries;
    uint64_t size, cluster_size, l1_offset, l2_offset;
    uint64_t *l1, *l2;
    bool writable, checked;
};
static uint32_t be32(const uint8_t *p) {
    return (uint32_t)p[0] << 24 | (uint32_t)p[1] << 16 | (uint32_t)p[2] << 8 | p[3];
}
static uint64_t be64(const uint8_t *p) { return (uint64_t)be32(p) << 32 | be32(p + 4); }
static void put64(uint8_t *p, uint64_t v) { for (size_t i = 0; i < 8; ++i) p[i] = (uint8_t)(v >> (56 - i * 8)); }
static bool fail(ghostos_vm_disk_error *e, uint32_t code) { e->code = code; return false; }
static bool seek(const ghostos_vm_disk_file_io *io, uint32_t origin, uint64_t offset, ghostos_vm_disk_error *e) {
    uint64_t position;
    return io->seek(io->context, origin, offset, &position) || fail(e, 1);
}
static bool read(const ghostos_vm_disk_file_io *io, uint8_t *out, size_t length, ghostos_vm_disk_error *e) {
    return io->read(io->context, out, length) || fail(e, 1);
}
static bool write(const ghostos_vm_disk_file_io *io, const uint8_t *bytes, size_t length, ghostos_vm_disk_error *e) {
    return io->write(io->context, bytes, length) || fail(e, 1);
}
static bool length(const ghostos_vm_disk_file_io *io, uint64_t *value, ghostos_vm_disk_error *e) {
    return io->length(io->context, value) || fail(e, 1);
}
static bool capacity(uint64_t size, uint32_t format, ghostos_vm_disk_error *e) {
    if (size && size % 512 == 0) return true;
    e->value = size; e->format = format; return fail(e, 14);
}
static bool data_entry(uint64_t entry, uint64_t cluster, uint64_t file_length, ghostos_vm_disk_error *e) {
    if (entry & (UINT64_C(1) << 62)) return fail(e, 32);
    uint64_t offset = entry & OFFSET_MASK;
    if (offset && (offset % cluster || offset > UINT64_MAX - cluster || offset + cluster > file_length)) return fail(e, 33);
    return true;
}
static bool validate_l2(const ghostos_vm_disk_file_io *io, uint64_t offset, uint64_t cluster,
    uint32_t entries, uint64_t file_length, ghostos_vm_disk_error *e) {
    if (!seek(io, 0, offset, e)) return false;
    for (uint32_t i = 0; i < entries; ++i) {
        uint8_t bytes[8] = {0};
        if (!read(io, bytes, 8, e) || !data_entry(be64(bytes), cluster, file_length, e)) return false;
    }
    return true;
}
static bool open_vhd(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, ghostos_vm_disk_error *e) {
    uint64_t file_length;
    if (!length(io, &file_length, e)) return false;
    if (file_length < 1024) return fail(e, 11);
    uint8_t footer[512] = {0};
    if (!seek(io, 1, (uint64_t)-512, e) || !read(io, footer, 512, e)) return false;
    uint32_t original = be32(footer + 36), current = be32(footer + 40), type = be32(footer + 48);
    if (type != 2) { e->value = type; return fail(e, 12); }
    if (original != current) return fail(e, 13);
    d->size = current;
    if (!capacity(d->size, 1, e)) return false;
    if (file_length != d->size + 512) { e->value = file_length; e->other = d->size; return fail(e, 15); }
    uint32_t sum = 0;
    for (size_t i = 0; i < 512; ++i) if (i < 52 || i >= 56) sum += footer[i];
    if (~sum != be32(footer + 52)) return fail(e, 16);
    return seek(io, 0, 0, e);
}
static bool open_qcow(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, ghostos_vm_disk_error *e) {
    uint64_t file_length;
    if (!length(io, &file_length, e)) return false;
    if (file_length < 104) return fail(e, 17);
    uint8_t h[104] = {0};
    if (!seek(io, 0, 0, e) || !read(io, h, sizeof(h), e)) return false;
    uint32_t version = be32(h + 4);
    if (version != 2 && version != 3) { e->value = version; return fail(e, 18); }
    if (be64(h + 8) || be32(h + 16)) return fail(e, 19);
    if (be32(h + 32)) return fail(e, 20);
    d->cluster_bits = be32(h + 20);
    if (d->cluster_bits < 9 || d->cluster_bits > 21) { e->value = d->cluster_bits; return fail(e, 21); }
    uint64_t incompatible = be64(h + 72);
    if (incompatible) { e->value = incompatible; return fail(e, 22); }
    d->l1_size = be32(h + 36);
    if (!d->l1_size || d->l1_size > (1u << 22)) return fail(e, 23);
    d->cluster_size = UINT64_C(1) << d->cluster_bits;
    d->size = be64(h + 24);
    if (!capacity(d->size, 2, e)) return false;
    if (version == 3 && (be32(h + 100) < 104 || be32(h + 100) > file_length)) return fail(e, 24);
    d->l2_entries = (uint32_t)(d->cluster_size / 8);
    while ((UINT64_C(1) << d->l2_bits) < d->l2_entries) ++d->l2_bits;
    uint64_t coverage = d->cluster_size * d->l2_entries;
    d->l1_offset = be64(h + 40);
    if (d->l1_offset % d->cluster_size) return fail(e, 25);
    uint64_t l1_bytes = (uint64_t)d->l1_size * 8;
    if (d->l1_offset > UINT64_MAX - l1_bytes) return fail(e, 27);
    if (d->l1_offset < d->cluster_size || d->l1_offset + l1_bytes > file_length) return fail(e, 28);
    if (coverage > UINT64_MAX / d->l1_size || coverage * d->l1_size < d->size) return fail(e, 29);
    d->l1 = calloc(d->l1_size, sizeof(*d->l1));
    if (!d->l1) return fail(e, 4);
    if (!seek(io, 0, d->l1_offset, e)) return false;
    for (uint32_t i = 0; i < d->l1_size; ++i) {
        uint8_t bytes[8] = {0};
        if (!read(io, bytes, 8, e)) return false;
        d->l1[i] = be64(bytes);
    }
    for (uint32_t i = 0; i < d->l1_size; ++i) {
        uint64_t entry = d->l1[i], offset = entry & OFFSET_MASK;
        if (entry && !offset) return fail(e, 30);
        if (offset && (offset % d->cluster_size || offset > UINT64_MAX - d->cluster_size
            || offset + d->cluster_size > file_length)) return fail(e, 31);
        if (offset && !validate_l2(io, offset, d->cluster_size, d->l2_entries, file_length, e)) return false;
    }
    return true;
}
ghostos_vm_disk_image *ghostos_vm_disk_image_open(const ghostos_vm_disk_file_io *io,
    bool writable, bool checked, ghostos_vm_disk_error *e) {
    memset(e, 0, sizeof(*e));
    uint64_t file_length;
    uint8_t magic[8] = {0};
    if (!length(io, &file_length, e)) return NULL;
    if (file_length < 8) { fail(e, 10); return NULL; }
    if (!read(io, magic, 8, e)) return NULL;
    uint32_t format = 0;
    if (!memcmp(magic, "QFI\xfb", 4)) format = 2;
    else if (!memcmp(magic, "conectix", 8)) format = 1;
    else if (file_length >= 512) {
        uint8_t footer[8] = {0};
        if (!seek(io, 1, (uint64_t)-512, e) || !read(io, footer, 8, e) || !seek(io, 0, 0, e)) return NULL;
        if (!memcmp(footer, "conectix", 8)) format = 1;
    }
    ghostos_vm_disk_image *d = calloc(1, sizeof(*d));
    if (!d) { fail(e, 4); return NULL; }
    d->format = format; d->writable = writable; d->checked = checked; d->size = file_length;
    bool ok = format == 0 ? capacity(file_length, 0, e) : format == 1 ? open_vhd(d, io, e) : open_qcow(d, io, e);
    if (!ok) { ghostos_vm_disk_image_free(d); return NULL; }
    return d;
}
void ghostos_vm_disk_image_free(ghostos_vm_disk_image *d) { if (d) { free(d->l1); free(d->l2); free(d); } }
uint32_t ghostos_vm_disk_image_format(const ghostos_vm_disk_image *d) { return d->format; }
uint64_t ghostos_vm_disk_image_size(const ghostos_vm_disk_image *d) { return d->size; }
uint64_t ghostos_vm_disk_image_sectors(const ghostos_vm_disk_image *d) { return d->size / 512; }
bool ghostos_vm_disk_image_writable(const ghostos_vm_disk_image *d) { return d->writable; }

static bool load_l2(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, uint64_t offset, ghostos_vm_disk_error *e) {
    if (d->l2_offset == offset && d->l2) return true;
    uint64_t *entries = malloc((size_t)d->l2_entries * sizeof(*entries));
    if (!entries) return fail(e, 4);
    bool ok = seek(io, 0, offset, e);
    for (uint32_t i = 0; ok && i < d->l2_entries; ++i) {
        uint8_t bytes[8] = {0}; ok = read(io, bytes, 8, e); entries[i] = be64(bytes);
    }
    uint64_t file_length;
    if (ok) ok = length(io, &file_length, e);
    for (uint32_t i = 0; ok && i < d->l2_entries; ++i) ok = data_entry(entries[i], d->cluster_size, file_length, e);
    if (!ok) { free(entries); return false; }
    free(d->l2); d->l2 = entries; d->l2_offset = offset; return true;
}
static bool physical(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, uint64_t cluster,
    uint64_t *result, ghostos_vm_disk_error *e) {
    size_t index = (size_t)(cluster >> d->l2_bits);
    if (index >= d->l1_size) return fail(e, 3);
    uint64_t entry = d->l1[index]; *result = 0;
    if (!entry) return true;
    uint64_t offset = entry & OFFSET_MASK;
    if (!offset) return fail(e, 30);
    if (!load_l2(d, io, offset, e)) return false;
    entry = d->l2[cluster & (d->l2_entries - 1)]; offset = entry & OFFSET_MASK;
    if (!offset) return true;
    if (entry & (UINT64_C(1) << 62)) return fail(e, 32);
    *result = offset; return true;
}
static bool new_cluster(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, uint64_t *offset, ghostos_vm_disk_error *e) {
    uint64_t end;
    if (!seek(io, 1, 0, e) || !io->seek(io->context, 2, 0, &end)) return fail(e, 1);
    if (d->checked && end > UINT64_MAX - (d->cluster_size - 1)) return fail(e, 5);
    uint64_t aligned = (end + d->cluster_size - 1) & ~(d->cluster_size - 1);
    if (d->checked && aligned > UINT64_MAX - d->cluster_size) return fail(e, 5);
    if (!io->resize(io->context, aligned + d->cluster_size)) return fail(e, 1);
    *offset = aligned; return true;
}
static bool ensure_l2(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, size_t index,
    uint64_t *offset, ghostos_vm_disk_error *e) {
    uint64_t existing = d->l1[index] & OFFSET_MASK;
    if (existing) { *offset = existing; return load_l2(d, io, existing, e); }
    if (!new_cluster(d, io, offset, e) || !seek(io, 0, *offset, e)) return false;
    uint8_t *zeros = calloc(1, (size_t)d->cluster_size);
    if (!zeros) return fail(e, 4);
    bool ok = write(io, zeros, (size_t)d->cluster_size, e); free(zeros);
    if (!ok) return false;
    uint64_t entry = *offset | 1; uint8_t bytes[8]; put64(bytes, entry);
    if (!seek(io, 0, d->l1_offset + index * 8, e) || !write(io, bytes, 8, e)) return false;
    d->l1[index] = entry;
    d->l2_offset = *offset;
    uint64_t *cache = calloc(d->l2_entries, sizeof(*cache));
    if (!cache) return fail(e, 4);
    free(d->l2); d->l2 = cache; return true;
}
static bool allocate_cluster(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io, uint64_t cluster,
    uint64_t *offset, ghostos_vm_disk_error *e) {
    size_t l1_index = (size_t)(cluster >> d->l2_bits), l2_index = (size_t)(cluster & (d->l2_entries - 1));
    if (l1_index >= d->l1_size) return fail(e, 3);
    /* Preserve the existing fast path: it uses the current L2 cache before
     * checking which L1 entry owns that cache. */
    if (d->l2_offset && d->l2 && (d->l2[l2_index] & OFFSET_MASK)) {
        *offset = d->l2[l2_index] & OFFSET_MASK; return true;
    }
    uint64_t l2;
    if (!ensure_l2(d, io, l1_index, &l2, e) || !new_cluster(d, io, offset, e)) return false;
    uint64_t entry = *offset | 1; uint8_t bytes[8]; put64(bytes, entry);
    if (!seek(io, 0, l2 + l2_index * 8, e) || !write(io, bytes, 8, e)) return false;
    if (d->l2) d->l2[l2_index] = entry;
    return true;
}
static bool bounds(const ghostos_vm_disk_image *d, uint64_t lba, ghostos_vm_disk_error *e) {
    if (lba > (UINT64_MAX - 512) / 512 || lba * 512 + 512 > d->size) return fail(e, 3);
    return true;
}
bool ghostos_vm_disk_image_read(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io,
    uint64_t lba, uint8_t out[512], ghostos_vm_disk_error *e) {
    memset(e, 0, sizeof(*e));
    if (!bounds(d, lba, e)) return false;
    uint64_t address = lba * 512;
    if (d->format != 2) return seek(io, 0, address, e) && read(io, out, 512, e);
    uint64_t offset;
    if (!physical(d, io, address >> d->cluster_bits, &offset, e)) return false;
    if (!offset) { memset(out, 0, 512); return true; }
    return seek(io, 0, offset + (address & (d->cluster_size - 1)), e) && read(io, out, 512, e);
}
bool ghostos_vm_disk_image_write(ghostos_vm_disk_image *d, const ghostos_vm_disk_file_io *io,
    uint64_t lba, const uint8_t bytes[512], ghostos_vm_disk_error *e) {
    memset(e, 0, sizeof(*e));
    if (!d->writable) return fail(e, 2);
    if (!bounds(d, lba, e)) return false;
    uint64_t address = lba * 512;
    if (d->format == 2) {
        uint64_t offset;
        if (!allocate_cluster(d, io, address >> d->cluster_bits, &offset, e)) return false;
        address = offset + (address & (d->cluster_size - 1));
    }
    if (!seek(io, 0, address, e) || !write(io, bytes, 512, e)) return false;
    return io->flush(io->context) || fail(e, 1);
}
bool ghostos_vm_disk_image_flush(const ghostos_vm_disk_file_io *io, bool durable, ghostos_vm_disk_error *e) {
    memset(e, 0, sizeof(*e));
    if (!io->flush(io->context)) return fail(e, 1);
    return !durable || io->sync(io->context) || fail(e, 1);
}
const char *ghostos_vm_disk_error_message(uint32_t code) {
    switch (code) {
        case 10: return "disk image header is truncated";
        case 11: return "VHD image is truncated";
        case 13: return "VHD original/current size mismatch";
        case 16: return "VHD footer checksum mismatch";
        case 17: return "QCOW2 header is truncated";
        case 19: return "QCOW2 backing files are not supported";
        case 20: return "QCOW2 encryption is not supported";
        case 23: return "invalid QCOW2 L1 size";
        case 24: return "invalid QCOW2 header length";
        case 25: return "QCOW2 L1 table is not cluster-aligned";
        case 27: return "QCOW2 L1 table range overflows";
        case 28: return "QCOW2 L1 table is outside the image";
        case 29: return "QCOW2 L1 table does not cover the virtual disk";
        case 30: return "QCOW2 L1 entry has no table offset";
        case 31: return "QCOW2 L2 table is outside the image";
        case 32: return "QCOW2 compressed clusters are not supported";
        case 33: return "QCOW2 data cluster is outside the image";
        default: return "";
    }
}
