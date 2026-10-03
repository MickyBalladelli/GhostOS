#include "ghostos/api_compat.h"

#define CONTRACT(kind, title) {kind, title, {1, 0}, {{1, 0}, {1, 0}}, false, {0}}
const ghostos_api_contract ghostos_all_contracts[8] = {
    CONTRACT(GHOSTOS_API_RUST, "public Rust API"),
    CONTRACT(GHOSTOS_API_SWIFT, "public Swift API"),
    CONTRACT(GHOSTOS_API_WIRE, "wire API"),
    CONTRACT(GHOSTOS_API_SHELL, "shell API"),
    CONTRACT(GHOSTOS_API_PACKAGE, "package API"),
    {GHOSTOS_API_SNAPSHOT, "snapshot API", {2, 0}, {{1, 0}, {2, 0}}, true,
     {"snapshot-v1-to-v2", {1, 0}, {2, 0}, "ghostos-vm snapshot convert --from 1 --to 2 input.vm output.vm"}},
    CONTRACT(GHOSTOS_API_CONFIGURATION, "configuration API"),
    CONTRACT(GHOSTOS_API_SDK, "user-space SDK")
};
#undef CONTRACT

bool ghostos_api_version_is_zero(ghostos_api_version version) { return version.major == 0 && version.minor == 0; }
bool ghostos_api_version_is_before(ghostos_api_version version, ghostos_api_version other) {
    return version.major < other.major || (version.major == other.major && version.minor < other.minor);
}
bool ghostos_api_version_is_after(ghostos_api_version version, ghostos_api_version other) {
    return ghostos_api_version_is_before(other, version);
}
const char *ghostos_api_kind_name(ghostos_api_kind kind) {
    switch (kind) {
        case GHOSTOS_API_RUST: return "rust";
        case GHOSTOS_API_SWIFT: return "swift";
        case GHOSTOS_API_WIRE: return "wire";
        case GHOSTOS_API_SHELL: return "shell";
        case GHOSTOS_API_PACKAGE: return "package";
        case GHOSTOS_API_SNAPSHOT: return "snapshot";
        case GHOSTOS_API_CONFIGURATION: return "configuration";
        case GHOSTOS_API_SDK: return "user-space SDK";
        default: return "unknown";
    }
}
bool ghostos_api_range_contains(ghostos_api_version_range range, ghostos_api_version version) {
    return !ghostos_api_version_is_before(version, range.minimum) && !ghostos_api_version_is_after(version, range.maximum);
}
bool ghostos_api_range_is_valid(ghostos_api_version_range range) {
    return !ghostos_api_version_is_zero(range.minimum) && !ghostos_api_version_is_after(range.minimum, range.maximum);
}
bool ghostos_api_contract_get(ghostos_api_kind kind, ghostos_api_contract *out) {
    if (kind < GHOSTOS_API_RUST || kind > GHOSTOS_API_SDK || out == NULL) return false;
    *out = ghostos_all_contracts[kind - 1];
    return true;
}
bool ghostos_api_contract_accepts(ghostos_api_contract contract, ghostos_api_version offered) {
    return ghostos_api_range_contains(contract.supported, offered);
}
static ghostos_compatibility_error compatibility_error(ghostos_api_contract contract,
                                                      ghostos_api_version offered, const char *code) {
    return (ghostos_compatibility_error){contract.kind, code, offered, contract.supported};
}
ghostos_compatibility ghostos_api_contract_check(ghostos_api_contract contract, ghostos_api_version offered) {
    ghostos_compatibility result = {0};
    const char *code = NULL;
    if (!ghostos_api_range_is_valid(contract.supported)) code = GHOSTOS_COMPAT_INVALID_RANGE;
    else if (ghostos_api_version_is_before(offered, contract.supported.minimum)) code = GHOSTOS_COMPAT_TOO_OLD;
    else if (ghostos_api_version_is_after(offered, contract.supported.maximum)) code = GHOSTOS_COMPAT_TOO_NEW;
    if (code != NULL) {
        result.kind = GHOSTOS_COMPAT_REJECTED;
        result.error = compatibility_error(contract, offered, code);
    } else if (ghostos_api_version_is_before(offered, contract.current)) {
        result.kind = GHOSTOS_COMPAT_DEPRECATED;
        result.warning = (ghostos_deprecation_warning){GHOSTOS_COMPAT_LEGACY, contract.kind, offered, contract.current};
    } else result.kind = GHOSTOS_COMPAT_ACCEPTED;
    return result;
}
static bool version_equal(ghostos_api_version a, ghostos_api_version b) {
    return a.major == b.major && a.minor == b.minor;
}
bool ghostos_api_migrate_to_current(ghostos_api_contract contract, ghostos_api_version offered,
                                   ghostos_api_migration *out, ghostos_compatibility_error *error) {
    if (contract.has_migration && version_equal(contract.migration.from, offered) &&
        version_equal(contract.migration.to, contract.current) && out != NULL) {
        *out = contract.migration;
        return true;
    }
    if (error != NULL) *error = compatibility_error(contract, offered, GHOSTOS_COMPAT_MIGRATION_REQUIRED);
    return false;
}

