#include "ghostos/protocol.h"

_Static_assert(sizeof(ghostos_protocol_error_kind) == 4, "error tag size");
_Static_assert(sizeof(ghostos_protocol_error) == 24, "error size");
_Static_assert(offsetof(ghostos_protocol_error, limit) == 8, "error limit offset");
_Static_assert(offsetof(ghostos_protocol_error, actual) == 16, "error actual offset");
_Static_assert(sizeof(ghostos_protocol_limits) == 32, "limits size");
_Static_assert(offsetof(ghostos_protocol_limits, max_inflight_bytes) == 8, "inflight limit offset");
_Static_assert(offsetof(ghostos_protocol_limits, max_inflight_messages) == 16, "message limit offset");
_Static_assert(offsetof(ghostos_protocol_limits, max_auth_failures) == 18, "auth limit offset");
_Static_assert(offsetof(ghostos_protocol_limits, reconnect_base_delay_us) == 24, "retry delay offset");
_Static_assert(sizeof(ghostos_protocol_version_range) == 4, "version range size");
_Static_assert(offsetof(ghostos_protocol_version_range, maximum) == 2, "maximum offset");
_Static_assert(sizeof(ghostos_protocol_guard) == 104, "guard size");
_Static_assert(offsetof(ghostos_protocol_guard, limits) == 8, "limits offset");
_Static_assert(offsetof(ghostos_protocol_guard, local_versions) == 40, "local_versions offset");
_Static_assert(offsetof(ghostos_protocol_guard, negotiated_version) == 44, "negotiated_version offset");
_Static_assert(offsetof(ghostos_protocol_guard, highest_sequence) == 48, "highest_sequence offset");
_Static_assert(offsetof(ghostos_protocol_guard, seen) == 56, "seen offset");
_Static_assert(offsetof(ghostos_protocol_guard, replay_initialized) == 64, "replay_initialized offset");
_Static_assert(offsetof(ghostos_protocol_guard, auth_failures) == 65, "auth_failures offset");
_Static_assert(offsetof(ghostos_protocol_guard, auth_locked) == 66, "auth_locked offset");
_Static_assert(offsetof(ghostos_protocol_guard, inflight_bytes) == 72, "inflight_bytes offset");
_Static_assert(offsetof(ghostos_protocol_guard, inflight_messages) == 80, "inflight_messages offset");
_Static_assert(offsetof(ghostos_protocol_guard, reconnect_attempts) == 82, "reconnect_attempts offset");
_Static_assert(offsetof(ghostos_protocol_guard, next_retry_at_us) == 88, "next_retry_at_us offset");
_Static_assert(offsetof(ghostos_protocol_guard, reconnect_exhausted) == 96, "reconnect_exhausted offset");

