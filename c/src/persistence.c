#include "ghostos/persistence.h"

#if defined(__x86_64__)
static void io_out8(uint16_t port, uint8_t value) {
    __asm__ volatile("outb %0, %w1" : : "a"(value), "Nd"(port) : "memory");
}

static void io_out32(uint16_t port, uint32_t value) {
    __asm__ volatile("outl %0, %w1" : : "a"(value), "Nd"(port) : "memory");
}

static uint8_t io_in8(uint16_t port) {
    uint8_t value;
    __asm__ volatile("inb %w1, %0" : "=a"(value) : "Nd"(port) : "memory");
    return value;
}

static uint32_t io_in32(uint16_t port) {
    uint32_t value;
    __asm__ volatile("inl %w1, %0" : "=a"(value) : "Nd"(port) : "memory");
    return value;
}
#else
static void io_out8(uint16_t port, uint8_t value) { (void)port; (void)value; }
static void io_out32(uint16_t port, uint32_t value) { (void)port; (void)value; }
static uint8_t io_in8(uint16_t port) { (void)port; return 0; }
static uint32_t io_in32(uint16_t port) { (void)port; return 0; }
#endif

bool ghostos_persistence_load(uint8_t *bytes, size_t capacity, size_t *length) {
    if (!bytes || !length || capacity > GHOSTOS_PERSISTENCE_MAX_BYTES) return false;
    io_out8(GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_LOAD);
    uint32_t stored_length = io_in32(GHOSTOS_PERSISTENCE_LENGTH_PORT);
    if ((size_t)stored_length > capacity) return false;
    for (size_t index = 0; index < stored_length; ++index) {
        bytes[index] = io_in8(GHOSTOS_PERSISTENCE_DATA_PORT);
    }
    *length = stored_length;
    return true;
}

void ghostos_persistence_save(const uint8_t *bytes, size_t length) {
    if ((!bytes && length) || length > GHOSTOS_PERSISTENCE_MAX_BYTES) return;
    io_out8(GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_SAVE);
    io_out32(GHOSTOS_PERSISTENCE_LENGTH_PORT, (uint32_t)length);
    for (size_t index = 0; index < length; ++index) {
        io_out8(GHOSTOS_PERSISTENCE_DATA_PORT, bytes[index]);
    }
    io_out8(GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_FLUSH);
}
