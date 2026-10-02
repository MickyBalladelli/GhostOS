#define _DEFAULT_SOURCE 1
#define _DARWIN_C_SOURCE 1
#define _POSIX_C_SOURCE 200809L
#include "ghostos/vm_terminal_platform.h"
#include <stdlib.h>

#if defined(__unix__) || defined(__APPLE__)
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdatomic.h>
#include <string.h>
#include <sys/ioctl.h>
#include <termios.h>
#include <unistd.h>

_Static_assert(ATOMIC_BOOL_LOCK_FREE == 2 && ATOMIC_INT_LOCK_FREE == 2,
    "terminal signal atomics must be lock-free");

static const int terminal_signals[] = {
    SIGINT, SIGTERM, SIGHUP, SIGQUIT, SIGABRT, SIGILL,
    SIGTRAP, SIGBUS, SIGFPE, SIGSEGV, SIGPIPE, SIGSYS
};
#define SIGNAL_COUNT (sizeof(terminal_signals) / sizeof(terminal_signals[0]))
static atomic_bool signal_active = ATOMIC_VAR_INIT(false);
static atomic_bool signal_ready = ATOMIC_VAR_INIT(false);
static atomic_int signal_fd = ATOMIC_VAR_INIT(-1);
static struct termios signal_settings;

struct ghostos_vm_terminal_mode {
    int fd;
    struct termios saved;
    struct sigaction previous[SIGNAL_COUNT];
    size_t installed;
    bool armed;
};

static int open_tty(void) {
    int flags = O_RDWR;
#ifdef O_CLOEXEC
    flags |= O_CLOEXEC;
#endif
    int fd = open("/dev/tty", flags);
#ifndef O_CLOEXEC
    if (fd >= 0) {
        int result = fcntl(fd, F_SETFD, FD_CLOEXEC);
        if (result < 0) { int error = errno; close(fd); errno = error; return -1; }
    }
#endif
    return fd;
}

static void terminal_signal_handler(int signal_number) {
    if (atomic_load_explicit(&signal_ready, memory_order_acquire)) {
        int fd = atomic_load_explicit(&signal_fd, memory_order_acquire);
        if (fd >= 0) (void)tcsetattr(fd, TCSANOW, &signal_settings);
    }
    (void)signal(signal_number, SIG_DFL);
    (void)raise(signal_number);
    _exit(128 + signal_number);
}

static void disarm(ghostos_vm_terminal_mode *mode) {
    if (!mode->armed) return;
    atomic_store_explicit(&signal_ready, false, memory_order_release);
    for (size_t i = 0; i < mode->installed; ++i)
        (void)sigaction(terminal_signals[i], &mode->previous[i], NULL);
    atomic_store_explicit(&signal_fd, -1, memory_order_release);
    atomic_store_explicit(&signal_active, false, memory_order_release);
    mode->installed = 0;
    mode->armed = false;
}

uint32_t ghostos_vm_terminal_mode_enter(ghostos_vm_terminal_mode **output, int32_t *error) {
    if (!output || !error) return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    *output = NULL;
    *error = 0;
    ghostos_vm_terminal_mode *mode = calloc(1, sizeof(*mode));
    if (!mode) return GHOSTOS_VM_TERMINAL_PLATFORM_ALLOCATION;
    mode->fd = open_tty();
    if (mode->fd < 0) { *error = errno; free(mode); return GHOSTOS_VM_TERMINAL_PLATFORM_IO; }
    if (tcgetattr(mode->fd, &mode->saved)) {
        *error = errno;
        close(mode->fd);
        free(mode);
        return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    }
    if (atomic_exchange_explicit(&signal_active, true, memory_order_acq_rel)) {
        close(mode->fd);
        free(mode);
        return GHOSTOS_VM_TERMINAL_PLATFORM_ACTIVE;
    }
    mode->armed = true;
    atomic_store_explicit(&signal_fd, mode->fd, memory_order_release);
    signal_settings = mode->saved;
    for (size_t i = 0; i < SIGNAL_COUNT; ++i) {
        struct sigaction action;
        memset(&action, 0, sizeof(action));
        action.sa_handler = terminal_signal_handler;
        sigemptyset(&action.sa_mask);
        if (sigaction(terminal_signals[i], &action, &mode->previous[i])) {
            *error = errno;
            disarm(mode);
            close(mode->fd);
            free(mode);
            return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
        }
        ++mode->installed;
    }
    atomic_store_explicit(&signal_ready, true, memory_order_release);
    struct termios raw = mode->saved;
    cfmakeraw(&raw);
    raw.c_cc[VMIN] = 1;
    raw.c_cc[VTIME] = 0;
    if (tcsetattr(mode->fd, TCSANOW, &raw)) {
        *error = errno;
        disarm(mode);
        close(mode->fd);
        free(mode);
        return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    }
    *output = mode;
    return GHOSTOS_VM_TERMINAL_PLATFORM_OK;
}

