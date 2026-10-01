#include "ghostos/status.h"
#include <stddef.h>

bool ghostos_severity_from_raw(uint8_t raw, ghostos_severity *out) {
    if (raw > GHOSTOS_SEVERITY_FATAL || out == NULL) return false;
    *out = raw;
    return true;
}

bool ghostos_status_new(ghostos_severity severity, uint16_t facility, uint16_t code,
                        uint8_t flags, ghostos_status *out) {
    if (severity > GHOSTOS_SEVERITY_FATAL || facility > 0xfff || code > 0x1fff ||
        flags > 0xf || out == NULL) return false;
    *out = GHOSTOS_STATUS_BITS(severity, facility, code, flags);
    return true;
}

bool ghostos_status_from_raw(uint32_t raw, ghostos_status *out) {
    if ((raw & 7u) > GHOSTOS_SEVERITY_FATAL || out == NULL) return false;
    *out = raw;
    return true;
}

ghostos_severity ghostos_status_severity(ghostos_status status) { return status & 7u; }
uint16_t ghostos_status_facility(ghostos_status status) { return (status >> 16) & 0xfffu; }
uint16_t ghostos_status_code(ghostos_status status) { return (status >> 3) & 0x1fffu; }
uint8_t ghostos_status_flags(ghostos_status status) { return (status >> 28) & 0xfu; }
bool ghostos_status_is_success(ghostos_status status) { return (status & 1u) != 0; }

const char *ghostos_status_message(ghostos_status status) {
    /* Severity and flags deliberately do not affect the message lookup. */
    switch (GHOSTOS_STATUS_BITS(0, ghostos_status_facility(status), ghostos_status_code(status), 0)) {
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 1, 0): return "normal";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 2, 0): return "pending";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 3, 0): return "invalid argument";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 4, 0): return "not found";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 5, 0): return "no space";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 6, 0): return "corrupt";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 7, 0): return "busy";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 8, 0): return "cancelled";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 9, 0): return "internal error";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 10, 0): return "method not allowed";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SYSTEM, 11, 0): return "request too large";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_SECURITY, 1, 0): return "access denied";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 2, 0): return "already exists";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 3, 0): return "conflict";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 5, 0): return "directory not empty";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 6, 0): return "invalid path";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 7, 0): return "not a directory";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 8, 0): return "read-only mount";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 9, 0): return "partial wildcard match";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FILESYSTEM, 10, 0): return "malformed wildcard pattern";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 1, 0): return "quorum lost";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 2, 0): return "cluster partitioned";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 3, 0): return "clock skew";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 4, 0): return "protocol mismatch";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 11, 0): return "cluster degraded";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 6, 0): return "stale state";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 7, 0): return "node unsafe";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 8, 0): return "confirmation required";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 9, 0): return "reconciliation required";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 10, 0): return "rollback unavailable";
        case GHOSTOS_STATUS_BITS(0, GHOSTOS_FACILITY_FABRIC, 12, 0): return "recovery state invalid";
        default: return "unknown status";
    }
}

const char *ghostos_status_operator_action(ghostos_status status) {
    switch (status) {
        case GHOSTOS_STATUS_INVALID_ARGUMENT:
        case GHOSTOS_STATUS_INVALID_PATH:
        case GHOSTOS_STATUS_INVALID_PATTERN:
            return "Fix the request, then submit it again.";
        case GHOSTOS_STATUS_ACCESS_DENIED:
            return "Use an authorized identity or ask an administrator.";
        case GHOSTOS_STATUS_NOT_FOUND:
            return "Check the resource name and current cluster state.";
        case GHOSTOS_STATUS_NO_SPACE:
            return "Free capacity or wait for recovery capacity, then retry.";
        case GHOSTOS_STATUS_CORRUPT:
            return "Stop using the affected data and follow the recovery runbook.";
        case GHOSTOS_STATUS_BUSY:
        case GHOSTOS_STATUS_PENDING:
            return "Wait for the current operation to finish, then retry.";
        case GHOSTOS_STATUS_CANCELLED:
            return "Review the incomplete work before starting it again.";
        case GHOSTOS_STATUS_CONFLICT:
        case GHOSTOS_STATUS_STALE_STATE:
            return "Refresh state and repeat the operation once.";
        case GHOSTOS_STATUS_CONFIRMATION_REQUIRED:
            return "Review the requested change and confirm it explicitly.";
        case GHOSTOS_STATUS_RECONCILIATION_REQUIRED:
            return "Reconcile the reported state before changing it.";
        case GHOSTOS_STATUS_ROLLBACK_UNAVAILABLE:
        case GHOSTOS_STATUS_RECOVERY_STATE_INVALID:
            return "Stop the rollout and follow the recovery runbook.";
        default: return "Inspect the audit record before repeating the operation.";
    }
}

