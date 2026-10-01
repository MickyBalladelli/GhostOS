#include "ghostos/abi.h"
#include "ghostos/api_compat.h"
#include "ghostos/boot_protocol.h"
#include "ghostos/protocol.h"
#include "ghostos/status.h"
#include "ghostos/test_property.h"
#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void run_property(const char *name, size_t cases, ghostos_property_predicate check) {
    ghostos_property_config config = ghostos_property_config_with_env(ghostos_property_config_new(0x593, cases));
    ghostos_property_failure failure;
    ghostos_property_result result = ghostos_property_run_assert(name, config, check, NULL, &failure);
    if (result == GHOSTOS_PROPERTY_FAILED) {
        char message[1024];
        if (ghostos_property_failure_format(&failure, message, sizeof(message)) >= 0) fprintf(stderr, "%s\n", message);
    } else if (result != GHOSTOS_PROPERTY_OK) {
        fprintf(stderr, "property %s runner failed with error %d\n", name, (int)result);
    }
    ghostos_property_failure_dispose(&failure);
    if (result != GHOSTOS_PROPERTY_OK) abort();
}

static bool status_round_trip_property(size_t case_index, uint64_t case_seed,
                                      ghostos_test_entropy *entropy, void *context) {
    (void)case_index;
    (void)case_seed;
    (void)context;
    ghostos_severity severity = (ghostos_severity)(ghostos_test_entropy_next_u64(entropy) % 5);
    uint16_t facility = (uint16_t)(ghostos_test_entropy_next_u64(entropy) % 0x1000);
    uint16_t code = (uint16_t)(ghostos_test_entropy_next_u64(entropy) % 0x2000);
    uint8_t flags = (uint8_t)(ghostos_test_entropy_next_u64(entropy) % 16);
    ghostos_status status, decoded;
    return ghostos_status_new(severity, facility, code, flags, &status) &&
        ghostos_status_from_raw(status, &decoded) && status == decoded;
}

static void status_contract(void) {
    ghostos_status status;
    assert(ghostos_status_new(GHOSTOS_SEVERITY_INFORMATION, GHOSTOS_FACILITY_NETWORK, 0x123, 0xa, &status));
    assert(ghostos_status_severity(status) == GHOSTOS_SEVERITY_INFORMATION);
    assert(ghostos_status_facility(status) == GHOSTOS_FACILITY_NETWORK);
    assert(ghostos_status_code(status) == 0x123);
    assert(ghostos_status_flags(status) == 0xa);
    ghostos_status decoded;
    assert(ghostos_status_from_raw(status, &decoded) && decoded == status);
    assert(!ghostos_status_new(GHOSTOS_SEVERITY_ERROR, 0x1000, 1, 0, &decoded));
    assert(!ghostos_status_new(GHOSTOS_SEVERITY_ERROR, 1, 0x2000, 0, &decoded));
    assert(!ghostos_status_new(GHOSTOS_SEVERITY_ERROR, 1, 1, 0x10, &decoded));
    assert(!ghostos_status_from_raw(7, &decoded));
    assert(ghostos_status_is_success(GHOSTOS_STATUS_NORMAL));
    assert(!ghostos_status_is_success(GHOSTOS_STATUS_INVALID_ARGUMENT));
    assert(!ghostos_status_is_success(GHOSTOS_STATUS_BUSY));
    ghostos_audit_context audit = ghostos_audit_context_new(0xfeed, UINT64_MAX, 3);
    ghostos_public_error error = ghostos_public_error_new(GHOSTOS_STATUS_ACCESS_DENIED, 7,
        (ghostos_retry_hint){GHOSTOS_RETRY_NEVER, 0}, audit);
    assert(error.code == GHOSTOS_STATUS_ACCESS_DENIED && error.operation == 7);
    assert(!ghostos_retry_is_retryable(error.retry));
    assert(error.audit.correlation_low == 0xfeed && error.audit.correlation_high == UINT64_MAX && error.audit.node == 3);
    error = ghostos_status_public_error(GHOSTOS_STATUS_BUSY, 12, ghostos_audit_context_new(8, 0, 1));
    assert(error.retry.kind == GHOSTOS_RETRY_AFTER_US && error.retry.delay_us == 1000000);
    assert(error.code == GHOSTOS_STATUS_BUSY && error.operation == 12 && error.audit.correlation_low == 8);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_NORMAL), "normal") == 0);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_ACCESS_DENIED), "access denied") == 0);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_METHOD_NOT_ALLOWED), "method not allowed") == 0);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_DIRECTORY_NOT_EMPTY), "directory not empty") == 0);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_INVALID_PATH), "invalid path") == 0);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_NOT_DIRECTORY), "not a directory") == 0);
    assert(strcmp(ghostos_status_message(GHOSTOS_STATUS_READ_ONLY), "read-only mount") == 0);
    assert(ghostos_status_new(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_KERNEL, 0x1f, 0, &status));
    assert(strcmp(ghostos_status_message(status), "unknown status") == 0);
    run_property("status.raw-round-trip", 256, status_round_trip_property);
}

