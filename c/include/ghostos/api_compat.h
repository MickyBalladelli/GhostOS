#ifndef GHOSTOS_API_COMPAT_H
#define GHOSTOS_API_COMPAT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_COMPAT_TOO_OLD "GHOSTOS-COMPAT-001"
#define GHOSTOS_COMPAT_TOO_NEW "GHOSTOS-COMPAT-002"
#define GHOSTOS_COMPAT_INVALID_RANGE "GHOSTOS-COMPAT-003"
#define GHOSTOS_COMPAT_MIGRATION_REQUIRED "GHOSTOS-COMPAT-004"
#define GHOSTOS_COMPAT_LEGACY "GHOSTOS-COMPAT-DEP-001"

typedef struct { uint16_t major, minor; } ghostos_api_version;
#define GHOSTOS_API_V1 ((ghostos_api_version){1, 0})
#define GHOSTOS_API_V2 ((ghostos_api_version){2, 0})
typedef uint8_t ghostos_api_kind;
enum {
    GHOSTOS_API_RUST = 1, GHOSTOS_API_SWIFT, GHOSTOS_API_WIRE, GHOSTOS_API_SHELL,
    GHOSTOS_API_PACKAGE, GHOSTOS_API_SNAPSHOT, GHOSTOS_API_CONFIGURATION, GHOSTOS_API_SDK
};
typedef struct { ghostos_api_version minimum, maximum; } ghostos_api_version_range;
typedef struct {
    const char *id;
    ghostos_api_version from, to;
    const char *example;
} ghostos_api_migration;
typedef struct {
    ghostos_api_kind kind;
    const char *name;
    ghostos_api_version current;
    ghostos_api_version_range supported;
    bool has_migration;
    ghostos_api_migration migration;
} ghostos_api_contract;
typedef struct {
    const char *code;
    ghostos_api_kind kind;
    ghostos_api_version version, replacement;
} ghostos_deprecation_warning;
typedef struct {
    ghostos_api_kind kind;
    const char *code;
    ghostos_api_version offered;
    ghostos_api_version_range supported;
} ghostos_compatibility_error;
typedef enum { GHOSTOS_COMPAT_ACCEPTED, GHOSTOS_COMPAT_DEPRECATED, GHOSTOS_COMPAT_REJECTED } ghostos_compatibility_kind;
typedef struct {
    ghostos_compatibility_kind kind;
    ghostos_deprecation_warning warning;
    ghostos_compatibility_error error;
} ghostos_compatibility;

extern const ghostos_api_contract ghostos_all_contracts[8];
bool ghostos_api_version_is_zero(ghostos_api_version version);
bool ghostos_api_version_is_before(ghostos_api_version version, ghostos_api_version other);
bool ghostos_api_version_is_after(ghostos_api_version version, ghostos_api_version other);
const char *ghostos_api_kind_name(ghostos_api_kind kind);
bool ghostos_api_range_contains(ghostos_api_version_range range, ghostos_api_version version);
bool ghostos_api_range_is_valid(ghostos_api_version_range range);
bool ghostos_api_contract_get(ghostos_api_kind kind, ghostos_api_contract *out);
bool ghostos_api_contract_accepts(ghostos_api_contract contract, ghostos_api_version offered);
ghostos_compatibility ghostos_api_contract_check(ghostos_api_contract contract, ghostos_api_version offered);
bool ghostos_api_migrate_to_current(ghostos_api_contract contract, ghostos_api_version offered,
                                   ghostos_api_migration *out, ghostos_compatibility_error *error);
/* Allocation-free policy bridge for typed host adapters. String metadata stays
 * with the caller; the C contract policy determines these stable result tags. */
enum ghostos_compat_policy_result {
    GHOSTOS_POLICY_ACCEPTED, GHOSTOS_POLICY_DEPRECATED,
    GHOSTOS_POLICY_INVALID_RANGE, GHOSTOS_POLICY_TOO_OLD, GHOSTOS_POLICY_TOO_NEW
};
uint32_t ghostos_api_check_policy(ghostos_api_version current,
    ghostos_api_version_range supported, ghostos_api_version offered);
bool ghostos_api_migration_policy(ghostos_api_version current,
    ghostos_api_version offered, bool has_migration,
    ghostos_api_version from, ghostos_api_version to);
/* Formatting returns required characters excluding NUL; truncated output is NUL terminated. */
size_t ghostos_api_version_format(ghostos_api_version version, char *out, size_t capacity);
size_t ghostos_compatibility_error_format(ghostos_compatibility_error error, char *out, size_t capacity);
#endif
