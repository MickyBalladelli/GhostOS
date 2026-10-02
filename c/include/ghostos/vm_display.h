#ifndef GHOSTOS_VM_DISPLAY_H
#define GHOSTOS_VM_DISPLAY_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "ghostos/vm_bios.h"

#define GHOSTOS_VM_VGA_TEXT_BASE UINT64_C(0xb8000)
#define GHOSTOS_VM_VGA_TEXT_SIZE 32768
#define GHOSTOS_VM_VGA_COLS 80
#define GHOSTOS_VM_VGA_ROWS 25
#define GHOSTOS_VM_VESA_LFB_BASE UINT64_C(0xf0000000)
#define GHOSTOS_VM_VESA_FB_SIZE 16777216
#define GHOSTOS_VM_VBE_MODE_COUNT 12

enum ghostos_vm_display_result {
    GHOSTOS_VM_DISPLAY_OK, GHOSTOS_VM_DISPLAY_SIZE, GHOSTOS_VM_DISPLAY_ADDRESS,
    GHOSTOS_VM_DISPLAY_ALLOCATION, GHOSTOS_VM_DISPLAY_LEGACY_BOUNDS,
    GHOSTOS_VM_DISPLAY_LEGACY_MODE_INFO
};
typedef struct ghostos_vm_display ghostos_vm_display;
typedef struct {
    uint32_t width, height, pixel_width, pixel_height, pitch;
    uint16_t vesa_mode;
    uint8_t mode, current_mode, bpp, cursor_x, cursor_y;
    bool cursor_visible, current_lfb, dirty;
} ghostos_vm_display_info;

ghostos_vm_display *ghostos_vm_display_new(void);
void ghostos_vm_display_free(ghostos_vm_display *);
void ghostos_vm_display_reset(ghostos_vm_display *);
void ghostos_vm_display_info_get(const ghostos_vm_display *, ghostos_vm_display_info *);
bool ghostos_vm_display_cell(const ghostos_vm_display *, size_t, size_t, uint8_t *, uint8_t *);
const uint32_t *ghostos_vm_display_pixels(const ghostos_vm_display *, size_t *);
uint32_t ghostos_vm_display_read(const ghostos_vm_display *, bool, uint64_t, uint8_t, uint64_t *);
uint32_t ghostos_vm_display_write(ghostos_vm_display *, bool, uint64_t, uint8_t, uint64_t);
uint8_t ghostos_vm_display_port_read(ghostos_vm_display *, uint16_t);
void ghostos_vm_display_port_write(ghostos_vm_display *, uint16_t, uint8_t);
uint32_t ghostos_vm_display_render(ghostos_vm_display *);
uint32_t ghostos_vm_display_ppm(ghostos_vm_display *, uint8_t **, size_t *);
void ghostos_vm_display_bytes_free(uint8_t *);
/* UTF-8 encoding of legacy byte-as-character snapshots, including U+0080..00FF. */
bool ghostos_vm_display_text_snapshot(const ghostos_vm_display *, uint8_t *, size_t, size_t *);
/* C reports legacy Rust bounds/width panics rather than writing outside arrays. */
uint32_t ghostos_vm_display_int10(ghostos_vm_display *, ghostos_vm_bios_registers *,
    const ghostos_vm_bios_io *);
#endif