static void abi_contract(void) {
    ghostos_request request = ghostos_request_new(GHOSTOS_OP_CLOCK_NOW);
    assert(request.abi_version == GHOSTOS_ABI_SCHEMA_VERSION);
    assert(sizeof(request) == 64 && sizeof(ghostos_response) == 40);
    ghostos_rpc_frame_header header = {GHOSTOS_RPC_METHOD_CLUSTER_STATE, 0, 7, 0, GHOSTOS_RPC_STATUS_OK};
    uint8_t frame[GHOSTOS_RPC_FRAME_HEADER_BYTES];
    assert(ghostos_rpc_frame_encode(header, frame, sizeof(frame)) == GHOSTOS_FRAME_OK);
    const uint8_t expected[24] = {'S', 'Y', 'R', 'P', 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 0};
    assert(memcmp(frame, expected, sizeof(expected)) == 0);
    ghostos_rpc_frame_header decoded;
    assert(ghostos_rpc_frame_decode(frame, sizeof(frame), &decoded) == GHOSTOS_FRAME_OK);
    assert(decoded.request_id == 7 && decoded.method == GHOSTOS_RPC_METHOD_CLUSTER_STATE);
    frame[4] = GHOSTOS_RPC_PROTOCOL_VERSION + 1;
    assert(ghostos_rpc_frame_decode(frame, sizeof(frame), &decoded) == GHOSTOS_FRAME_UNSUPPORTED_VERSION);
    assert(GHOSTOS_RPC_STATUS_PROTOCOL_MISMATCH == 8);
    frame[4] = GHOSTOS_RPC_PROTOCOL_VERSION;
    frame[7] = 2;
    assert(ghostos_rpc_frame_decode(frame, sizeof(frame), &decoded) == GHOSTOS_FRAME_INVALID_FLAGS);
    frame[7] = 0;
    frame[19] = 1;
    assert(ghostos_rpc_frame_decode(frame, sizeof(frame), &decoded) == GHOSTOS_FRAME_INVALID_LENGTH);
    frame[19] = 0;
    frame[5] = 255;
    assert(ghostos_rpc_frame_decode(frame, sizeof(frame), &decoded) == GHOSTOS_FRAME_UNKNOWN_METHOD);
    frame[5] = 1;
    frame[21] = 255;
    assert(ghostos_rpc_frame_decode(frame, sizeof(frame), &decoded) == GHOSTOS_FRAME_INVALID_STATUS);
    ghostos_capability capability;
    assert(!ghostos_capability_from_raw(UINT32_MAX, &capability));
    assert(ghostos_capability_from_raw(UINT64_C(1) << 32, &capability));
    request = ghostos_request_with_capability(request, capability);
    request = ghostos_request_with_buffer(request, (ghostos_shared_buffer){3, 4, 5, true});
    assert(request.capability == capability && request.arguments[0] == 3 && request.arguments[1] == 4);
    assert(request.arguments[2] == 5 && request.arguments[3] == 1);
}

