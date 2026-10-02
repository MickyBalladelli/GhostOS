#include "ghostos/vm_control.h"
#include <assert.h>
#include <string.h>

static void partial_monitor_command_waits_for_newline(void) {
    ghostos_monitor_buffer state = {0};
    size_t length = 0;
    const char first[] = "auth 1 0000", second[] = " status\n";
    assert(ghostos_monitor_push(&state, (const uint8_t *)first, sizeof(first) - 1, &length) == 0);
    assert(ghostos_monitor_push(&state, (const uint8_t *)second, sizeof(second) - 1, &length) == 1);
    assert(length == strlen("auth 1 0000 status"));
    assert(!memcmp(state.bytes, "auth 1 0000 status", length));
}

static void closed_partial_monitor_command_is_rejected(void) {
    ghostos_monitor_buffer state = {0};
    size_t length = 0;
    const char input[] = "auth incomplete";
    assert(ghostos_monitor_push(&state, (const uint8_t *)input, sizeof(input) - 1, &length) == 0);
    assert(ghostos_monitor_frame(&state, true, &length) == 3);
}

static void oversized_monitor_request_is_rejected(void) {
    ghostos_monitor_buffer state = {0};
    uint8_t oversized[GHOSTOS_MONITOR_REQUEST_LIMIT + 1];
    memset(oversized, 'a', sizeof(oversized));
    size_t length = 0;
    assert(ghostos_monitor_push(&state, oversized, sizeof(oversized), &length) == 2);
}

static void malformed_monitor_framing_and_encoding_are_rejected(void) {
    ghostos_monitor_buffer state = {0};
    size_t length = 0;
    const char multiple[] = "first\nsecond\n";
    assert(ghostos_monitor_push(&state, (const uint8_t *)multiple, sizeof(multiple) - 1, &length) == 4);
    ghostos_monitor_buffer invalid = {0};
    const uint8_t bytes[] = {0xff, '\n'};
    assert(ghostos_monitor_push(&invalid, bytes, sizeof(bytes), &length) == 5);
}

static void command_and_response_limits_are_enforced(void) {
    uint8_t command[GHOSTOS_MONITOR_COMMAND_LIMIT + 1];
    memset(command, 'x', sizeof(command));
    ghostos_monitor_command result;
    assert(ghostos_monitor_parse(command, sizeof(command), &result) == 1);
    assert(!ghostos_monitor_response_fits(GHOSTOS_MONITOR_RESPONSE_LIMIT + 1));
    /* The Rust adapter still assembles the response-too-large error envelope. */
}

int main(void) {
    partial_monitor_command_waits_for_newline();
    closed_partial_monitor_command_is_rejected();
    oversized_monitor_request_is_rejected();
    malformed_monitor_framing_and_encoding_are_rejected();
    command_and_response_limits_are_enforced();
    return 0;
}
