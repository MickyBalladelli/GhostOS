#ifndef GHOSTOS_STATUS_H
#define GHOSTOS_STATUS_H

#include <stdbool.h>
#include <stdint.h>

typedef uint32_t ghostos_status;
typedef uint8_t ghostos_severity;
enum {
    GHOSTOS_SEVERITY_WARNING = 0,
    GHOSTOS_SEVERITY_SUCCESS = 1,
    GHOSTOS_SEVERITY_ERROR = 2,
    GHOSTOS_SEVERITY_INFORMATION = 3,
    GHOSTOS_SEVERITY_FATAL = 4
};

#define GHOSTOS_STATUS_BITS(severity, facility, code, flags) \
    ((uint32_t)(severity) | ((uint32_t)(code) << 3) | \
     ((uint32_t)(facility) << 16) | ((uint32_t)(flags) << 28))

#define GHOSTOS_FACILITY_SYSTEM 1u
#define GHOSTOS_FACILITY_KERNEL 2u
#define GHOSTOS_FACILITY_FILESYSTEM 3u
#define GHOSTOS_FACILITY_DRIVER 4u
#define GHOSTOS_FACILITY_COMMAND 5u
#define GHOSTOS_FACILITY_LOGICAL_NAME 6u
#define GHOSTOS_FACILITY_SECURITY 7u
#define GHOSTOS_FACILITY_DLM 8u
#define GHOSTOS_FACILITY_RMS 9u
#define GHOSTOS_FACILITY_FABRIC 10u
#define GHOSTOS_FACILITY_LLM 11u
#define GHOSTOS_FACILITY_SHELL 12u
#define GHOSTOS_FACILITY_NETWORK 13u
#define GHOSTOS_FACILITY_COMPUTE 14u
#define GHOSTOS_FACILITY_SCRIPT 15u

#define GHOSTOS_STATUS_NORMAL GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_SUCCESS, GHOSTOS_FACILITY_SYSTEM, 1u, 0u)
#define GHOSTOS_STATUS_PENDING GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_INFORMATION, GHOSTOS_FACILITY_SYSTEM, 2u, 0u)
#define GHOSTOS_STATUS_INVALID_ARGUMENT GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SYSTEM, 3u, 0u)
#define GHOSTOS_STATUS_NOT_FOUND GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SYSTEM, 4u, 0u)
#define GHOSTOS_STATUS_ACCESS_DENIED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SECURITY, 1u, 0u)
#define GHOSTOS_STATUS_NO_SPACE GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SYSTEM, 5u, 0u)
#define GHOSTOS_STATUS_CORRUPT GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_FATAL, GHOSTOS_FACILITY_SYSTEM, 6u, 0u)
#define GHOSTOS_STATUS_BUSY GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_SYSTEM, 7u, 0u)
#define GHOSTOS_STATUS_CANCELLED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_SYSTEM, 8u, 0u)
#define GHOSTOS_STATUS_INTERNAL GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SYSTEM, 9u, 0u)
#define GHOSTOS_STATUS_METHOD_NOT_ALLOWED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SYSTEM, 10u, 0u)
#define GHOSTOS_STATUS_REQUEST_TOO_LARGE GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_SYSTEM, 11u, 0u)
#define GHOSTOS_STATUS_ALREADY_EXISTS GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 2u, 0u)
#define GHOSTOS_STATUS_CONFLICT GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FILESYSTEM, 3u, 0u)
#define GHOSTOS_STATUS_DIRECTORY_NOT_EMPTY GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 5u, 0u)
#define GHOSTOS_STATUS_INVALID_PATH GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 6u, 0u)
#define GHOSTOS_STATUS_NOT_DIRECTORY GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 7u, 0u)
#define GHOSTOS_STATUS_READ_ONLY GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 8u, 0u)
#define GHOSTOS_STATUS_PARTIAL_MATCH GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FILESYSTEM, 9u, 0u)
#define GHOSTOS_STATUS_INVALID_PATTERN GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FILESYSTEM, 10u, 0u)
#define GHOSTOS_STATUS_QUORUM_LOST GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FABRIC, 1u, 0u)
#define GHOSTOS_STATUS_PARTITIONED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FABRIC, 2u, 0u)
#define GHOSTOS_STATUS_CLOCK_SKEW GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FABRIC, 3u, 0u)
#define GHOSTOS_STATUS_PROTOCOL_MISMATCH GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FABRIC, 4u, 0u)
#define GHOSTOS_STATUS_CLUSTER_DEGRADED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FABRIC, 11u, 0u)
#define GHOSTOS_STATUS_STALE_STATE GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FABRIC, 6u, 0u)
#define GHOSTOS_STATUS_NODE_UNSAFE GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FABRIC, 7u, 0u)
#define GHOSTOS_STATUS_CONFIRMATION_REQUIRED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FABRIC, 8u, 0u)
#define GHOSTOS_STATUS_RECONCILIATION_REQUIRED GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_WARNING, GHOSTOS_FACILITY_FABRIC, 9u, 0u)
#define GHOSTOS_STATUS_ROLLBACK_UNAVAILABLE GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FABRIC, 10u, 0u)
#define GHOSTOS_STATUS_RECOVERY_STATE_INVALID GHOSTOS_STATUS_BITS(GHOSTOS_SEVERITY_ERROR, GHOSTOS_FACILITY_FABRIC, 12u, 0u)