static void compatibility_contract(void) {
    for (size_t i = 0; i < 8; ++i) {
        ghostos_api_contract contract = ghostos_all_contracts[i];
        assert(ghostos_api_range_is_valid(contract.supported));
        assert(ghostos_api_contract_accepts(contract, contract.current));
        assert(ghostos_api_contract_check(contract, contract.current).kind == GHOSTOS_COMPAT_ACCEPTED);
    }
    ghostos_api_contract wire;
    assert(ghostos_api_contract_get(GHOSTOS_API_WIRE, &wire));
    ghostos_compatibility result = ghostos_api_contract_check(wire, (ghostos_api_version){0, 9});
    assert(result.kind == GHOSTOS_COMPAT_REJECTED && strcmp(result.error.code, GHOSTOS_COMPAT_TOO_OLD) == 0);
    char formatted[160];
    const char *expected = "GHOSTOS-COMPAT-001: wire API version 0.9 is outside supported range 1.0..=1.0";
    assert(ghostos_compatibility_error_format(result.error, formatted, sizeof(formatted)) == strlen(expected));
    assert(strcmp(formatted, expected) == 0);
    assert(ghostos_compatibility_error_format(result.error, formatted, 1) == strlen(expected));
    assert(formatted[0] == '\0');
    result = ghostos_api_contract_check(wire, GHOSTOS_API_V2);
    assert(result.kind == GHOSTOS_COMPAT_REJECTED && strcmp(result.error.code, GHOSTOS_COMPAT_TOO_NEW) == 0);
    ghostos_api_contract snapshot;
    assert(ghostos_api_contract_get(GHOSTOS_API_SNAPSHOT, &snapshot));
    result = ghostos_api_contract_check(snapshot, GHOSTOS_API_V1);
    assert(result.kind == GHOSTOS_COMPAT_DEPRECATED);
    ghostos_api_migration migration;
    ghostos_compatibility_error error;
    assert(ghostos_api_migrate_to_current(snapshot, GHOSTOS_API_V1, &migration, &error));
    assert(strcmp(migration.id, "snapshot-v1-to-v2") == 0);
    assert(!ghostos_api_migrate_to_current(snapshot, GHOSTOS_API_V2, &migration, &error));
    assert(strcmp(error.code, GHOSTOS_COMPAT_MIGRATION_REQUIRED) == 0);
    ghostos_api_contract sdk;
    assert(ghostos_api_contract_get(GHOSTOS_API_SDK, &sdk));
    result = ghostos_api_contract_check(sdk, GHOSTOS_API_V2);
    assert(result.kind == GHOSTOS_COMPAT_REJECTED && strcmp(result.error.code, GHOSTOS_COMPAT_TOO_NEW) == 0);
}

static void protocol_contract(void) {
    ghostos_protocol_version_range versions = {1, 1};
    uint16_t selected;
    assert(ghostos_protocol_negotiate_versions((ghostos_protocol_version_range){2, 3}, versions, &selected).kind == GHOSTOS_PROTOCOL_NO_COMMON_VERSION);
    for (ghostos_traffic_class class = GHOSTOS_TRAFFIC_HTTP; class <= GHOSTOS_TRAFFIC_CLUSTER; ++class) {
        ghostos_protocol_guard guard;
        assert(ghostos_protocol_guard_new(class, versions, &guard).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_validate_message(&guard, 1).kind == GHOSTOS_PROTOCOL_NOT_NEGOTIATED);
        assert(ghostos_protocol_negotiate(&guard, versions, &selected).kind == GHOSTOS_PROTOCOL_OK);
        assert(selected == 1 && guard.negotiated_version == 1);
        assert(ghostos_protocol_require_class(&guard, class).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_validate_message(&guard, guard.limits.max_message_bytes).kind == GHOSTOS_PROTOCOL_OK);
        ghostos_protocol_error error = ghostos_protocol_validate_message(&guard, guard.limits.max_message_bytes + 1);
        assert(error.kind == GHOSTOS_PROTOCOL_MESSAGE_TOO_LARGE && error.limit == guard.limits.max_message_bytes && error.actual == error.limit + 1);
        assert(ghostos_protocol_accept_sequence(&guard, 10).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_accept_sequence(&guard, 10).kind == GHOSTOS_PROTOCOL_REPLAY);
        assert(ghostos_protocol_accept_sequence(&guard, 9).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_accept_sequence(&guard, 75).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_accept_sequence(&guard, 10).kind == GHOSTOS_PROTOCOL_SEQUENCE_TOO_OLD);
        assert(ghostos_protocol_authenticate(&guard, false).kind == GHOSTOS_PROTOCOL_AUTHENTICATION_FAILED);
        assert(ghostos_protocol_authenticate(&guard, true).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_reserve_message(&guard, 1).kind == GHOSTOS_PROTOCOL_OK);
        assert(guard.inflight_bytes == 1 && guard.inflight_messages == 1);
        assert(ghostos_protocol_release_message(&guard, 1).kind == GHOSTOS_PROTOCOL_OK);
        assert(ghostos_protocol_release_message(&guard, 1).kind == GHOSTOS_PROTOCOL_INVALID_RELEASE);
        uint64_t retry_at;
        assert(ghostos_protocol_disconnected(&guard, 100, &retry_at).kind == GHOSTOS_PROTOCOL_OK);
        assert(retry_at == 100 + guard.limits.reconnect_base_delay_us);
        assert(!ghostos_protocol_reconnect_due(&guard, retry_at - 1));
        assert(ghostos_protocol_reconnect_due(&guard, retry_at));
        ghostos_protocol_reconnected(&guard);
        assert(guard.reconnect_attempts == 0);
    }
    ghostos_protocol_guard guard;
    assert(ghostos_protocol_guard_new(GHOSTOS_TRAFFIC_HTTP, versions, &guard).kind == GHOSTOS_PROTOCOL_OK);
    assert(ghostos_protocol_authenticate(&guard, false).kind == GHOSTOS_PROTOCOL_AUTHENTICATION_FAILED);
    assert(ghostos_protocol_authenticate(&guard, false).kind == GHOSTOS_PROTOCOL_AUTHENTICATION_FAILED);
    assert(ghostos_protocol_authenticate(&guard, false).kind == GHOSTOS_PROTOCOL_AUTHENTICATION_LOCKED);
    assert(guard.auth_locked && ghostos_protocol_authenticate(&guard, true).kind == GHOSTOS_PROTOCOL_AUTHENTICATION_LOCKED);
    assert(ghostos_protocol_guard_new(GHOSTOS_TRAFFIC_REMOTE_TERMINAL, versions, &guard).kind == GHOSTOS_PROTOCOL_OK);
    assert(ghostos_protocol_negotiate(&guard, versions, &selected).kind == GHOSTOS_PROTOCOL_OK);
    size_t limit = guard.limits.max_inflight_bytes;
    assert(ghostos_protocol_reserve_message(&guard, limit).kind == GHOSTOS_PROTOCOL_OK);
    assert(ghostos_protocol_reserve_message(&guard, 1).kind == GHOSTOS_PROTOCOL_BACKPRESSURE);
    assert(ghostos_protocol_release_message(&guard, limit).kind == GHOSTOS_PROTOCOL_OK);
    uint64_t retry_at;
    for (uint8_t attempt = 0; attempt < 8; ++attempt) {
        assert(ghostos_protocol_disconnected(&guard, attempt, &retry_at).kind == GHOSTOS_PROTOCOL_OK);
        assert(retry_at == attempt + guard.limits.reconnect_base_delay_us * (UINT64_C(1) << attempt));
    }
    assert(ghostos_protocol_disconnected(&guard, 9, &retry_at).kind == GHOSTOS_PROTOCOL_RECONNECT_EXHAUSTED);
}