static ghostos_protocol_error result(ghostos_protocol_error_kind kind) {
    return (ghostos_protocol_error){kind, 0, 0};
}
bool ghostos_traffic_limits(ghostos_traffic_class class, ghostos_protocol_limits *out) {
    if (out == NULL) return false;
    switch (class) {
        case GHOSTOS_TRAFFIC_HTTP: *out = (ghostos_protocol_limits){65536, 65536, 32, 3, 250000}; break;
        case GHOSTOS_TRAFFIC_GRPC: *out = (ghostos_protocol_limits){1048576, 1048576, 64, 3, 1000000}; break;
        case GHOSTOS_TRAFFIC_SDK: *out = (ghostos_protocol_limits){4096, 4096, 64, 3, 16384}; break;
        case GHOSTOS_TRAFFIC_REMOTE_TERMINAL: *out = (ghostos_protocol_limits){16384, 16384, 32, 3, 65536}; break;
        case GHOSTOS_TRAFFIC_MESH:
        case GHOSTOS_TRAFFIC_CLUSTER: *out = (ghostos_protocol_limits){4096, 4096, 64, 3, 65536}; break;
        default: return false;
    }
    return true;
}
static bool range_valid(ghostos_protocol_version_range range) {
    return range.minimum != 0 && range.minimum <= range.maximum;
}
bool ghostos_protocol_version_range_new(uint16_t minimum, uint16_t maximum, ghostos_protocol_version_range *out) {
    ghostos_protocol_version_range range = {minimum, maximum};
    if (!range_valid(range) || out == NULL) return false;
    *out = range;
    return true;
}
bool ghostos_protocol_version_contains(ghostos_protocol_version_range range, uint16_t version) {
    return version >= range.minimum && version <= range.maximum;
}
ghostos_protocol_error ghostos_protocol_negotiate_versions(ghostos_protocol_version_range local,
    ghostos_protocol_version_range peer, uint16_t *selected) {
    if (!range_valid(local) || !range_valid(peer)) return result(GHOSTOS_PROTOCOL_INVALID_VERSION_RANGE);
    uint16_t version = local.maximum < peer.maximum ? local.maximum : peer.maximum;
    if (version < local.minimum || version < peer.minimum) return result(GHOSTOS_PROTOCOL_NO_COMMON_VERSION);
    if (selected == NULL) return result(GHOSTOS_PROTOCOL_INVALID_ARGUMENT);
    *selected = version;
    return result(GHOSTOS_PROTOCOL_OK);
}
ghostos_protocol_error ghostos_protocol_guard_new(ghostos_traffic_class class,
    ghostos_protocol_version_range local, ghostos_protocol_guard *out) {
    if (!range_valid(local)) return result(GHOSTOS_PROTOCOL_INVALID_VERSION_RANGE);
    ghostos_protocol_guard guard = {0};
    if (!ghostos_traffic_limits(class, &guard.limits)) return result(GHOSTOS_PROTOCOL_WRONG_TRAFFIC_CLASS);
    if (out == NULL) return result(GHOSTOS_PROTOCOL_INVALID_ARGUMENT);
    guard.class = class;
    guard.local_versions = local;
    *out = guard;
    return result(GHOSTOS_PROTOCOL_OK);
}
ghostos_protocol_error ghostos_protocol_require_class(const ghostos_protocol_guard *guard, ghostos_traffic_class class) {
    return result(guard->class == class ? GHOSTOS_PROTOCOL_OK : GHOSTOS_PROTOCOL_WRONG_TRAFFIC_CLASS);
}
ghostos_protocol_error ghostos_protocol_negotiate(ghostos_protocol_guard *guard, ghostos_protocol_version_range peer, uint16_t *selected) {
    uint16_t version;
    ghostos_protocol_error error = ghostos_protocol_negotiate_versions(guard->local_versions, peer, &version);
    if (error.kind != GHOSTOS_PROTOCOL_OK) return error;
    guard->negotiated_version = version;
    if (selected != NULL) *selected = version;
    return result(GHOSTOS_PROTOCOL_OK);
}
ghostos_protocol_error ghostos_protocol_validate_message(const ghostos_protocol_guard *guard, size_t bytes) {
    if (guard->negotiated_version == 0) return result(GHOSTOS_PROTOCOL_NOT_NEGOTIATED);
    if (bytes > guard->limits.max_message_bytes)
        return (ghostos_protocol_error){GHOSTOS_PROTOCOL_MESSAGE_TOO_LARGE, guard->limits.max_message_bytes, bytes};
    return result(GHOSTOS_PROTOCOL_OK);
}
ghostos_protocol_error ghostos_protocol_accept_sequence(ghostos_protocol_guard *guard, uint64_t sequence) {
    if (sequence == 0) return result(GHOSTOS_PROTOCOL_INVALID_SEQUENCE);
    if (!guard->replay_initialized) {
        guard->replay_initialized = true;
        guard->highest_sequence = sequence;
        guard->seen = 1;
        return result(GHOSTOS_PROTOCOL_OK);
    }
    if (sequence > guard->highest_sequence) {
        uint64_t shift = sequence - guard->highest_sequence;
        guard->seen = shift >= GHOSTOS_REPLAY_WINDOW_BITS ? 1 : (guard->seen << shift) | 1;
        guard->highest_sequence = sequence;
        return result(GHOSTOS_PROTOCOL_OK);
    }
    uint64_t offset = guard->highest_sequence - sequence;
    if (offset >= GHOSTOS_REPLAY_WINDOW_BITS) return result(GHOSTOS_PROTOCOL_SEQUENCE_TOO_OLD);
    uint64_t bit = UINT64_C(1) << offset;
    if ((guard->seen & bit) != 0) return result(GHOSTOS_PROTOCOL_REPLAY);
    guard->seen |= bit;
    return result(GHOSTOS_PROTOCOL_OK);
}
bool ghostos_protocol_highest_sequence(const ghostos_protocol_guard *guard, uint64_t *out) {
    if (!guard->replay_initialized || out == NULL) return false;
    *out = guard->highest_sequence;
    return true;
}
ghostos_protocol_error ghostos_protocol_authenticate(ghostos_protocol_guard *guard, bool success) {
    if (guard->auth_locked) return result(GHOSTOS_PROTOCOL_AUTHENTICATION_LOCKED);
    if (success) { guard->auth_failures = 0; return result(GHOSTOS_PROTOCOL_OK); }
    if (guard->auth_failures != UINT8_MAX) ++guard->auth_failures;
    if (guard->auth_failures >= guard->limits.max_auth_failures) {
        guard->auth_locked = true;
        return result(GHOSTOS_PROTOCOL_AUTHENTICATION_LOCKED);
    }
    return result(GHOSTOS_PROTOCOL_AUTHENTICATION_FAILED);
}
ghostos_protocol_error ghostos_protocol_reserve_message(ghostos_protocol_guard *guard, size_t bytes) {
    ghostos_protocol_error error = ghostos_protocol_validate_message(guard, bytes);
    if (error.kind != GHOSTOS_PROTOCOL_OK) return error;
    if (bytes > SIZE_MAX - guard->inflight_bytes || guard->inflight_messages == UINT16_MAX)
        return result(GHOSTOS_PROTOCOL_BACKPRESSURE);
    size_t next_bytes = guard->inflight_bytes + bytes;
    uint16_t next_messages = guard->inflight_messages + 1;
    if (next_bytes > guard->limits.max_inflight_bytes || next_messages > guard->limits.max_inflight_messages)
        return result(GHOSTOS_PROTOCOL_BACKPRESSURE);
    guard->inflight_bytes = next_bytes;
    guard->inflight_messages = next_messages;
    return result(GHOSTOS_PROTOCOL_OK);
}
ghostos_protocol_error ghostos_protocol_release_message(ghostos_protocol_guard *guard, size_t bytes) {
    if (guard->inflight_messages == 0 || bytes > guard->inflight_bytes) return result(GHOSTOS_PROTOCOL_INVALID_RELEASE);
    guard->inflight_bytes -= bytes;
    --guard->inflight_messages;
    return result(GHOSTOS_PROTOCOL_OK);
}
ghostos_protocol_error ghostos_protocol_disconnected(ghostos_protocol_guard *guard, uint64_t now_us, uint64_t *retry_at) {
    if (guard->reconnect_attempts >= 8) {
        guard->reconnect_exhausted = true;
        return result(GHOSTOS_PROTOCOL_RECONNECT_EXHAUSTED);
    }
    uint64_t multiplier = UINT64_C(1) << guard->reconnect_attempts;
    uint64_t base = guard->limits.reconnect_base_delay_us;
    uint64_t delay = base > UINT64_MAX / multiplier ? UINT64_MAX : base * multiplier;
    ++guard->reconnect_attempts;
    guard->next_retry_at_us = now_us > UINT64_MAX - delay ? UINT64_MAX : now_us + delay;
    if (retry_at != NULL) *retry_at = guard->next_retry_at_us;
    return result(GHOSTOS_PROTOCOL_OK);
}
bool ghostos_protocol_reconnect_due(const ghostos_protocol_guard *guard, uint64_t now_us) {
    return guard->reconnect_attempts != 0 && !guard->reconnect_exhausted && now_us >= guard->next_retry_at_us;
}
void ghostos_protocol_reconnected(ghostos_protocol_guard *guard) {
    guard->reconnect_attempts = 0;
    guard->next_retry_at_us = 0;
    guard->reconnect_exhausted = false;
}