#define GHOSTOS_OPERATION_HTTP_PARSE 1u
#define GHOSTOS_OPERATION_HTTP_ROUTE 2u
#define GHOSTOS_OPERATION_HTTP_RPC 3u
#define GHOSTOS_OPERATION_HTTP_SERVER 4u
#define GHOSTOS_OPERATION_GRPC 5u
#define GHOSTOS_OPERATION_FRONTEND_RPC 6u

/* C ports use explicit tags instead of Rust enum layout. These are not wire structs. */
typedef enum {
    GHOSTOS_RETRY_NEVER,
    GHOSTOS_RETRY_IMMEDIATE,
    GHOSTOS_RETRY_AFTER_US
} ghostos_retry_kind;

typedef struct {
    ghostos_retry_kind kind;
    uint64_t delay_us;
} ghostos_retry_hint;

typedef struct {
    uint64_t correlation_low;
    uint64_t correlation_high;
    uint32_t node;
} ghostos_audit_context;

typedef struct {
    ghostos_status code;
    uint16_t operation;
    ghostos_retry_hint retry;
    ghostos_audit_context audit;
} ghostos_public_error;

bool ghostos_severity_from_raw(uint8_t raw, ghostos_severity *out);
bool ghostos_status_new(ghostos_severity severity, uint16_t facility, uint16_t code,
                        uint8_t flags, ghostos_status *out);
bool ghostos_status_from_raw(uint32_t raw, ghostos_status *out);
ghostos_severity ghostos_status_severity(ghostos_status status);
uint16_t ghostos_status_facility(ghostos_status status);
uint16_t ghostos_status_code(ghostos_status status);
uint8_t ghostos_status_flags(ghostos_status status);
bool ghostos_status_is_success(ghostos_status status);
const char *ghostos_status_message(ghostos_status status);
const char *ghostos_status_operator_action(ghostos_status status);
const char *ghostos_status_operator_impact(ghostos_status status);
ghostos_retry_hint ghostos_status_retry_hint(ghostos_status status);
bool ghostos_retry_is_retryable(ghostos_retry_hint retry);
const char *ghostos_retry_label(ghostos_retry_hint retry);
const char *ghostos_retry_safety(ghostos_retry_hint retry);
ghostos_audit_context ghostos_audit_context_new(uint64_t correlation_low,
                                              uint64_t correlation_high, uint32_t node);
ghostos_public_error ghostos_public_error_new(ghostos_status code, uint16_t operation,
                                             ghostos_retry_hint retry, ghostos_audit_context audit);
ghostos_public_error ghostos_status_public_error(ghostos_status code, uint16_t operation,
                                                ghostos_audit_context audit);

#endif