static ghostos_memory_region region(uint64_t start, uint64_t length) {
    return (ghostos_memory_region){start, length, GHOSTOS_MEMORY_USABLE, 0};
}

static bool region_capacity_property(size_t case_index, uint64_t case_seed,
                                     ghostos_test_entropy *entropy, void *context) {
    (void)case_index;
    (void)case_seed;
    (void)context;
    size_t count = (size_t)ghostos_test_entropy_next_u64(entropy) % (GHOSTOS_MAX_MEMORY_REGIONS * 2 + 1);
    ghostos_boot_info info = ghostos_boot_info_empty(GHOSTOS_BOOT_BIOS);
    for (size_t i = 0; i < count; ++i) {
        bool accepted = ghostos_boot_info_push_region(&info, region(((uint64_t)i + 1) * 0x1000, 0x1000));
        if (accepted != (i < GHOSTOS_MAX_MEMORY_REGIONS)) return false;
    }
    return info.memory_region_count == (count < GHOSTOS_MAX_MEMORY_REGIONS ? count : GHOSTOS_MAX_MEMORY_REGIONS) &&
        ghostos_boot_info_is_valid(&info);
}

static void boot_contract(void) {
    ghostos_boot_info info = ghostos_boot_info_empty(GHOSTOS_BOOT_UEFI);
    assert(info.magic == GHOSTOS_BOOT_INFO_MAGIC && info.version == GHOSTOS_BOOT_INFO_VERSION);
    assert(ghostos_boot_info_is_valid(&info) && info.memory_region_count == 0);
    assert(_Alignof(ghostos_boot_info) == 16 && sizeof(ghostos_boot_info) % 16 == 0);
    assert(ghostos_boot_info_push_region(&info, region(0x1000, 0x2000)));
    assert(ghostos_boot_info_push_region(&info, region(0x8000, 0x1000)));
    const ghostos_memory_region *regions;
    size_t count;
    assert(ghostos_boot_info_regions(&info, &regions, &count));
    assert(count == 2 && regions[0].start == 0x1000 && regions[1].start == 0x8000);
    info = ghostos_boot_info_empty(GHOSTOS_BOOT_BIOS);
    for (size_t i = 0; i < GHOSTOS_MAX_MEMORY_REGIONS; ++i)
        assert(ghostos_boot_info_push_region(&info, region((i + 1) * 0x1000, 0x1000)));
    assert(!ghostos_boot_info_push_region(&info, region(0, 0x1000)));
    assert(info.memory_region_count == GHOSTOS_MAX_MEMORY_REGIONS);
    assert(ghostos_memory_region_end(region(UINT64_MAX - 1, 8)) == UINT64_MAX);
    ghostos_boot_method method;
    ghostos_memory_kind kind;
    assert(ghostos_boot_method_from_raw(1, &method) && method == GHOSTOS_BOOT_BIOS);
    assert(ghostos_boot_method_from_raw(2, &method) && method == GHOSTOS_BOOT_UEFI);
    assert(!ghostos_boot_method_from_raw(0, &method) && !ghostos_boot_method_from_raw(UINT32_MAX, &method));
    assert(ghostos_memory_kind_from_raw(GHOSTOS_MEMORY_FRAMEBUFFER, &kind) && kind == GHOSTOS_MEMORY_FRAMEBUFFER);
    assert(!ghostos_memory_kind_from_raw(0, &kind) && !ghostos_memory_kind_from_raw(8, &kind));
    info = ghostos_boot_info_empty(GHOSTOS_BOOT_BIOS);
    assert(!ghostos_boot_info_push_region(&info, region(0x1000, 0)));
    assert(ghostos_boot_info_push_region(&info, region(0x1000, 0x2000)));
    assert(!ghostos_boot_info_push_region(&info, region(0x2000, 0x1000)));
    info.memory_regions[0] = (ghostos_memory_region){UINT64_MAX - 1, 8, GHOSTOS_MEMORY_RESERVED, 0};
    assert(!ghostos_boot_info_is_valid(&info));
    info = ghostos_boot_info_empty(GHOSTOS_BOOT_UEFI);
    info.framebuffer = (ghostos_framebuffer_info){0x1000, 64, 4, 4, 4, GHOSTOS_FRAMEBUFFER_PIXEL_RGB};
    assert(ghostos_boot_info_is_valid(&info));
    info.framebuffer.pixel_format = 99;
    assert(!ghostos_boot_info_is_valid(&info));
    info.framebuffer.pixel_format = GHOSTOS_FRAMEBUFFER_PIXEL_BGR;
    info.framebuffer.size = 1;
    assert(!ghostos_boot_info_is_valid(&info));
    run_property("boot-protocol.region-capacity", 128, region_capacity_property);
    info = ghostos_boot_info_empty(GHOSTOS_BOOT_UEFI);
    assert(ghostos_boot_info_push_region(&info, region(0x1000, 0x2000)));
    info.memory_regions[1] = info.memory_regions[0];
    info.memory_region_count = 2;
    assert(!ghostos_boot_info_is_valid(&info));
    info.memory_region_count = 1;
    info.version = GHOSTOS_BOOT_INFO_VERSION - 1;
    assert(!ghostos_boot_info_is_valid(&info));
    info.version = GHOSTOS_BOOT_INFO_VERSION + 1;
    assert(!ghostos_boot_info_is_valid(&info));
    info.version = GHOSTOS_BOOT_INFO_VERSION;
    info.magic ^= 1;
    assert(!ghostos_boot_info_is_valid(&info));
    info.magic = GHOSTOS_BOOT_INFO_MAGIC;
    info.memory_regions[0].start = 0;
    assert(!ghostos_boot_info_is_valid(&info));
    info = ghostos_boot_info_empty(GHOSTOS_BOOT_BIOS);
    assert(ghostos_boot_info_is_valid(&info));
    info.framebuffer = (ghostos_framebuffer_info){0x1000, 4096, 32, 32, 32, GHOSTOS_FRAMEBUFFER_PIXEL_RGB};
    assert(ghostos_boot_info_is_valid(&info));
    info.framebuffer.stride = 31;
    assert(!ghostos_boot_info_is_valid(&info));
}

int main(void) {
    status_contract();
    abi_contract();
    compatibility_contract();
    protocol_contract();
    boot_contract();
    return 0;
}
