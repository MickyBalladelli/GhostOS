#include "ghostos/vm_display.h"
#include <stdlib.h>
#include <stdio.h>
#include <string.h>

_Static_assert(sizeof(ghostos_vm_display_info) == 32, "display info ABI");

struct ghostos_vm_display {
    uint8_t text[GHOSTOS_VM_VGA_TEXT_SIZE];
    uint8_t *framebuffer;
    uint32_t *pixels;
    size_t pixel_count;
    ghostos_vm_display_info info;
    uint8_t dac[768];
    uint8_t dac_write_index, dac_read_index, dac_write_phase, dac_read_phase;
    uint8_t crtc_index, atc_index, seq_index, gc_index;
    uint8_t crtc_regs[32], atc_regs[32], seq_regs[16], gc_regs[16];
};

static const uint8_t default_dac[16][3] = {
    {0,0,0}, {0,0,42}, {0,42,0}, {0,42,42},
    {42,0,0}, {42,0,42}, {42,21,0}, {42,42,42},
    {21,21,21}, {21,21,63}, {21,63,21}, {21,63,63},
    {63,21,21}, {63,21,63}, {63,63,21}, {63,63,63}
};
static const uint16_t vbe_modes[GHOSTOS_VM_VBE_MODE_COUNT] = {
    0x101,0x103,0x105,0x110,0x111,0x112,0x113,0x114,0x115,0x116,0x117,0x118
};

void ghostos_vm_display_reset(ghostos_vm_display *display) {
    if (!display) return;
    uint8_t *framebuffer = display->framebuffer;
    free(display->pixels);
    memset(display, 0, sizeof(*display));
    display->framebuffer = framebuffer;
    memset(framebuffer, 0, GHOSTOS_VM_VESA_FB_SIZE);
    display->info = (ghostos_vm_display_info){
        .width = 640, .height = 480, .pixel_width = 640, .pixel_height = 200,
        .pitch = 2560, .vesa_mode = 0x112, .current_mode = 3, .bpp = 32,
        .cursor_visible = true, .current_lfb = true, .dirty = true
    };
    memcpy(display->dac, default_dac, sizeof(default_dac));
    for (size_t i = 16; i < 256; ++i) {
        uint8_t value = (uint8_t)((i - 16) * 63 / 239);
        for (size_t c = 0; c < 3; ++c) display->dac[i * 3 + c] = value;
    }
}

ghostos_vm_display *ghostos_vm_display_new(void) {
    ghostos_vm_display *display = calloc(1, sizeof(*display));
    if (!display) return NULL;
    display->framebuffer = malloc(GHOSTOS_VM_VESA_FB_SIZE);
    if (!display->framebuffer) { free(display); return NULL; }
    ghostos_vm_display_reset(display);
    return display;
}
void ghostos_vm_display_free(ghostos_vm_display *display) {
    if (!display) return;
    free(display->framebuffer);
    free(display->pixels);
    free(display);
}
void ghostos_vm_display_info_get(const ghostos_vm_display *display, ghostos_vm_display_info *info) {
    if (display && info) *info = display->info;
}
bool ghostos_vm_display_cell(const ghostos_vm_display *display, size_t row, size_t col,
    uint8_t *character, uint8_t *attribute) {
    if (!display || row >= GHOSTOS_VM_VGA_ROWS || col >= GHOSTOS_VM_VGA_COLS) return false;
    size_t offset = (row * GHOSTOS_VM_VGA_COLS + col) * 2;
    if (character) *character = display->text[offset];
    if (attribute) *attribute = display->text[offset + 1];
    return true;
}
const uint32_t *ghostos_vm_display_pixels(const ghostos_vm_display *display, size_t *count) {
    if (!display || !count) return NULL;
    *count = display->pixel_count;
    return display->pixels;
}