const char *ghostos_status_operator_impact(ghostos_status status) {
    switch (status) {
        case GHOSTOS_STATUS_INVALID_ARGUMENT:
        case GHOSTOS_STATUS_INVALID_PATH:
        case GHOSTOS_STATUS_INVALID_PATTERN:
            return "Nothing was changed.";
        case GHOSTOS_STATUS_ACCESS_DENIED:
            return "Nothing was changed because authorization failed.";
        case GHOSTOS_STATUS_NOT_FOUND:
            return "The requested resource was not found; no change was made.";
        case GHOSTOS_STATUS_NO_SPACE:
            return "The operation did not complete because capacity is exhausted.";
        case GHOSTOS_STATUS_CORRUPT:
            return "Affected data may be unsafe until recovery completes.";
        case GHOSTOS_STATUS_BUSY:
        case GHOSTOS_STATUS_PENDING:
            return "The operation is not complete yet.";
        case GHOSTOS_STATUS_CANCELLED:
            return "The operation stopped before all work completed.";
        case GHOSTOS_STATUS_CONFLICT:
        case GHOSTOS_STATUS_STALE_STATE:
            return "The requested change was not committed.";
        case GHOSTOS_STATUS_CONFIRMATION_REQUIRED:
            return "The change is waiting for explicit approval.";
        case GHOSTOS_STATUS_RECONCILIATION_REQUIRED:
            return "Live state may differ from the requested state.";
        case GHOSTOS_STATUS_ROLLBACK_UNAVAILABLE:
        case GHOSTOS_STATUS_RECOVERY_STATE_INVALID:
            return "The requested recovery state is not safe to activate.";
        default: return "The operation failed; inspect the audit record for exact scope.";
    }
}

ghostos_retry_hint ghostos_status_retry_hint(ghostos_status status) {
    switch (status) {
        case GHOSTOS_STATUS_BUSY:
        case GHOSTOS_STATUS_NO_SPACE:
        case GHOSTOS_STATUS_INTERNAL:
            return (ghostos_retry_hint){GHOSTOS_RETRY_AFTER_US, UINT64_C(1000000)};
        default: return (ghostos_retry_hint){GHOSTOS_RETRY_NEVER, 0};
    }
}

bool ghostos_retry_is_retryable(ghostos_retry_hint retry) { return retry.kind != GHOSTOS_RETRY_NEVER; }
const char *ghostos_retry_label(ghostos_retry_hint retry) {
    switch (retry.kind) {
        case GHOSTOS_RETRY_IMMEDIATE: return "immediate";
        case GHOSTOS_RETRY_AFTER_US: return "after_us";
        default: return "never";
    }
}
const char *ghostos_retry_safety(ghostos_retry_hint retry) {
    switch (retry.kind) {
        case GHOSTOS_RETRY_IMMEDIATE: return "Safe to retry now.";
        case GHOSTOS_RETRY_AFTER_US: return "Retry only after the stated delay.";
        default: return "Do not retry automatically.";
    }
}
ghostos_audit_context ghostos_audit_context_new(uint64_t low, uint64_t high, uint32_t node) {
    return (ghostos_audit_context){low, high, node};
}
ghostos_public_error ghostos_public_error_new(ghostos_status code, uint16_t operation,
                                             ghostos_retry_hint retry, ghostos_audit_context audit) {
    return (ghostos_public_error){code, operation, retry, audit};
}
ghostos_public_error ghostos_status_public_error(ghostos_status code, uint16_t operation,
                                                ghostos_audit_context audit) {
    return ghostos_public_error_new(code, operation, ghostos_status_retry_hint(code), audit);
}
