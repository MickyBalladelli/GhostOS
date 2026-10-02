#ifndef GHOSTOS_VM_DISK_IMAGE_H
#define GHOSTOS_VM_DISK_IMAGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct ghostos_vm_disk_image ghostos_vm_disk_image;
typedef struct {
    bool (*length)(void *, uint64_t *);
    /* Origin: 0 start, 1 end, 2 current. End/current offsets are signed
     * two's-complement values; position receives the host seek result. */
    bool (*seek)(void *, uint32_t origin, uint64_t offset, uint64_t *position);
    bool (*read)(void *, uint8_t *, size_t);
    bool (*write)(void *, const uint8_t *, size_t);
    bool (*resize)(void *, uint64_t);
    bool (*flush)(void *);
    bool (*sync)(void *);
    void *context;
} ghostos_vm_disk_file_io;
typedef struct { uint32_t code, format; uint64_t value, other; } ghostos_vm_disk_error;
/* Formats: 0 RAW, 1 fixed VHD, 2 QCOW2. Error codes: 0 success, 1 host I/O,
 * 2 read-only, 3 range, 4 allocation, 5 legacy checked-arithmetic panic;
 * 10..33 validation errors. File callbacks are never retained. */
ghostos_vm_disk_image *ghostos_vm_disk_image_open(const ghostos_vm_disk_file_io *io,
    bool writable, bool checked, ghostos_vm_disk_error *error);
void ghostos_vm_disk_image_free(ghostos_vm_disk_image *image);
uint32_t ghostos_vm_disk_image_format(const ghostos_vm_disk_image *image);
uint64_t ghostos_vm_disk_image_size(const ghostos_vm_disk_image *image);
uint64_t ghostos_vm_disk_image_sectors(const ghostos_vm_disk_image *image);
bool ghostos_vm_disk_image_writable(const ghostos_vm_disk_image *image);
bool ghostos_vm_disk_image_read(ghostos_vm_disk_image *image, const ghostos_vm_disk_file_io *io,
    uint64_t lba, uint8_t output[512], ghostos_vm_disk_error *error);
bool ghostos_vm_disk_image_write(ghostos_vm_disk_image *image, const ghostos_vm_disk_file_io *io,
    uint64_t lba, const uint8_t bytes[512], ghostos_vm_disk_error *error);
bool ghostos_vm_disk_image_flush(const ghostos_vm_disk_file_io *io, bool durable, ghostos_vm_disk_error *error);
/* Safe fixed-VHD repair eligibility retains the historical 32-bit size fields
 * at offsets 36 and 40. Invalid sector capacity is an error, not ineligibility.
 * Repair writes only checksum bytes and syncs; the host syncs the directory. */
bool ghostos_vm_disk_vhd_repairable(const ghostos_vm_disk_file_io *io,
    bool *repairable, ghostos_vm_disk_error *error);
bool ghostos_vm_disk_vhd_repair_checksum(const ghostos_vm_disk_file_io *io,
    ghostos_vm_disk_error *error);
const char *ghostos_vm_disk_format_name(uint32_t format);
/* UINT32_MAX means unrecognized, names are exact lowercase byte strings. */
uint32_t ghostos_vm_disk_parse_format(const uint8_t *bytes, size_t length);
const char *ghostos_vm_disk_error_message(uint32_t code);

#endif