_Static_assert(sizeof(ghostos_api_version) == 4, "API version size");
_Static_assert(offsetof(ghostos_api_version, minor) == 2, "API minor offset");
_Static_assert(sizeof(ghostos_api_version_range) == 8, "API range size");
_Static_assert(offsetof(ghostos_api_version_range, maximum) == 4, "API maximum offset");

static bool code_equal(const char *a, const char *b) {
    while (*a != '\0' && *a == *b) { ++a; ++b; }
    return *a == *b;
}

uint32_t ghostos_api_check_policy(ghostos_api_version current,
    ghostos_api_version_range supported, ghostos_api_version offered) {
    ghostos_api_contract contract = {.current = current, .supported = supported};
    ghostos_compatibility result = ghostos_api_contract_check(contract, offered);
    if (result.kind == GHOSTOS_COMPAT_ACCEPTED) return GHOSTOS_POLICY_ACCEPTED;
    if (result.kind == GHOSTOS_COMPAT_DEPRECATED) return GHOSTOS_POLICY_DEPRECATED;
    if (code_equal(result.error.code, GHOSTOS_COMPAT_INVALID_RANGE)) return GHOSTOS_POLICY_INVALID_RANGE;
    if (code_equal(result.error.code, GHOSTOS_COMPAT_TOO_OLD)) return GHOSTOS_POLICY_TOO_OLD;
    return GHOSTOS_POLICY_TOO_NEW;
}

bool ghostos_api_migration_policy(ghostos_api_version current,
    ghostos_api_version offered, bool has_migration,
    ghostos_api_version from, ghostos_api_version to) {
    ghostos_api_contract contract = {.current = current, .has_migration = has_migration,
        .migration = {.from = from, .to = to}};
    ghostos_api_migration migration;
    return ghostos_api_migrate_to_current(contract, offered, &migration, NULL);
}

/* Freestanding formatting avoids making the kernel port depend on stdio. */
typedef struct { char *out; size_t capacity, length; } writer;
static void put(writer *w, char value) {
    if (w->out != NULL && w->capacity > 0 && w->length < w->capacity - 1) w->out[w->length] = value;
    ++w->length;
}
static void string(writer *w, const char *value) { while (*value != '\0') put(w, *value++); }
static void number(writer *w, uint16_t value) {
    char digits[5];
    size_t count = 0;
    do { digits[count++] = (char)('0' + value % 10); value /= 10; } while (value != 0);
    while (count != 0) put(w, digits[--count]);
}
static void version(writer *w, ghostos_api_version value) { number(w, value.major); put(w, '.'); number(w, value.minor); }
static size_t finish(writer *w) {
    if (w->out != NULL && w->capacity > 0) w->out[w->length < w->capacity ? w->length : w->capacity - 1] = '\0';
    return w->length;
}
size_t ghostos_api_version_format(ghostos_api_version value, char *out, size_t capacity) {
    writer w = {out, capacity, 0};
    version(&w, value);
    return finish(&w);
}
size_t ghostos_compatibility_error_format(ghostos_compatibility_error error, char *out, size_t capacity) {
    writer w = {out, capacity, 0};
    string(&w, error.code);
    string(&w, ": ");
    string(&w, ghostos_api_kind_name(error.kind));
    string(&w, " API version ");
    version(&w, error.offered);
    string(&w, " is outside supported range ");
    version(&w, error.supported.minimum);
    string(&w, "..=");
    version(&w, error.supported.maximum);
    return finish(&w);
}