static uint32_t access_offset(bool framebuffer, uint64_t address, uint8_t size, size_t *offset) {
    if (size != 1 && size != 2 && size != 4 && size != 8) return GHOSTOS_VM_DISPLAY_SIZE;
    uint64_t base = framebuffer ? GHOSTOS_VM_VESA_LFB_BASE : GHOSTOS_VM_VGA_TEXT_BASE;
    size_t length = framebuffer ? GHOSTOS_VM_VESA_FB_SIZE : GHOSTOS_VM_VGA_TEXT_SIZE;
    uint64_t relative = address >= base ? address - base : address & (length - 1);
    if (relative > length - size) return GHOSTOS_VM_DISPLAY_ADDRESS;
    *offset = (size_t)relative;
    return GHOSTOS_VM_DISPLAY_OK;
}
uint32_t ghostos_vm_display_read(const ghostos_vm_display *display, bool framebuffer,
    uint64_t address, uint8_t size, uint64_t *value) {
    if (!display || !value) return GHOSTOS_VM_DISPLAY_ADDRESS;
    size_t offset;
    uint32_t result = access_offset(framebuffer, address, size, &offset);
    if (result) return result;
    const uint8_t *bytes = framebuffer ? display->framebuffer : display->text;
    *value = 0;
    for (size_t i = 0; i < size; ++i) *value |= (uint64_t)bytes[offset + i] << (i * 8);
    return GHOSTOS_VM_DISPLAY_OK;
}
uint32_t ghostos_vm_display_write(ghostos_vm_display *display, bool framebuffer,
    uint64_t address, uint8_t size, uint64_t value) {
    if (!display) return GHOSTOS_VM_DISPLAY_ADDRESS;
    size_t offset;
    uint32_t result = access_offset(framebuffer, address, size, &offset);
    if (result) return result;
    uint8_t *bytes = framebuffer ? display->framebuffer : display->text;
    for (size_t i = 0; i < size; ++i) bytes[offset + i] = (uint8_t)(value >> (i * 8));
    display->info.dirty = true;
    return GHOSTOS_VM_DISPLAY_OK;
}

uint8_t ghostos_vm_display_port_read(ghostos_vm_display *display, uint16_t port) {
    if (!display) return 0xff;
    switch (port) {
        case 0x3c0: return display->atc_regs[display->atc_index & 31];
        case 0x3c1: return display->atc_index;
        case 0x3c4: return display->seq_regs[display->seq_index & 15];
        case 0x3c5: return display->seq_index;
        case 0x3c7: return 3;
        case 0x3c8: return display->dac_write_index;
        case 0x3c9: {
            uint8_t value = display->dac[display->dac_read_index * 3 + display->dac_read_phase];
            if (++display->dac_read_phase >= 3) {
                display->dac_read_phase = 0;
                ++display->dac_read_index;
            }
            return value;
        }
        case 0x3ce: return display->gc_regs[display->gc_index & 15];
        case 0x3cf: return display->gc_index;
        case 0x3d4: return display->crtc_index;
        case 0x3d5: {
            size_t linear = display->info.cursor_y * GHOSTOS_VM_VGA_COLS + display->info.cursor_x;
            if (display->crtc_index == 14) return (uint8_t)(linear >> 8);
            if (display->crtc_index == 15) return (uint8_t)linear;
            return display->crtc_regs[display->crtc_index & 31];
        }
        case 0x3da: return 0x10;
        default: return 0xff;
    }
}

void ghostos_vm_display_port_write(ghostos_vm_display *display, uint16_t port, uint8_t value) {
    if (!display) return;
    switch (port) {
        case 0x3c0: display->atc_index = value & 31; break;
        case 0x3c1: display->atc_regs[display->atc_index & 31] = value; break;
        case 0x3c4: display->seq_index = value & 7; break;
        case 0x3c5: display->seq_regs[display->seq_index & 15] = value; break;
        case 0x3c7: display->dac_read_index = value; display->dac_read_phase = 0; break;
        case 0x3c8: display->dac_write_index = value; display->dac_write_phase = 0; break;
        case 0x3c9:
            display->dac[display->dac_write_index * 3 + display->dac_write_phase] = value & 63;
            if (++display->dac_write_phase >= 3) {
                display->dac_write_phase = 0;
                ++display->dac_write_index;
            }
            display->info.dirty = true;
            break;
        case 0x3ce: display->gc_index = value & 15; break;
        case 0x3cf: display->gc_regs[display->gc_index & 15] = value; break;
        case 0x3d4: display->crtc_index = value & 31; break;
        case 0x3d5: {
            display->crtc_regs[display->crtc_index & 31] = value;
            if (display->crtc_index == 10) display->info.cursor_visible = !(value & 0x20);
            if (display->crtc_index == 14 || display->crtc_index == 15) {
                size_t linear = (size_t)display->crtc_regs[14] << 8 | display->crtc_regs[15];
                display->info.cursor_y = (uint8_t)((linear / GHOSTOS_VM_VGA_COLS) % GHOSTOS_VM_VGA_ROWS);
                display->info.cursor_x = (uint8_t)(linear % GHOSTOS_VM_VGA_COLS);
            }
            display->info.dirty = true;
            break;
        }
        default: break;
    }
}

