#include "ghostos/console.h"

#include <stdatomic.h>

static atomic_flag console_lock = ATOMIC_FLAG_INIT;
static atomic_size_t remote_columns;
static atomic_size_t remote_rows;
static ghostos_console_backend active_backend;

static void lock_console(void) {
    while (atomic_flag_test_and_set_explicit(&console_lock, memory_order_acquire)) {
        atomic_signal_fence(memory_order_seq_cst);
    }
}

static void unlock_console(void) {
    atomic_flag_clear_explicit(&console_lock, memory_order_release);
}

void ghostos_console_init(ghostos_framebuffer_info framebuffer, ghostos_console_backend backend) {
    lock_console();
    active_backend = backend;
    atomic_store_explicit(&remote_columns, 0, memory_order_relaxed);
    atomic_store_explicit(&remote_rows, 0, memory_order_relaxed);
    if (active_backend.init != NULL) {
        active_backend.init(active_backend.context, framebuffer);
    }
    unlock_console();
}

void ghostos_console_write_bytes(const uint8_t *bytes, size_t length) {
    if (bytes == NULL) return;
    lock_console();
    for (size_t i = 0; i < length; i++) {
        if (active_backend.write_byte != NULL) {
            active_backend.write_byte(active_backend.context, bytes[i]);
        }
    }
    unlock_console();
}

void ghostos_console_write_byte(uint8_t byte) {
    ghostos_console_write_bytes(&byte, 1);
}

bool ghostos_console_read_byte(uint8_t *byte) {
    if (byte == NULL || active_backend.read_byte == NULL) return false;
    return active_backend.read_byte(active_backend.context, byte);
}

void ghostos_console_clear(void) {
    lock_console();
    if (active_backend.clear != NULL) {
        active_backend.clear(active_backend.context);
    }
    unlock_console();
}

void ghostos_console_terminal_size(size_t *columns, size_t *rows) {
    size_t remote_cols = atomic_load_explicit(&remote_columns, memory_order_acquire);
    size_t remote_rws = atomic_load_explicit(&remote_rows, memory_order_acquire);
    size_t cols = 80;
    size_t rws = 25;
    if (remote_cols != 0 && remote_rws != 0) {
        cols = remote_cols;
        rws = remote_rws;
    } else {
        lock_console();
        if (active_backend.terminal_size != NULL) {
            active_backend.terminal_size(active_backend.context, &cols, &rws);
        }
        unlock_console();
    }
    if (columns != NULL) *columns = cols;
    if (rows != NULL) *rows = rws;
}

void ghostos_console_set_remote_terminal_size(size_t columns, size_t rows) {
    if (columns == 0 || rows == 0) return;
    atomic_store_explicit(&remote_columns, columns, memory_order_release);
    atomic_store_explicit(&remote_rows, rows, memory_order_release);
}