uint32_t ghostos_vm_terminal_mode_restore(ghostos_vm_terminal_mode *mode, int32_t *error) {
    if (!mode || !error) return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    int result = tcsetattr(mode->fd, TCSANOW, &mode->saved);
    *error = result ? errno : 0;
    disarm(mode);
    return result ? GHOSTOS_VM_TERMINAL_PLATFORM_IO : GHOSTOS_VM_TERMINAL_PLATFORM_OK;
}
void ghostos_vm_terminal_mode_free(ghostos_vm_terminal_mode *mode) {
    if (!mode) return;
    int32_t error;
    (void)ghostos_vm_terminal_mode_restore(mode, &error);
    close(mode->fd);
    free(mode);
}
bool ghostos_vm_terminal_size(uint16_t *rows, uint16_t *columns) {
    if (!rows || !columns) return false;
    int fd = open_tty();
    if (fd < 0) return false;
    struct winsize window = {0};
    int result = ioctl(fd, TIOCGWINSZ, &window);
    close(fd);
    if (result || !window.ws_row || !window.ws_col) return false;
    *rows = window.ws_row;
    *columns = window.ws_col;
    return true;
}

#elif defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

struct ghostos_vm_terminal_mode {
    HANDLE input, output;
    DWORD input_saved, output_saved;
};
uint32_t ghostos_vm_terminal_mode_enter(ghostos_vm_terminal_mode **output, int32_t *error) {
    if (!output || !error) return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    *output = NULL;
    *error = 0;
    ghostos_vm_terminal_mode *mode = calloc(1, sizeof(*mode));
    if (!mode) return GHOSTOS_VM_TERMINAL_PLATFORM_ALLOCATION;
    mode->input = GetStdHandle(STD_INPUT_HANDLE);
    mode->output = GetStdHandle(STD_OUTPUT_HANDLE);
    if (!GetConsoleMode(mode->input, &mode->input_saved) ||
        !GetConsoleMode(mode->output, &mode->output_saved)) {
        *error = (int32_t)GetLastError();
        free(mode);
        return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    }
    DWORD raw_input = mode->input_saved & ~(ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT |
        ENABLE_ECHO_INPUT | ENABLE_QUICK_EDIT_MODE);
    raw_input |= ENABLE_EXTENDED_FLAGS | ENABLE_VIRTUAL_TERMINAL_INPUT;
    if (!SetConsoleMode(mode->input, raw_input)) {
        *error = (int32_t)GetLastError();
        free(mode);
        return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    }
    if (!SetConsoleMode(mode->output, mode->output_saved | ENABLE_VIRTUAL_TERMINAL_PROCESSING)) {
        *error = (int32_t)GetLastError();
        (void)SetConsoleMode(mode->input, mode->input_saved);
        free(mode);
        return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    }
    *output = mode;
    return GHOSTOS_VM_TERMINAL_PLATFORM_OK;
}
uint32_t ghostos_vm_terminal_mode_restore(ghostos_vm_terminal_mode *mode, int32_t *error) {
    if (!mode || !error) return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    bool output_ok = SetConsoleMode(mode->output, mode->output_saved) != 0;
    int32_t output_error = output_ok ? 0 : (int32_t)GetLastError();
    bool input_ok = SetConsoleMode(mode->input, mode->input_saved) != 0;
    int32_t input_error = input_ok ? 0 : (int32_t)GetLastError();
    *error = output_ok ? input_error : output_error;
    return output_ok && input_ok ? GHOSTOS_VM_TERMINAL_PLATFORM_OK : GHOSTOS_VM_TERMINAL_PLATFORM_IO;
}
void ghostos_vm_terminal_mode_free(ghostos_vm_terminal_mode *mode) {
    if (!mode) return;
    int32_t error;
    (void)ghostos_vm_terminal_mode_restore(mode, &error);
    free(mode);
}
bool ghostos_vm_terminal_size(uint16_t *rows, uint16_t *columns) {
    if (!rows || !columns) return false;
    HANDLE handle = GetStdHandle(STD_OUTPUT_HANDLE);
    DWORD mode;
    CONSOLE_SCREEN_BUFFER_INFO info;
    if (!GetConsoleMode(handle, &mode) || !GetConsoleScreenBufferInfo(handle, &info)) return false;
    int width = (int)info.srWindow.Right - info.srWindow.Left + 1;
    int height = (int)info.srWindow.Bottom - info.srWindow.Top + 1;
    if (width < 0 || height < 0 || width > UINT16_MAX || height > UINT16_MAX) return false;
    *rows = (uint16_t)height;
    *columns = (uint16_t)width;
    return true;
}

#else
struct ghostos_vm_terminal_mode { bool active; };
uint32_t ghostos_vm_terminal_mode_enter(ghostos_vm_terminal_mode **output, int32_t *error) {
    if (!output || !error) return GHOSTOS_VM_TERMINAL_PLATFORM_IO;
    *output = calloc(1, sizeof(**output));
    *error = 0;
    return *output ? GHOSTOS_VM_TERMINAL_PLATFORM_OK : GHOSTOS_VM_TERMINAL_PLATFORM_ALLOCATION;
}
uint32_t ghostos_vm_terminal_mode_restore(ghostos_vm_terminal_mode *mode, int32_t *error) {
    (void)mode;
    if (error) *error = 0;
    return GHOSTOS_VM_TERMINAL_PLATFORM_OK;
}
void ghostos_vm_terminal_mode_free(ghostos_vm_terminal_mode *mode) { free(mode); }
bool ghostos_vm_terminal_size(uint16_t *rows, uint16_t *columns) {
    (void)rows; (void)columns;
    return false;
}
#endif