static uint32_t dac_rgb(const ghostos_vm_display *display, size_t index) {
    const uint8_t *rgb = display->dac + (index & 255) * 3;
    return (((uint32_t)rgb[0] * 255 + 31) / 63) << 16 |
        (((uint32_t)rgb[1] * 255 + 31) / 63) << 8 |
        ((uint32_t)rgb[2] * 255 + 31) / 63;
}
static uint8_t fb_byte(const ghostos_vm_display *display, size_t offset) {
    return offset < GHOSTOS_VM_VESA_FB_SIZE ? display->framebuffer[offset] : 0;
}

uint32_t ghostos_vm_display_render(ghostos_vm_display *display) {
    if (!display) return GHOSTOS_VM_DISPLAY_ADDRESS;
    size_t width = display->info.pixel_width, height = display->info.pixel_height;
    if (height && width > SIZE_MAX / height) return GHOSTOS_VM_DISPLAY_ALLOCATION;
    size_t count = width * height;
    if (count > SIZE_MAX / sizeof(*display->pixels)) return GHOSTOS_VM_DISPLAY_ALLOCATION;
    if (count != display->pixel_count && count) {
        uint32_t *pixels = realloc(display->pixels, count * sizeof(*pixels));
        if (!pixels) return GHOSTOS_VM_DISPLAY_ALLOCATION;
        display->pixels = pixels;
    }
    display->pixel_count = count;
    if (!display->info.mode) {
        for (size_t row = 0; row < GHOSTOS_VM_VGA_ROWS; ++row) {
            for (size_t col = 0; col < GHOSTOS_VM_VGA_COLS; ++col) {
                size_t offset = (row * GHOSTOS_VM_VGA_COLS + col) * 2;
                uint8_t ch = display->text[offset], attr = display->text[offset + 1];
                uint32_t foreground = dac_rgb(display, attr & 15);
                uint32_t background = dac_rgb(display, (attr >> 4) & 7);
                bool cursor = display->info.cursor_visible && row == display->info.cursor_y && col == display->info.cursor_x;
                for (size_t y = 0; y < 8; ++y) {
                    uint8_t bits = 0;
                    if (ch >= 0x21 && ch < 0x7f && y < 7) {
                        for (size_t x = 0; x < 5; ++x)
                            if (((y ^ x) & 1) == (ch & 1)) bits |= (uint8_t)(0x80 >> (1 + x));
                    }
                    if (cursor && y >= 4) bits = 0xff;
                    for (size_t x = 0; x < 8; ++x)
                        display->pixels[(row * 8 + y) * width + col * 8 + x] =
                            bits & (0x80 >> x) ? foreground : background;
                }
            }
        }
    } else {
        size_t bytes_per_pixel = (display->info.bpp + 7) / 8;
        for (size_t y = 0; y < height; ++y) {
            for (size_t x = 0; x < width; ++x) {
                size_t offset = y * display->info.pitch + x * bytes_per_pixel;
                uint32_t rgb;
                if (display->info.bpp == 8) rgb = dac_rgb(display, fb_byte(display, offset));
                else if (display->info.bpp == 15 || display->info.bpp == 16) {
                    uint16_t value = (uint16_t)((uint16_t)fb_byte(display, offset) |
                        (uint16_t)fb_byte(display, offset + 1) << 8);
                    bool rgb565 = display->info.bpp == 16;
                    uint32_t r = (value >> (rgb565 ? 11 : 10)) & 31;
                    uint32_t g = (value >> 5) & (rgb565 ? 63 : 31);
                    uint32_t b = value & 31;
                    rgb = (r * 255 / 31) << 16 | (g * 255 / (rgb565 ? 63 : 31)) << 8 | b * 255 / 31;
                } else {
                    rgb = (uint32_t)fb_byte(display, offset + 2) << 16 |
                        (uint32_t)fb_byte(display, offset + 1) << 8 | fb_byte(display, offset);
                }
                display->pixels[y * width + x] = rgb;
            }
        }
    }
    display->info.dirty = false;
    return GHOSTOS_VM_DISPLAY_OK;
}

