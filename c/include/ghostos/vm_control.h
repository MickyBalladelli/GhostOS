#ifndef GHOSTOS_VM_CONTROL_H
#define GHOSTOS_VM_CONTROL_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#define GHOSTOS_MONITOR_REQUEST_LIMIT 4096
#define GHOSTOS_MONITOR_COMMAND_LIMIT 2048
#define GHOSTOS_MONITOR_RESPONSE_LIMIT 65536
#define GHOSTOS_MONITOR_NONCES 1024

typedef struct { uint8_t bytes[4096]; size_t length; } ghostos_monitor_buffer;
/* Framing: 0 pending, 1 complete, 2 request too large, 3 partial command,
 * 4 multiple commands, 5 invalid UTF-8. Complete receives command length. */
uint32_t ghostos_monitor_push(ghostos_monitor_buffer *state, const uint8_t *bytes,
    size_t length, size_t *command_length);
uint32_t ghostos_monitor_frame(const ghostos_monitor_buffer *state, bool eof, size_t *command_length);
/* Commands: kind 0 help, 1 info, 2 save, 3 quit; topic 0 status, 1 devices,
 * 2 disks, 3 snapshots, 4 migration, 5 registers. Offsets borrow input bytes. */
typedef struct { uint32_t kind, topic; bool sensitive; size_t path_start, path_length; } ghostos_monitor_command;
/* Parse: 0 success, 1 command too large, 2 missing save path, 3 unknown,
 * 4 invalid UTF-8 (Rust string adapters cannot supply this case). */
uint32_t ghostos_monitor_parse(const uint8_t *input, size_t length, ghostos_monitor_command *out);
/* Permissions: 0 success, 1 empty list, 2 invalid name, 3 invalid UTF-8.
 * Invalid-name offsets identify the exact trimmed token. */
uint32_t ghostos_monitor_permissions(const uint8_t *input, size_t length,
    uint8_t *permissions, size_t *error_start, size_t *error_length);
/* JSON output excludes a terminator; returns required bytes, SIZE_MAX on
 * overflow. A short/null output is a size query and writes no bytes. */
size_t ghostos_monitor_json_string(const uint8_t *input, size_t length, uint8_t *output, size_t capacity);
bool ghostos_monitor_response_fits(size_t length);
size_t ghostos_monitor_envelope(const uint8_t *command, size_t command_length,
    const uint8_t *data, size_t data_length, uint8_t *output, size_t capacity);
size_t ghostos_monitor_failure(bool has_command, const uint8_t *command, size_t command_length,
    const uint8_t *code, size_t code_length, const uint8_t *message, size_t message_length,
    uint8_t *output, size_t capacity);
size_t ghostos_monitor_action(const uint8_t *command, size_t command_length,
    const uint8_t *action, size_t action_length, uint8_t *output, size_t capacity);
size_t ghostos_monitor_help(uint8_t *output, size_t capacity);

typedef struct ghostos_monitor_nonces ghostos_monitor_nonces;
ghostos_monitor_nonces *ghostos_monitor_nonces_new(void);
void ghostos_monitor_nonces_free(ghostos_monitor_nonces *state);
typedef struct {
    bool (*now)(void *context, uint64_t *seconds);
    bool (*verify)(void *context, uint64_t timestamp, const uint8_t nonce[32],
        const uint8_t *command, size_t length, const uint8_t tag[32]);
    void *context;
} ghostos_monitor_auth_io;
typedef struct {
    size_t command_start, command_length;
    ghostos_monitor_command command;
    uint32_t parse_error;
    uint8_t missing_permission;
} ghostos_monitor_auth_result;
/* Auth errors 1..18 identify the original ordered authentication diagnostics.
 * Callbacks are synchronous and never retained. Nonces are remembered only
 * after signature, command, and both permission checks succeed. */
uint32_t ghostos_monitor_authenticate(ghostos_monitor_nonces *state, uint8_t permissions,
    const uint8_t *input, size_t length, const ghostos_monitor_auth_io *io,
    ghostos_monitor_auth_result *out);
#endif
