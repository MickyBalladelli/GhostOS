#ifndef GHOSTOS_VM_PASSKEY_H
#define GHOSTOS_VM_PASSKEY_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#define GHOSTOS_PASSKEY_REQUEST_LIMIT 16384
/* Asset kinds: page=0, style=1, script=2. Borrowed immutable UTF-8 bytes. */
const uint8_t *ghostos_passkey_asset(uint32_t kind, size_t *length);
bool ghostos_passkey_request_fits(size_t current, size_t incoming);
typedef struct {
    size_t method_start, method_length, target_start, target_length;
    size_t token_start, token_length, body_start, body_length;
    bool has_token;
} ghostos_passkey_request;
/* HTTP parse: 0 pending, 1 complete, 2 invalid data, 3 checked-add panic,
 * 4 wrapped slice-range panic. Offsets borrow the supplied request bytes. */
uint32_t ghostos_passkey_request_parse(const uint8_t *bytes, size_t length,
    bool checked, ghostos_passkey_request *request);
bool ghostos_passkey_query(const uint8_t *target, size_t length,
    const uint8_t *wanted, size_t wanted_length, size_t *start, size_t *size);
bool ghostos_passkey_percent_decode(const uint8_t *input, size_t length,
    uint8_t *output, size_t *written);
bool ghostos_passkey_valid_username(const uint8_t *input, size_t length);
bool ghostos_passkey_valid_hex(const uint8_t *input, size_t length, size_t maximum);
bool ghostos_passkey_decode_hex(const uint8_t *input, size_t length, uint8_t *output);
/* Frame modes: 0 CR-terminated line (including empty), 1 nonempty username
 * limited to 32 bytes, 2 nonempty length-prefixed bytes limited by maximum.
 * Zero means invalid/short output; otherwise returns bytes written. */
size_t ghostos_passkey_frame(const uint8_t *input, size_t length, uint32_t mode,
    size_t maximum, uint8_t *output, size_t capacity);
bool ghostos_passkey_enrollment_pending(const uint8_t *text, size_t length);
bool ghostos_passkey_login_pending(const uint8_t *text, size_t length);
bool ghostos_passkey_succeeded(const uint8_t *text, size_t length);
bool ghostos_passkey_challenge(const uint8_t *text, size_t length, size_t *start);
/* Modes: waiting=0, enroll=1, login=2, challenge=3, success=4. */
typedef struct { uint32_t mode; bool committed, login_in_progress; } ghostos_passkey_state;
typedef struct {
    /* Challenge action 0 preserve, 1 clear, 2 set; error action 0 preserve,
     * 1 clear, 2 login rejection, 3 enrollment rejection. */
    uint32_t challenge_action, error_action;
    size_t challenge_start;
    bool clear_flow;
} ghostos_passkey_observation;
void ghostos_passkey_observe(ghostos_passkey_state *state,
    const uint8_t *text, size_t length, const uint8_t *new_text, size_t new_length,
    ghostos_passkey_observation *out);
#endif