bool ghostos_vm_display_text_snapshot(const ghostos_vm_display *display,
    uint8_t *output, size_t capacity, size_t *length) {
    if (!display || !length) return false;
    size_t required = GHOSTOS_VM_VGA_ROWS;
    for (size_t i = 0; i < GHOSTOS_VM_VGA_ROWS * GHOSTOS_VM_VGA_COLS; ++i)
        required += display->text[i * 2] >= 128 ? 2 : 1;
    *length = required;
    if (!output) return capacity == 0;
    if (capacity < required) return false;
    size_t offset = 0;
    for (size_t row = 0; row < GHOSTOS_VM_VGA_ROWS; ++row) {
        for (size_t col = 0; col < GHOSTOS_VM_VGA_COLS; ++col) {
            uint8_t ch = display->text[(row * GHOSTOS_VM_VGA_COLS + col) * 2];
            if (ch >= 128) {
                output[offset++] = (uint8_t)(0xc0 | ch >> 6);
                output[offset++] = (uint8_t)(0x80 | (ch & 63));
            } else output[offset++] = ch ? ch : ' ';
        }
        output[offset++] = '\n';
    }
    return true;
}

uint32_t ghostos_vm_display_ppm(ghostos_vm_display *display, uint8_t **output, size_t *length) {
    if (!output || !length) return GHOSTOS_VM_DISPLAY_ADDRESS;
    *output = NULL;
    *length = 0;
    uint32_t result = ghostos_vm_display_render(display);
    if (result) return result;
    char header[64];
    int header_length = snprintf(header, sizeof(header), "P6\n%u %u\n255\n",
        display->info.pixel_width, display->info.pixel_height);
    if (header_length < 0 || (size_t)header_length >= sizeof(header) ||
        display->pixel_count > (SIZE_MAX - (size_t)header_length) / 3)
        return GHOSTOS_VM_DISPLAY_ALLOCATION;
    size_t required = (size_t)header_length + display->pixel_count * 3;
    uint8_t *bytes = malloc(required);
    if (!bytes) return GHOSTOS_VM_DISPLAY_ALLOCATION;
    memcpy(bytes, header, (size_t)header_length);
    size_t offset = (size_t)header_length;
    for (size_t i = 0; i < display->pixel_count; ++i) {
        bytes[offset++] = (uint8_t)(display->pixels[i] >> 16);
        bytes[offset++] = (uint8_t)(display->pixels[i] >> 8);
        bytes[offset++] = (uint8_t)display->pixels[i];
    }
    *output = bytes;
    *length = required;
    return GHOSTOS_VM_DISPLAY_OK;
}

void ghostos_vm_display_bytes_free(uint8_t *bytes) { free(bytes); }

static void set_cursor(ghostos_vm_display *display, uint8_t row, uint8_t col) {
    display->info.cursor_y = row < GHOSTOS_VM_VGA_ROWS ? row : GHOSTOS_VM_VGA_ROWS - 1;
    display->info.cursor_x = col < GHOSTOS_VM_VGA_COLS ? col : GHOSTOS_VM_VGA_COLS - 1;
    uint16_t linear = display->info.cursor_y * GHOSTOS_VM_VGA_COLS + display->info.cursor_x;
    display->crtc_regs[14] = (uint8_t)(linear >> 8);
    display->crtc_regs[15] = (uint8_t)linear;
    display->info.dirty = true;
}
static void write_cell(ghostos_vm_display *display, size_t row, size_t col, uint8_t ch, uint8_t attr) {
    size_t offset = (row * GHOSTOS_VM_VGA_COLS + col) * 2;
    if (offset + 1 < GHOSTOS_VM_VGA_TEXT_SIZE) {
        display->text[offset] = ch;
        display->text[offset + 1] = attr;
        display->info.dirty = true;
    }
}

