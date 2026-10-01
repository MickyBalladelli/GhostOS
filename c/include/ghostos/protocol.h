#ifndef GHOSTOS_PROTOCOL_H
#define GHOSTOS_PROTOCOL_H
#include "ghostos/abi.h"
#include "ghostos/api_compat.h"

#define GHOSTOS_CURRENT_PROTOCOL_VERSION GHOSTOS_ABI_SCHEMA_VERSION
#define GHOSTOS_REPLAY_WINDOW_BITS 64u

typedef uint8_t ghostos_traffic_class;
enum {
    GHOSTOS_TRAFFIC_HTTP = 1, GHOSTOS_TRAFFIC_GRPC, GHOSTOS_TRAFFIC_SDK,
    GHOSTOS_TRAFFIC_REMOTE_TERMINAL, GHOSTOS_TRAFFIC_MESH, GHOSTOS_TRAFFIC_CLUSTER
};
typedef struct {
    size_t max_message_bytes, max_inflight_bytes;
    uint16_t max_inflight_messages;
    uint8_t max_auth_failures;
    uint64_t reconnect_base_delay_us;
} ghostos_protocol_limits;
typedef struct { uint16_t minimum, maximum; } ghostos_protocol_version_range;
typedef enum {
    GHOSTOS_PROTOCOL_OK, GHOSTOS_PROTOCOL_INVALID_VERSION_RANGE, GHOSTOS_PROTOCOL_NO_COMMON_VERSION,
    GHOSTOS_PROTOCOL_WRONG_TRAFFIC_CLASS, GHOSTOS_PROTOCOL_NOT_NEGOTIATED, GHOSTOS_PROTOCOL_MESSAGE_TOO_LARGE,
    GHOSTOS_PROTOCOL_INVALID_SEQUENCE, GHOSTOS_PROTOCOL_REPLAY, GHOSTOS_PROTOCOL_SEQUENCE_TOO_OLD,
    GHOSTOS_PROTOCOL_AUTHENTICATION_FAILED, GHOSTOS_PROTOCOL_AUTHENTICATION_LOCKED,
    GHOSTOS_PROTOCOL_BACKPRESSURE, GHOSTOS_PROTOCOL_INVALID_RELEASE, GHOSTOS_PROTOCOL_RECONNECT_EXHAUSTED,
    GHOSTOS_PROTOCOL_INVALID_ARGUMENT
} ghostos_protocol_error_kind;
typedef struct {
    ghostos_protocol_error_kind kind;
    size_t limit, actual;
} ghostos_protocol_error;

/* Caller owns the guard and serializes access, as with Rust's mutable borrow. */
typedef struct {
    ghostos_traffic_class class;
    ghostos_protocol_limits limits;
    ghostos_protocol_version_range local_versions;
    uint16_t negotiated_version; /* zero means None */
    uint64_t highest_sequence, seen;
    bool replay_initialized;
    uint8_t auth_failures;
    bool auth_locked;
    size_t inflight_bytes;
    uint16_t inflight_messages;
    uint8_t reconnect_attempts;
    uint64_t next_retry_at_us;
    bool reconnect_exhausted;
} ghostos_protocol_guard;

bool ghostos_traffic_limits(ghostos_traffic_class class, ghostos_protocol_limits *out);
bool ghostos_protocol_version_range_new(uint16_t minimum, uint16_t maximum, ghostos_protocol_version_range *out);
bool ghostos_protocol_version_contains(ghostos_protocol_version_range range, uint16_t version);
ghostos_protocol_error ghostos_protocol_negotiate_versions(ghostos_protocol_version_range local,
    ghostos_protocol_version_range peer, uint16_t *selected);
ghostos_protocol_error ghostos_protocol_guard_new(ghostos_traffic_class class,
    ghostos_protocol_version_range local, ghostos_protocol_guard *out);
ghostos_protocol_error ghostos_protocol_require_class(const ghostos_protocol_guard *guard, ghostos_traffic_class class);
ghostos_protocol_error ghostos_protocol_negotiate(ghostos_protocol_guard *guard, ghostos_protocol_version_range peer, uint16_t *selected);
ghostos_protocol_error ghostos_protocol_validate_message(const ghostos_protocol_guard *guard, size_t bytes);
ghostos_protocol_error ghostos_protocol_accept_sequence(ghostos_protocol_guard *guard, uint64_t sequence);
bool ghostos_protocol_highest_sequence(const ghostos_protocol_guard *guard, uint64_t *out);
ghostos_protocol_error ghostos_protocol_authenticate(ghostos_protocol_guard *guard, bool success);
ghostos_protocol_error ghostos_protocol_reserve_message(ghostos_protocol_guard *guard, size_t bytes);
ghostos_protocol_error ghostos_protocol_release_message(ghostos_protocol_guard *guard, size_t bytes);
ghostos_protocol_error ghostos_protocol_disconnected(ghostos_protocol_guard *guard, uint64_t now_us, uint64_t *retry_at);
bool ghostos_protocol_reconnect_due(const ghostos_protocol_guard *guard, uint64_t now_us);
void ghostos_protocol_reconnected(ghostos_protocol_guard *guard);
#endif