static uint32_t scroll_rect(ghostos_vm_display *display, uint8_t lines,
    uint8_t top, uint8_t left, uint8_t bottom, uint8_t right, uint8_t attr) {
    size_t rows = bottom < GHOSTOS_VM_VGA_ROWS ? bottom : GHOSTOS_VM_VGA_ROWS - 1;
    size_t cols = right < GHOSTOS_VM_VGA_COLS ? right : GHOSTOS_VM_VGA_COLS - 1;
    if (rows < top) rows = top;
    if (cols < left) cols = left;
    size_t remaining = lines ? lines : rows - top + 1;
    for (size_t row = top; row <= rows; ++row) {
        size_t source = row + remaining;
        if (source <= rows) {
            for (size_t col = left; col <= cols; ++col) {
                size_t src = (source * GHOSTOS_VM_VGA_COLS + col) * 2;
                size_t dst = (row * GHOSTOS_VM_VGA_COLS + col) * 2;
                if (src + 1 >= GHOSTOS_VM_VGA_TEXT_SIZE || dst + 1 >= GHOSTOS_VM_VGA_TEXT_SIZE)
                    return GHOSTOS_VM_DISPLAY_LEGACY_BOUNDS;
                display->text[dst] = display->text[src];
                display->text[dst + 1] = display->text[src + 1];
            }
        } else if (remaining) {
            for (size_t col = left; col <= cols; ++col) {
                size_t dst = (row * GHOSTOS_VM_VGA_COLS + col) * 2;
                if (dst + 1 >= GHOSTOS_VM_VGA_TEXT_SIZE) return GHOSTOS_VM_DISPLAY_LEGACY_BOUNDS;
                display->text[dst] = ' ';
                display->text[dst + 1] = attr;
            }
            --remaining;
        }
    }
    display->info.dirty = true;
    return GHOSTOS_VM_DISPLAY_OK;
}

static void enter_text_mode(ghostos_vm_display *display, uint8_t mode) {
    memset(display->text, 0, GHOSTOS_VM_VGA_ROWS * GHOSTOS_VM_VGA_COLS * 2);
    display->info.mode = 0;
    display->info.current_mode = mode & 0x7f;
    display->info.current_lfb = false;
    display->info.pixel_width = GHOSTOS_VM_VGA_COLS * 8;
    display->info.pixel_height = GHOSTOS_VM_VGA_ROWS * 8;
    set_cursor(display, 0, 0);
}
static void enter_vesa_mode(ghostos_vm_display *display, uint16_t mode,
    uint32_t width, uint32_t height, uint8_t bpp, bool lfb) {
    display->info.mode = 1;
    display->info.vesa_mode = mode;
    display->info.current_lfb = lfb;
    display->info.width = display->info.pixel_width = width;
    display->info.height = display->info.pixel_height = height;
    display->info.bpp = bpp;
    display->info.pitch = width * (bpp / 8);
    display->info.current_mode = 0x13;
    display->info.dirty = true;
}

static void put_char(ghostos_vm_display *display, uint8_t ch, uint8_t attr) {
    switch (ch) {
        case '\r': display->info.cursor_x = 0; break;
        case '\n':
            ++display->info.cursor_y;
            if (display->info.cursor_y >= GHOSTOS_VM_VGA_ROWS) {
                display->info.cursor_y = GHOSTOS_VM_VGA_ROWS - 1;
                (void)scroll_rect(display, 1, 0, 0, 24, 79, attr);
            }
            break;
        case 8: if (display->info.cursor_x) --display->info.cursor_x; break;
        case '\t': {
            uint8_t col = (uint8_t)((display->info.cursor_x / 8 + 1) * 8);
            display->info.cursor_x = col < GHOSTOS_VM_VGA_COLS ? col : GHOSTOS_VM_VGA_COLS - 1;
            break;
        }
        case 7: break;
        default:
            write_cell(display, display->info.cursor_y, display->info.cursor_x, ch, attr);
            if (++display->info.cursor_x >= GHOSTOS_VM_VGA_COLS) {
                display->info.cursor_x = 0;
                if (++display->info.cursor_y >= GHOSTOS_VM_VGA_ROWS) {
                    display->info.cursor_y = GHOSTOS_VM_VGA_ROWS - 1;
                    (void)scroll_rect(display, 1, 0, 0, 24, 79, attr);
                }
            }
            break;
    }
    display->info.dirty = true;
}

static bool vbe_geometry(uint16_t mode, uint32_t *width, uint32_t *height, uint8_t *bpp) {
    static const uint32_t widths[] = {640,800,1024,640,640,640,800,800,800,1024,1024,1024};
    static const uint32_t heights[] = {480,600,768,480,480,480,600,600,600,768,768,768};
    static const uint8_t bits[] = {8,8,8,15,16,32,15,16,32,15,24,32};
    for (size_t i = 0; i < GHOSTOS_VM_VBE_MODE_COUNT; ++i) {
        if (vbe_modes[i] == mode) { *width = widths[i]; *height = heights[i]; *bpp = bits[i]; return true; }
    }
    return false;
}
static void set_ax(ghostos_vm_bios_registers *registers, uint16_t ax) {
    registers->rax = (registers->rax & ~UINT64_C(0xffff)) | ax;
}

uint32_t ghostos_vm_display_int10(ghostos_vm_display *display,
    ghostos_vm_bios_registers *r, const ghostos_vm_bios_io *io) {
    if (!display || !r) return GHOSTOS_VM_DISPLAY_ADDRESS;
    uint8_t ah = (uint8_t)(r->rax >> 8), al = (uint8_t)r->rax;
    switch (ah) {
        case 0:
            if ((al & 0x7f) <= 3 || (al & 0x7f) == 7) enter_text_mode(display, al);
            else if ((al & 0x7f) == 0x13) enter_vesa_mode(display, 0x101, 320, 200, 8, true);
            else if ((al & 0x7f) == 0x11 || (al & 0x7f) == 0x12)
                enter_vesa_mode(display, 0x112, 640, 480, 32, true);
            break;
        case 2: set_cursor(display, (uint8_t)(r->rdx >> 8), (uint8_t)r->rdx); break;
        case 6:
            return scroll_rect(display, al, (uint8_t)(r->rcx >> 8), (uint8_t)r->rcx,
                (uint8_t)(r->rdx >> 8), (uint8_t)r->rdx, (uint8_t)(r->rbx >> 8));
        case 9: case 10:
            for (size_t i = 0; i < (uint16_t)r->rcx; ++i) {
                size_t col = display->info.cursor_x + i;
                if (col >= GHOSTOS_VM_VGA_COLS) break;
                write_cell(display, display->info.cursor_y, col, al, (uint8_t)(r->rbx >> 8));
            }
            display->info.dirty = true;
            break;
        case 14: put_char(display, al, (uint8_t)(r->rbx >> 8)); break;
        case 15: set_ax(r, (uint16_t)(GHOSTOS_VM_VGA_COLS << 8 | display->info.current_mode)); break;
        case 0x4f: {
            uint32_t width, height;
            uint8_t bpp;
            switch (al) {
                case 0: {
                    uint8_t buffer[512] = {0};
                    memcpy(buffer, "VESA", 4);
                    buffer[5] = 3;
                    uint64_t address = (uint64_t)r->es * 16 + (uint16_t)r->rdi;
                    uint32_t pointer = (uint32_t)(address + 256);
                    for (size_t i = 0; i < 4; ++i) buffer[12 + i] = (uint8_t)(pointer >> (i * 8));
                    for (size_t i = 0; i < GHOSTOS_VM_VBE_MODE_COUNT; ++i) {
                        buffer[256 + i * 2] = (uint8_t)vbe_modes[i];
                        buffer[257 + i * 2] = (uint8_t)(vbe_modes[i] >> 8);
                    }
                    buffer[280] = buffer[281] = 0xff;
                    if (io && io->write) (void)io->write(io->context, address, buffer, sizeof(buffer));
                    set_ax(r, 0x004f);
                    break;
                }
                case 1:
                    if (vbe_geometry((uint16_t)r->rcx, &width, &height, &bpp))
                        return GHOSTOS_VM_DISPLAY_LEGACY_MODE_INFO;
                    set_ax(r, 0x014f);
                    break;
                case 2: {
                    uint16_t mode = (uint16_t)r->rbx & 0x7fff;
                    if (vbe_geometry(mode, &width, &height, &bpp)) {
                        enter_vesa_mode(display, mode, width, height, bpp, (r->rbx & 0x4000) != 0);
                        set_ax(r, 0x004f);
                    } else if (mode == 3) { enter_text_mode(display, 3); set_ax(r, 0x004f); }
                    else set_ax(r, 0x014f);
                    break;
                }
                case 3:
                    r->rbx = (r->rbx & ~UINT64_C(0xffff)) | (display->info.mode ? display->info.vesa_mode : 3);
                    set_ax(r, 0x004f);
                    break;
                default: set_ax(r, 0x004f); break;
            }
            break;
        }
        default: break;
    }
    return GHOSTOS_VM_DISPLAY_OK;
}
