#include "ghostos/boot_services.h"
#include "ghostos/abi.h"

#define SPEC(id_, process_, image_low_, image_high_, profile_, name_, kind_, dep_count_, dep0_, dep1_, dep2_, dep3_, dep4_) \
    { (id_), (process_), (image_low_), (image_high_), (profile_), (name_), (kind_), \
      3, UINT64_C(60000000), UINT64_C(100000), UINT64_C(5000000), (dep_count_), \
      { (dep0_), (dep1_), (dep2_), (dep3_), (dep4_) } }

enum {
    SERVICE_FILESYSTEM = 0x46534444u,
    SERVICE_STORAGE = 0x53544f52u,
    SERVICE_NETWORK = 0x4e455444u,
    SERVICE_LOGGING = 0x4c4f4744u,
    SERVICE_AUDIT = 0x41554454u,
    SERVICE_AUTHENTICATION = 0x41555448u,
    SERVICE_PACKAGE = 0x504b4744u,
    SERVICE_SHELL = 0x5348454cu,
    SERVICE_LOGIN = 0x4c4f4749u,
    SERVICE_PCI = 0x50434944u,
    SERVICE_AHCI = 0x41484349u,
    SERVICE_NVME = 0x4e564d45u,
    SERVICE_ETHERNET = 0x45544844u
};

static const ghostos_boot_service_spec services[GHOSTOS_BOOT_SERVICE_COUNT] = {
    SPEC(SERVICE_FILESYSTEM, 2, UINT64_C(1), UINT64_C(0x53594e4f53465344), UINT64_C(0x534653445f524f4f), "ghostos-fsd", GHOSTOS_SERVICE_SYSTEM, 1, SERVICE_STORAGE, 0, 0, 0, 0),
    SPEC(SERVICE_STORAGE, 3, UINT64_C(1), UINT64_C(0x53594e4f53544f52), UINT64_C(0x53544f525f524f4f), "ghostos-storaged", GHOSTOS_SERVICE_STORAGE_DRIVER, 1, SERVICE_PCI, 0, 0, 0, 0),
    SPEC(SERVICE_NETWORK, 4, UINT64_C(1), UINT64_C(0x53594e4f4e455444), UINT64_C(0x4e4554445f524f4f), "ghostos-netd", GHOSTOS_SERVICE_NETWORK_DRIVER, 1, SERVICE_ETHERNET, 0, 0, 0, 0),
    SPEC(SERVICE_LOGGING, 5, UINT64_C(1), UINT64_C(0x53594e4f4c4f4744), UINT64_C(0x4c4f47445f524f4f), "ghostos-logd", GHOSTOS_SERVICE_SYSTEM, 0, 0, 0, 0, 0, 0),
    SPEC(SERVICE_AUDIT, 6, UINT64_C(1), UINT64_C(0x53594e4f41554454), UINT64_C(0x415544545f524f4f), "ghostos-auditd", GHOSTOS_SERVICE_SYSTEM, 1, SERVICE_LOGGING, 0, 0, 0, 0),
    SPEC(SERVICE_AUTHENTICATION, 7, UINT64_C(0x4800000000000001), UINT64_C(0x53594e4f53415554), UINT64_C(0x415554485f524f4f), "ghostos-authd", GHOSTOS_SERVICE_SYSTEM, 2, SERVICE_FILESYSTEM, SERVICE_AUDIT, 0, 0, 0),
    SPEC(SERVICE_PACKAGE, 8, UINT64_C(0x4400000000000001), UINT64_C(0x53594e4f53504b47), UINT64_C(0x504b47445f524f4f), "ghostos-pkgd", GHOSTOS_SERVICE_SYSTEM, 2, SERVICE_FILESYSTEM, SERVICE_LOGGING, 0, 0, 0),
    SPEC(SERVICE_SHELL, 9, UINT64_C(0x4c4c000000000001), UINT64_C(0x53594e4f53534845), UINT64_C(0x5348454c4c5f524f), "ghostos-shell", GHOSTOS_SERVICE_SYSTEM, 5, SERVICE_AUTHENTICATION, SERVICE_LOGIN, SERVICE_PACKAGE, SERVICE_NETWORK, SERVICE_LOGGING),
    SPEC(SERVICE_LOGIN, 14, UINT64_C(0x4e00000000000001), UINT64_C(0x53594e4f4c4f4749), UINT64_C(0x4c4f47494e5f524f), "ghostos-logind", GHOSTOS_SERVICE_SYSTEM, 1, SERVICE_AUTHENTICATION, 0, 0, 0, 0),
    SPEC(SERVICE_PCI, 10, UINT64_C(1), UINT64_C(0x53594e4f50434944), UINT64_C(0x504349445f524f4f), "ghostos-pcid", GHOSTOS_SERVICE_SYSTEM, 0, 0, 0, 0, 0, 0),
    SPEC(SERVICE_AHCI, 11, UINT64_C(1), UINT64_C(0x53594e4f41484349), UINT64_C(0x414843495f524f4f), "ghostos-ahcid", GHOSTOS_SERVICE_STORAGE_DRIVER, 1, SERVICE_PCI, 0, 0, 0, 0),
    SPEC(SERVICE_NVME, 12, UINT64_C(1), UINT64_C(0x53594e4f4e564d45), UINT64_C(0x4e564d455f524f4f), "ghostos-nvmed", GHOSTOS_SERVICE_STORAGE_DRIVER, 1, SERVICE_PCI, 0, 0, 0, 0),
    SPEC(SERVICE_ETHERNET, 13, UINT64_C(1), UINT64_C(0x53594e4f45544844), UINT64_C(0x455448445f524f4f), "ghostos-ethernetd", GHOSTOS_SERVICE_NETWORK_DRIVER, 1, SERVICE_PCI, 0, 0, 0, 0)
};

const ghostos_boot_service_spec *ghostos_boot_service_specs(size_t *count)
{
    if (count) *count = GHOSTOS_BOOT_SERVICE_COUNT;
    return services;
}

const ghostos_boot_service_spec *ghostos_boot_service_find(uint32_t service_id)
{
    for (size_t i = 0; i < GHOSTOS_BOOT_SERVICE_COUNT; ++i)
        if (services[i].id == service_id) return &services[i];
    return NULL;
}

static size_t index_for_id(uint32_t service_id)
{
    for (size_t i = 0; i < GHOSTOS_BOOT_SERVICE_COUNT; ++i)
        if (services[i].id == service_id) return i;
    return GHOSTOS_BOOT_SERVICE_COUNT;
}

bool ghostos_boot_service_startup_order(uint32_t *service_ids, size_t capacity,
                                        size_t *count)
{
    bool remaining[GHOSTOS_BOOT_SERVICE_COUNT];
    size_t output[GHOSTOS_BOOT_SERVICE_COUNT];
    size_t length = 0;
    if (!service_ids || capacity < GHOSTOS_BOOT_SERVICE_COUNT) return false;
    for (size_t i = 0; i < GHOSTOS_BOOT_SERVICE_COUNT; ++i) remaining[i] = true;
    while (length < GHOSTOS_BOOT_SERVICE_COUNT) {
        size_t candidate = GHOSTOS_BOOT_SERVICE_COUNT;
        for (size_t i = 0; i < GHOSTOS_BOOT_SERVICE_COUNT; ++i) {
            bool ready = remaining[i];
            for (size_t d = 0; ready && d < services[i].dependency_count; ++d) {
                size_t dependency = index_for_id(services[i].dependencies[d]);
                if (dependency == GHOSTOS_BOOT_SERVICE_COUNT || remaining[dependency]) ready = false;
            }
            if (ready && (candidate == GHOSTOS_BOOT_SERVICE_COUNT || services[i].id < services[candidate].id))
                candidate = i;
        }
        if (candidate == GHOSTOS_BOOT_SERVICE_COUNT) return false;
        remaining[candidate] = false;
        output[length++] = candidate;
    }
    for (size_t i = 0; i < length; ++i) service_ids[i] = services[output[i]].id;
    if (count) *count = length;
    return true;
}

bool ghostos_boot_process_for_service(uint32_t service_id, uint64_t *process_id)
{
    const ghostos_boot_service_spec *service = ghostos_boot_service_find(service_id);
    if (!service || !process_id) return false;
    *process_id = service->process_id;
    return true;
}

bool ghostos_boot_services_start(ghostos_boot_service_spawn spawn, void *context,
                                 ghostos_boot_startup_diagnostic *diagnostics,
                                 size_t diagnostic_capacity)
{
    uint32_t order[GHOSTOS_BOOT_SERVICE_COUNT];
    size_t order_count = 0;
    bool started[GHOSTOS_BOOT_SERVICE_COUNT] = { false };
    if (!spawn || !diagnostics || diagnostic_capacity < GHOSTOS_BOOT_SERVICE_COUNT ||
        !ghostos_boot_service_startup_order(order, GHOSTOS_BOOT_SERVICE_COUNT,
                                            &order_count)) return false;
    for (size_t i = 0; i < GHOSTOS_BOOT_SERVICE_COUNT; ++i) {
        diagnostics[i] = (ghostos_boot_startup_diagnostic){
            services[i].id, services[i].process_id, 0,
            services[i].dependency_count, 0, false, false
        };
    }
    for (size_t position = 0; position < order_count; ++position) {
        const ghostos_boot_service_spec *service = ghostos_boot_service_find(order[position]);
        size_t service_index = index_for_id(order[position]);
        uint64_t process_id = 0;
        if (!service || service_index == GHOSTOS_BOOT_SERVICE_COUNT) return false;
        for (size_t d = 0; d < service->dependency_count; ++d) {
            size_t dependency_index = index_for_id(service->dependencies[d]);
            if (dependency_index == GHOSTOS_BOOT_SERVICE_COUNT || !started[dependency_index]) {
                diagnostics[service_index].blocked_on = service->dependencies[d];
                diagnostics[service_index].has_blocked_on = true;
                return false;
            }
        }
        if (!spawn(context, service, &process_id) || process_id == 0) return false;
        started[service_index] = true;
        diagnostics[service_index].process_id = process_id;
        diagnostics[service_index].startup_order = position + 1;
        diagnostics[service_index].ready = true;
    }
    return true;
}

uint32_t ghostos_boot_service_filesystem_rights(uint32_t service_id)
{
    if (service_id == SERVICE_FILESYSTEM)
        return GHOSTOS_FILESYSTEM_READ | GHOSTOS_FILESYSTEM_WRITE |
               GHOSTOS_FILESYSTEM_DELETE | GHOSTOS_FILESYSTEM_ADMIN;
    if (service_id == SERVICE_SHELL) return 0;
    if (service_id == SERVICE_AUTHENTICATION || service_id == SERVICE_LOGIN)
        return GHOSTOS_FILESYSTEM_READ;
    return 0;
}

bool ghostos_boot_first_admin_required(const void *authorization_database,
                                       size_t database_length)
{
    (void)database_length;
    return authorization_database == NULL;
}

static bool username_equal(const uint8_t *left, size_t left_length,
                           const uint8_t *right, size_t right_length)
{
    if (left_length != right_length) return false;
    for (size_t i = 0; i < left_length; ++i) {
        uint8_t a = left[i], b = right[i];
        if (a >= 'A' && a <= 'Z') a = (uint8_t)(a + ('a' - 'A'));
        if (b >= 'A' && b <= 'Z') b = (uint8_t)(b + ('a' - 'A'));
        if (a != b) return false;
    }
    return true;
}

static bool fingerprint_equal(const uint8_t left[32], const uint8_t right[32])
{
    uint8_t difference = 0;
    for (size_t i = 0; i < 32; ++i) difference |= (uint8_t)(left[i] ^ right[i]);
    return difference == 0;
}

bool ghostos_local_passkey_find(const ghostos_local_passkey_record *records,
                                size_t record_count, const uint8_t *username,
                                size_t username_length,
                                const uint8_t key_fingerprint[32],
                                uint32_t *sign_count)
{
    if (!records || !username || !key_fingerprint || !sign_count ||
        username_length == 0 || username_length > 32) return false;
    if (record_count > 128) return false;
    for (size_t i = 0; i < record_count; ++i) {
        const ghostos_local_passkey_record *record = &records[i];
        if (record->username_length == 0 || record->username_length > 32) return false;
        if (record->username_length == username_length &&
            username_equal(record->username, record->username_length, username, username_length) &&
            fingerprint_equal(record->key_fingerprint, key_fingerprint)) {
            *sign_count = record->sign_count;
            return true;
        }
    }
    *sign_count = 0;
    return true;
}

bool ghostos_local_passkey_record_use(ghostos_local_passkey_record *records,
                                     size_t capacity, size_t *record_count,
                                     const uint8_t *username,
                                     size_t username_length,
                                     const uint8_t key_fingerprint[32],
                                     uint32_t sign_count)
{
    if (!records || !record_count || !username || !key_fingerprint ||
        username_length == 0 || username_length > 32 || *record_count > capacity) return false;
    if (capacity > 128) return false;
    for (size_t i = 0; i < *record_count; ++i) {
        ghostos_local_passkey_record *record = &records[i];
        if (record->username_length == 0 || record->username_length > 32) return false;
        if (record->username_length == username_length &&
            username_equal(record->username, record->username_length, username, username_length) &&
            fingerprint_equal(record->key_fingerprint, key_fingerprint)) {
            if (sign_count < record->sign_count) return false;
            record->sign_count = sign_count;
            return true;
        }
    }
    if (*record_count == capacity) return false;
    ghostos_local_passkey_record *record = &records[(*record_count)++];
    for (size_t i = 0; i < sizeof(*record); ++i) ((uint8_t *)record)[i] = 0;
    record->username_length = (uint8_t)username_length;
    for (size_t i = 0; i < username_length; ++i) record->username[i] = username[i];
    for (size_t i = 0; i < 32; ++i) record->key_fingerprint[i] = key_fingerprint[i];
    record->sign_count = sign_count;
    return true;
}

bool ghostos_boot_shell_filesystem_dispatch(ghostos_shell_filesystem_dispatch dispatch,
                                            void *context, uint32_t operation,
                                            uint16_t flags, uint64_t capability,
                                            uint64_t offset, uint64_t length,
                                            void *buffer, uint64_t values[4])
{
    if (!dispatch || !values) return false;
    for (size_t i = 0; i < 4; ++i) values[i] = 0;
    return dispatch(context, 9, operation, flags, capability, offset, length,
                    buffer, values);
}

#define AUTH_DB_PATH "/system/security/authorization"
#define ADMIN_USERNAME_PATH "/system/security/first-admin-username"
#define ADMIN_CREDENTIAL_PATH "/system/security/first-admin-credential"
#define PASSKEY_COUNTERS_PATH "/system/security/passkey-counters"
#define ADMIN_CREDENTIAL_RECORD_BYTES (GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES + 2u)
#define ACCOUNT_RECORD_HEADER_BYTES 36u
#define PASSKEY_COUNTER_HEADER_BYTES 6u
#define PASSKEY_COUNTER_ENTRY_BYTES 37u

static bool filesystem_ops_valid(const ghostos_boot_filesystem_ops *ops)
{
    return ops && ops->lookup && ops->read && ops->transaction_begin &&
           ops->transaction_mkdir && ops->transaction_write &&
           ops->transaction_delete && ops->transaction_commit &&
           ops->transaction_abort && ops->sync;
}

bool ghostos_boot_state_adopt_filesystem(ghostos_boot_state *state,
                                         const ghostos_boot_filesystem_ops *ops,
                                         void *owned_filesystem_context)
{
    bool regular = false;
    size_t size = 0;
    ghostos_status status;
    if (!state || !filesystem_ops_valid(ops) || !owned_filesystem_context ||
        !ops->release_owned_filesystem) return false;
    *state = (ghostos_boot_state){0};
    state->filesystem = *ops;
    state->filesystem_context = owned_filesystem_context;
    state->owns_filesystem = true;
    status = ops->lookup(owned_filesystem_context, AUTH_DB_PATH, &regular, &size);
    state->provisioning_required = status == GHOSTOS_STATUS_NOT_FOUND;
    if (status == GHOSTOS_STATUS_NORMAL) {
        uint8_t database[GHOSTOS_BOOT_AUTH_DATABASE_BYTES];
        size_t database_length = 0;
        if (!regular || size == 0 || size > sizeof(database) ||
            ops->read(owned_filesystem_context, AUTH_DB_PATH, database,
                      sizeof(database), &database_length) != GHOSTOS_STATUS_NORMAL ||
            database_length != size) {
            state->owns_filesystem = false;
            state->filesystem_context = NULL;
            return false;
        }
    } else if (status != GHOSTOS_STATUS_NOT_FOUND) {
        state->owns_filesystem = false;
        state->filesystem_context = NULL;
        return false;
    }
    return true;
}

void ghostos_boot_state_release_filesystem(ghostos_boot_state *state)
{
    if (!state || !state->owns_filesystem) return;
    if (state->shell_filesystem_capability && state->filesystem.unregister_process)
        (void)state->filesystem.unregister_process(state->filesystem_context, 9);
    state->shell_filesystem_capability = 0;
    state->shell_filesystem_rights = 0;
    if (state->filesystem.release_owned_filesystem)
        state->filesystem.release_owned_filesystem(state->filesystem_context);
    state->filesystem_context = NULL;
    state->owns_filesystem = false;
}

typedef struct {
    ghostos_boot_state *state;
    ghostos_boot_service_launch launch;
    void *context;
} boot_launch_context;

static bool launch_adapter(void *opaque, const ghostos_boot_service_spec *service,
                           uint64_t *process_id)
{
    boot_launch_context *launch = opaque;
    if (!launch->launch(launch->context, service,
                        launch->state->filesystem_context, process_id)) return false;
    return *process_id == service->process_id;
}

bool ghostos_boot_kernel_launch(ghostos_boot_state *state,
                               ghostos_boot_service_launch launch, void *context,
                               ghostos_boot_startup_diagnostic *diagnostics,
                               size_t diagnostic_capacity)
{
    boot_launch_context adapter;
    if (!state || !launch || !state->owns_filesystem || !state->filesystem_context)
        return false;
    adapter = (boot_launch_context){ state, launch, context };
    return ghostos_boot_services_start(launch_adapter, &adapter, diagnostics,
                                       diagnostic_capacity);
}

static bool reserved_username(const uint8_t *username, size_t length)
{
    static const char *const reserved[] = {
        ".", "..", "account", "anonymous", "daemon", "guest", "kernel",
        "nobody", "operator", "root", "service", "system"
    };
    uint8_t normalized[GHOSTOS_BOOT_ADMIN_USERNAME_BYTES];
    for (size_t i = 0; i < length; ++i) {
        uint8_t ch = username[i];
        normalized[i] = ch >= 'A' && ch <= 'Z' ? (uint8_t)(ch + ('a' - 'A')) : ch;
    }
    for (size_t r = 0; r < sizeof(reserved) / sizeof(reserved[0]); ++r) {
        size_t reserved_length = 0;
        while (reserved[r][reserved_length]) ++reserved_length;
        if (reserved_length == length &&
            __builtin_memcmp(normalized, reserved[r], length) == 0) return true;
    }
    return false;
}

static void normalize_username(uint8_t *destination, const uint8_t *source,
                               size_t length)
{
    for (size_t i = 0; i < length; ++i) {
        uint8_t ch = source[i];
        destination[i] = ch >= 'A' && ch <= 'Z' ? (uint8_t)(ch + ('a' - 'A')) : ch;
    }
}

static ghostos_status lookup_path(ghostos_boot_state *state, const char *path,
                                  bool *regular, size_t *size)
{
    if (!state || !state->owns_filesystem || !state->filesystem_context)
        return GHOSTOS_STATUS_BUSY;
    *regular = false;
    *size = 0;
    return state->filesystem.lookup(state->filesystem_context, path, regular, size);
}

static ghostos_status read_regular(ghostos_boot_state *state, const char *path,
                                   uint8_t *bytes, size_t capacity, size_t *length)
{
    bool regular = false;
    size_t size = 0;
    ghostos_status status = lookup_path(state, path, &regular, &size);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    if (!regular || size > capacity) return GHOSTOS_STATUS_CORRUPT;
    status = state->filesystem.read(state->filesystem_context, path, bytes,
                                    capacity, length);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    return *length == size ? GHOSTOS_STATUS_NORMAL : GHOSTOS_STATUS_CORRUPT;
}

static ghostos_status transaction_write_one(ghostos_boot_state *state,
                                            const char *path, const void *bytes,
                                            size_t length)
{
    void *transaction = NULL;
    ghostos_status status = state->filesystem.transaction_begin(
        state->filesystem_context, &transaction);
    if (status != GHOSTOS_STATUS_NORMAL || !transaction) return status;
    status = state->filesystem.transaction_write(state->filesystem_context,
                                                 transaction, path, bytes, length);
    if (status != GHOSTOS_STATUS_NORMAL) {
        state->filesystem.transaction_abort(state->filesystem_context, transaction);
        return status;
    }
    status = state->filesystem.transaction_commit(state->filesystem_context, transaction);
    return status;
}

bool ghostos_boot_first_admin_record_valid(const uint8_t *username,
                                           size_t username_length,
                                           const uint8_t *public_key,
                                           size_t public_key_length)
{
    if (!username || !public_key || username_length == 0 || username_length > 32 ||
        public_key_length == 0 || public_key_length > 96 ||
        reserved_username(username, username_length)) return false;
    for (size_t i = 0; i < username_length; ++i) {
        uint8_t ch = username[i];
        if (!((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
              (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.' || ch == '$'))
            return false;
    }
    return true;
}

ghostos_status ghostos_boot_first_admin_set_username(ghostos_boot_state *state,
                                                     const uint8_t *username,
                                                     size_t username_length)
{
    bool regular = false;
    size_t size = 0;
    uint8_t normalized[GHOSTOS_BOOT_ADMIN_USERNAME_BYTES];
    uint8_t existing[GHOSTOS_BOOT_ADMIN_USERNAME_BYTES];
    size_t existing_length = 0;
    ghostos_status status;
    if (!state || !username || username_length == 0 || username_length > 32 ||
        reserved_username(username, username_length)) return GHOSTOS_STATUS_INVALID_ARGUMENT;
    for (size_t i = 0; i < username_length; ++i) {
        uint8_t ch = username[i];
        if (!((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
              (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.' || ch == '$'))
            return GHOSTOS_STATUS_INVALID_ARGUMENT;
    }
    if (!state->provisioning_required) return GHOSTOS_STATUS_ALREADY_EXISTS;
    status = lookup_path(state, AUTH_DB_PATH, &regular, &size);
    if (status == GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_ALREADY_EXISTS;
    if (status != GHOSTOS_STATUS_NOT_FOUND) return status;
    normalize_username(normalized, username, username_length);
    status = read_regular(state, ADMIN_USERNAME_PATH, existing, sizeof(existing),
                          &existing_length);
    if (status == GHOSTOS_STATUS_NORMAL) {
        if (existing_length == username_length &&
            __builtin_memcmp(existing, normalized, username_length) == 0) {
            state->first_admin_username_length = (uint8_t)username_length;
            __builtin_memcpy(state->first_admin_username, normalized, username_length);
            return GHOSTOS_STATUS_NORMAL;
        }
        return GHOSTOS_STATUS_ALREADY_EXISTS;
    }
    if (status != GHOSTOS_STATUS_NOT_FOUND) return status;
    void *transaction = NULL;
    status = state->filesystem.transaction_begin(state->filesystem_context, &transaction);
    if (status != GHOSTOS_STATUS_NORMAL || !transaction) return status;
    status = lookup_path(state, "/system/security", &regular, &size);
    if (status == GHOSTOS_STATUS_NOT_FOUND)
        status = state->filesystem.transaction_mkdir(state->filesystem_context,
                                                    transaction, "/system/security");
    else if (status == GHOSTOS_STATUS_NORMAL && regular)
        status = GHOSTOS_STATUS_INVALID_PATH;
    if (status == GHOSTOS_STATUS_NORMAL)
        status = state->filesystem.transaction_write(state->filesystem_context,
                                                     transaction, ADMIN_USERNAME_PATH,
                                                     normalized, username_length);
    if (status != GHOSTOS_STATUS_NORMAL) {
        state->filesystem.transaction_abort(state->filesystem_context, transaction);
        return status;
    }
    status = state->filesystem.transaction_commit(state->filesystem_context, transaction);
    if (status == GHOSTOS_STATUS_NORMAL) {
        state->first_admin_username_length = (uint8_t)username_length;
        __builtin_memcpy(state->first_admin_username, normalized, username_length);
    }
    return status;
}

ghostos_status ghostos_boot_first_admin_set_credential(ghostos_boot_state *state,
                                                       uint8_t kind,
                                                       const uint8_t *material,
                                                       size_t material_length)
{
    bool regular = false;
    size_t size = 0;
    uint8_t username[GHOSTOS_BOOT_ADMIN_USERNAME_BYTES];
    uint8_t existing[ADMIN_CREDENTIAL_RECORD_BYTES];
    uint8_t record[ADMIN_CREDENTIAL_RECORD_BYTES];
    size_t username_length = 0;
    size_t existing_length = 0;
    ghostos_status status;
    if (!state || !material || kind < 1 || kind > 3 || material_length == 0 ||
        material_length > GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES)
        return GHOSTOS_STATUS_INVALID_ARGUMENT;
    if (kind == 1) {
        if (!state->filesystem.validate_passkey) return GHOSTOS_STATUS_METHOD_NOT_ALLOWED;
        status = state->filesystem.validate_passkey(state->filesystem_context,
                                                    material, material_length);
        if (status != GHOSTOS_STATUS_NORMAL) return status;
    }
    if (!state->provisioning_required) return GHOSTOS_STATUS_ACCESS_DENIED;
    status = lookup_path(state, AUTH_DB_PATH, &regular, &size);
    if (status == GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_ALREADY_EXISTS;
    if (status != GHOSTOS_STATUS_NOT_FOUND) return status;
    status = read_regular(state, ADMIN_USERNAME_PATH, username, sizeof(username),
                          &username_length);
    if (status != GHOSTOS_STATUS_NORMAL) return status == GHOSTOS_STATUS_NOT_FOUND
        ? GHOSTOS_STATUS_ACCESS_DENIED : status;
    record[0] = kind;
    record[1] = (uint8_t)material_length;
    __builtin_memcpy(record + 2, material, material_length);
    status = read_regular(state, ADMIN_CREDENTIAL_PATH, existing, sizeof(existing),
                          &existing_length);
    if (status == GHOSTOS_STATUS_NORMAL) {
        if (existing_length < 3 || existing[0] < 1 || existing[0] > 3 ||
            existing[1] == 0 || existing[1] > 96 || existing_length != existing[1] + 2)
            return GHOSTOS_STATUS_CORRUPT;
        if (existing_length == material_length + 2 &&
            __builtin_memcmp(existing, record, existing_length) == 0)
            return GHOSTOS_STATUS_NORMAL;
    } else if (status != GHOSTOS_STATUS_NOT_FOUND) return status;
    return transaction_write_one(state, ADMIN_CREDENTIAL_PATH, record,
                                 material_length + 2);
}

ghostos_status ghostos_boot_first_admin_commit(ghostos_boot_state *state)
{
    uint8_t username[GHOSTOS_BOOT_ADMIN_USERNAME_BYTES];
    uint8_t credential[ADMIN_CREDENTIAL_RECORD_BYTES];
    uint8_t database[GHOSTOS_BOOT_AUTH_DATABASE_BYTES];
    size_t username_length = 0, credential_length = 0;
    bool regular = false;
    size_t size = 0;
    ghostos_status status;
    if (!state || !state->owns_filesystem) return GHOSTOS_STATUS_BUSY;
    if (!state->provisioning_required) return GHOSTOS_STATUS_ALREADY_EXISTS;
    if (state->first_admin_sync_pending) {
        status = state->filesystem.sync(state->filesystem_context);
        if (status != GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_INTERNAL;
        state->first_admin_sync_pending = false;
        state->first_admin_username_length = 0;
        state->provisioning_required = false;
        return GHOSTOS_STATUS_NORMAL;
    }
    status = lookup_path(state, AUTH_DB_PATH, &regular, &size);
    if (status == GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_ALREADY_EXISTS;
    if (status != GHOSTOS_STATUS_NOT_FOUND) return status;
    status = read_regular(state, ADMIN_USERNAME_PATH, username, sizeof(username),
                          &username_length);
    if (status == GHOSTOS_STATUS_NOT_FOUND) return GHOSTOS_STATUS_CONFIRMATION_REQUIRED;
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    status = read_regular(state, ADMIN_CREDENTIAL_PATH, credential, sizeof(credential),
                          &credential_length);
    if (status == GHOSTOS_STATUS_NOT_FOUND) return GHOSTOS_STATUS_CONFIRMATION_REQUIRED;
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    if (credential_length < 3 || credential_length != credential[1] + 2 ||
        credential[0] < 1 || credential[0] > 3 || credential[1] == 0 ||
        credential[1] > GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES)
        return GHOSTOS_STATUS_CORRUPT;
    if (!ghostos_boot_first_admin_record_valid(username, username_length,
                                              credential + 2, credential[1]))
        return GHOSTOS_STATUS_CORRUPT;
    if (credential[0] == 1) {
        if (!state->filesystem.validate_passkey) return GHOSTOS_STATUS_METHOD_NOT_ALLOWED;
        status = state->filesystem.validate_passkey(state->filesystem_context,
                                                    credential + 2, credential[1]);
        if (status != GHOSTOS_STATUS_NORMAL) return status;
    }
    __builtin_memset(database, 0, sizeof(database));
    database[0] = 1;
    database[1] = (uint8_t)username_length;
    normalize_username(database + 2, username, username_length);
    database[34] = credential[0];
    database[35] = credential[1];
    __builtin_memcpy(database + ACCOUNT_RECORD_HEADER_BYTES, credential + 2,
                     credential[1]);
    void *transaction = NULL;
    status = state->filesystem.transaction_begin(state->filesystem_context, &transaction);
    if (status != GHOSTOS_STATUS_NORMAL || !transaction) return status;
    status = state->filesystem.transaction_write(state->filesystem_context, transaction,
                                                 AUTH_DB_PATH, database,
                                                 ACCOUNT_RECORD_HEADER_BYTES + credential[1]);
    if (status == GHOSTOS_STATUS_NORMAL)
        status = state->filesystem.transaction_delete(state->filesystem_context,
                                                     transaction, ADMIN_USERNAME_PATH);
    if (status == GHOSTOS_STATUS_NORMAL)
        status = state->filesystem.transaction_delete(state->filesystem_context,
                                                     transaction, ADMIN_CREDENTIAL_PATH);
    if (status != GHOSTOS_STATUS_NORMAL) {
        state->filesystem.transaction_abort(state->filesystem_context, transaction);
        return status;
    }
    status = state->filesystem.transaction_commit(state->filesystem_context, transaction);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    status = state->filesystem.sync(state->filesystem_context);
    if (status != GHOSTOS_STATUS_NORMAL) {
        state->first_admin_sync_pending = true;
        return GHOSTOS_STATUS_INTERNAL;
    }
    state->first_admin_username_length = 0;
    state->provisioning_required = false;
    return GHOSTOS_STATUS_NORMAL;
}

ghostos_status ghostos_boot_first_admin_recovery(ghostos_boot_state *state,
                                                 uint64_t action,
                                                 uint64_t values[4])
{
    bool regular = false;
    size_t size = 0;
    if (!state || !values || !state->owns_filesystem) return GHOSTOS_STATUS_INVALID_ARGUMENT;
    __builtin_memset(values, 0, 4 * sizeof(*values));
    if (!state->provisioning_required) return GHOSTOS_STATUS_ALREADY_EXISTS;
    if (action == 1) {
        ghostos_status username = lookup_path(state, ADMIN_USERNAME_PATH, &regular, &size);
        values[0] = username == GHOSTOS_STATUS_NORMAL && regular && size != 0 ? 1 :
                    username == GHOSTOS_STATUS_NOT_FOUND ? 0 : 2;
        ghostos_status credential = lookup_path(state, ADMIN_CREDENTIAL_PATH, &regular, &size);
        values[1] = credential == GHOSTOS_STATUS_NORMAL && regular && size >= 3 ? 1 :
                    credential == GHOSTOS_STATUS_NOT_FOUND ? 0 : 2;
        values[2] = state->first_admin_sync_pending;
        values[3] = 1;
        return GHOSTOS_STATUS_NORMAL;
    }
    if (action == 3) {
        ghostos_status status = ghostos_boot_first_admin_commit(state);
        if (status == GHOSTOS_STATUS_NORMAL) values[0] = 1;
        return status;
    }
    if (action != 2) return GHOSTOS_STATUS_INVALID_ARGUMENT;
    if (state->recovery_sync_pending) {
        ghostos_status status = state->filesystem.sync(state->filesystem_context);
        if (status != GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_INTERNAL;
        state->recovery_sync_pending = false;
    }
    ghostos_status status = lookup_path(state, AUTH_DB_PATH, &regular, &size);
    if (status == GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_ALREADY_EXISTS;
    if (status != GHOSTOS_STATUS_NOT_FOUND) return status;
    void *transaction = NULL;
    status = state->filesystem.transaction_begin(state->filesystem_context, &transaction);
    if (status != GHOSTOS_STATUS_NORMAL || !transaction) return status;
    const char *paths[] = { ADMIN_USERNAME_PATH, ADMIN_CREDENTIAL_PATH };
    for (size_t i = 0; i < 2; ++i) {
        status = lookup_path(state, paths[i], &regular, &size);
        if (status == GHOSTOS_STATUS_NOT_FOUND) continue;
        if (status != GHOSTOS_STATUS_NORMAL || !regular) {
            state->filesystem.transaction_abort(state->filesystem_context, transaction);
            return status == GHOSTOS_STATUS_NORMAL ? GHOSTOS_STATUS_CORRUPT : status;
        }
        status = state->filesystem.transaction_delete(state->filesystem_context,
                                                     transaction, paths[i]);
        if (status != GHOSTOS_STATUS_NORMAL) {
            state->filesystem.transaction_abort(state->filesystem_context, transaction);
            return status;
        }
    }
    status = state->filesystem.transaction_commit(state->filesystem_context, transaction);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    status = state->filesystem.sync(state->filesystem_context);
    if (status != GHOSTOS_STATUS_NORMAL) {
        state->recovery_sync_pending = true;
        return GHOSTOS_STATUS_INTERNAL;
    }
    state->first_admin_username_length = 0;
    return GHOSTOS_STATUS_NORMAL;
}

static uint32_t read_be32(const uint8_t *bytes)
{
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
           ((uint32_t)bytes[2] << 8) | (uint32_t)bytes[3];
}

static void write_be32(uint8_t *bytes, uint32_t value)
{
    bytes[0] = (uint8_t)(value >> 24);
    bytes[1] = (uint8_t)(value >> 16);
    bytes[2] = (uint8_t)(value >> 8);
    bytes[3] = (uint8_t)value;
}

bool ghostos_boot_local_passkey_keys(ghostos_boot_state *state,
                                     const uint8_t *username,
                                     size_t username_length,
                                     uint8_t keys[GHOSTOS_BOOT_PASSKEY_MAX_KEYS]
                                                  [GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES],
                                     uint8_t lengths[GHOSTOS_BOOT_PASSKEY_MAX_KEYS],
                                     size_t *key_count)
{
    uint8_t database[GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES];
    size_t database_length = 0, offset = 0, latest = SIZE_MAX;
    ghostos_status status;
    if (!state || !username || !keys || !lengths || !key_count ||
        username_length == 0 || username_length > GHOSTOS_BOOT_ADMIN_USERNAME_BYTES)
        return false;
    *key_count = 0;
    status = read_regular(state, AUTH_DB_PATH, database, sizeof(database), &database_length);
    if (status != GHOSTOS_STATUS_NORMAL || database_length == 0) return false;
    while (offset < database_length) {
        if (database_length - offset < ACCOUNT_RECORD_HEADER_BYTES) return false;
        uint8_t *record = database + offset;
        size_t bytes = ACCOUNT_RECORD_HEADER_BYTES + record[35];
        size_t record_username_length = record[1];
        if (bytes > database_length - offset || (record[0] != 1 && record[0] != 2) ||
            record_username_length == 0 || record_username_length > 32) return false;
        if (username_equal(record + 2, record_username_length, username, username_length))
            latest = offset;
        offset += bytes;
    }
    if (latest == SIZE_MAX) return true;
    uint8_t *record = database + latest;
    uint8_t record_state = record[34];
    if (record_state < 1 || record_state > 3) return true;
    if (record[0] == 1) {
        size_t length = record[35];
        if (length > GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES || record_state != 1) return true;
        __builtin_memcpy(keys[0], record + ACCOUNT_RECORD_HEADER_BYTES, length);
        lengths[0] = (uint8_t)length;
        *key_count = 1;
        return true;
    }
    size_t count = record[ACCOUNT_RECORD_HEADER_BYTES];
    if (count == 0 || count > GHOSTOS_BOOT_PASSKEY_MAX_KEYS) return false;
    size_t cursor = ACCOUNT_RECORD_HEADER_BYTES + 1;
    size_t record_end = ACCOUNT_RECORD_HEADER_BYTES + record[35];
    for (size_t i = 0; i < count; ++i) {
        if (cursor + 3 > record_end) return false;
        uint8_t kind = record[cursor + 1];
        size_t length = record[cursor + 2];
        size_t material = cursor + 3;
        if (length == 0 || length > record_end - material) return false;
        if (kind == 1 && *key_count < GHOSTOS_BOOT_PASSKEY_MAX_KEYS &&
            length <= GHOSTOS_BOOT_ADMIN_CREDENTIAL_BYTES) {
            __builtin_memcpy(keys[*key_count], record + material, length);
            lengths[*key_count] = (uint8_t)length;
            ++*key_count;
        }
        cursor = material + length;
    }
    return cursor == record_end;
}

static ghostos_status load_passkey_counters(ghostos_boot_state *state,
                                            uint8_t database[GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES],
                                            size_t *database_length)
{
    bool regular = false;
    size_t file_size = 0;
    ghostos_status status = lookup_path(state, PASSKEY_COUNTERS_PATH, &regular, &file_size);
    if (status == GHOSTOS_STATUS_NOT_FOUND) {
        database[0] = 'S'; database[1] = 'Y'; database[2] = 'P'; database[3] = 'C';
        database[4] = 2; database[5] = 0;
        *database_length = PASSKEY_COUNTER_HEADER_BYTES;
        return GHOSTOS_STATUS_NORMAL;
    }
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    if (!regular || file_size < PASSKEY_COUNTER_HEADER_BYTES ||
        file_size > GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES) return GHOSTOS_STATUS_CORRUPT;
    status = state->filesystem.read(state->filesystem_context, PASSKEY_COUNTERS_PATH,
                                    database, GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES,
                                    database_length);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    if (*database_length != file_size || database[0] != 'S' || database[1] != 'Y' ||
        database[2] != 'P' || database[3] != 'C' || database[4] != 2)
        return GHOSTOS_STATUS_CORRUPT;
    size_t cursor = PASSKEY_COUNTER_HEADER_BYTES;
    for (size_t i = 0; i < database[5]; ++i) {
        if (cursor + PASSKEY_COUNTER_ENTRY_BYTES > file_size) return GHOSTOS_STATUS_CORRUPT;
        size_t name_length = database[cursor];
        if (name_length == 0 || name_length > 32 ||
            cursor + PASSKEY_COUNTER_ENTRY_BYTES + name_length > file_size)
            return GHOSTOS_STATUS_CORRUPT;
        cursor += PASSKEY_COUNTER_ENTRY_BYTES + name_length;
    }
    if (cursor != file_size) return GHOSTOS_STATUS_CORRUPT;
    return GHOSTOS_STATUS_NORMAL;
}

ghostos_status ghostos_boot_local_passkey_sign_count(ghostos_boot_state *state,
                                                      const uint8_t *username,
                                                      size_t username_length,
                                                      const uint8_t key_fingerprint[32],
                                                      uint32_t *sign_count)
{
    uint8_t database[GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES];
    size_t length = 0, offset = PASSKEY_COUNTER_HEADER_BYTES;
    ghostos_status status;
    if (!state || !username || !key_fingerprint || !sign_count ||
        username_length == 0 || username_length > 32) return GHOSTOS_STATUS_INVALID_ARGUMENT;
    status = load_passkey_counters(state, database, &length);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    for (size_t i = 0; i < database[5]; ++i) {
        size_t name_length = database[offset];
        if (name_length == 0 || name_length > 32 ||
            offset + PASSKEY_COUNTER_ENTRY_BYTES + name_length > length)
            return GHOSTOS_STATUS_CORRUPT;
        if (__builtin_memcmp(database + offset + 5, key_fingerprint, 32) == 0 &&
            username_equal(database + offset + 37, name_length, username, username_length)) {
            *sign_count = read_be32(database + offset + 1);
            return GHOSTOS_STATUS_NORMAL;
        }
        offset += PASSKEY_COUNTER_ENTRY_BYTES + name_length;
    }
    *sign_count = 0;
    return GHOSTOS_STATUS_NORMAL;
}

ghostos_status ghostos_boot_local_passkey_record_sign_count(
    ghostos_boot_state *state, const uint8_t *username, size_t username_length,
    const uint8_t key_fingerprint[32], uint32_t sign_count)
{
    uint8_t database[GHOSTOS_BOOT_PASSKEY_COUNTERS_BYTES];
    size_t length = 0, offset = PASSKEY_COUNTER_HEADER_BYTES;
    ghostos_status status;
    if (!state || !username || !key_fingerprint || username_length == 0 || username_length > 32)
        return GHOSTOS_STATUS_INVALID_ARGUMENT;
    status = load_passkey_counters(state, database, &length);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    for (size_t i = 0; i < database[5]; ++i) {
        size_t name_length = database[offset];
        if (name_length == 0 || name_length > 32 ||
            offset + PASSKEY_COUNTER_ENTRY_BYTES + name_length > length)
            return GHOSTOS_STATUS_CORRUPT;
        if (__builtin_memcmp(database + offset + 5, key_fingerprint, 32) == 0 &&
            username_equal(database + offset + 37, name_length, username, username_length)) {
            uint32_t previous = read_be32(database + offset + 1);
            if (sign_count < previous) return GHOSTOS_STATUS_ACCESS_DENIED;
            write_be32(database + offset + 1, sign_count);
            status = transaction_write_one(state, PASSKEY_COUNTERS_PATH, database, length);
            if (status != GHOSTOS_STATUS_NORMAL) return status;
            return state->filesystem.sync(state->filesystem_context) == GHOSTOS_STATUS_NORMAL
                ? GHOSTOS_STATUS_NORMAL : GHOSTOS_STATUS_INTERNAL;
        }
        offset += PASSKEY_COUNTER_ENTRY_BYTES + name_length;
    }
    if (database[5] == UINT8_MAX || length + PASSKEY_COUNTER_ENTRY_BYTES + username_length > sizeof(database))
        return GHOSTOS_STATUS_NO_SPACE;
    database[offset] = (uint8_t)username_length;
    write_be32(database + offset + 1, sign_count);
    __builtin_memcpy(database + offset + 5, key_fingerprint, 32);
    __builtin_memcpy(database + offset + 37, username, username_length);
    ++database[5];
    length += PASSKEY_COUNTER_ENTRY_BYTES + username_length;
    status = transaction_write_one(state, PASSKEY_COUNTERS_PATH, database, length);
    if (status != GHOSTOS_STATUS_NORMAL) return status;
    return state->filesystem.sync(state->filesystem_context) == GHOSTOS_STATUS_NORMAL
        ? GHOSTOS_STATUS_NORMAL : GHOSTOS_STATUS_INTERNAL;
}

ghostos_status ghostos_boot_shell_filesystem_set_rights(ghostos_boot_state *state,
                                                        uint32_t rights)
{
    uint64_t capability = 0;
    ghostos_status status;
    if (!state || !state->owns_filesystem || rights & ~15u ||
        !state->filesystem.register_process || !state->filesystem.unregister_process)
        return GHOSTOS_STATUS_INVALID_ARGUMENT;
    if (state->shell_filesystem_capability) {
        uint64_t previous = state->shell_filesystem_capability;
        state->shell_filesystem_capability = 0;
        state->shell_filesystem_rights = 0;
        status = state->filesystem.unregister_process(state->filesystem_context, 9);
        if (status != GHOSTOS_STATUS_NORMAL) return GHOSTOS_STATUS_ACCESS_DENIED;
        (void)previous;
    }
    if (rights == 0) return GHOSTOS_STATUS_NORMAL;
    status = state->filesystem.register_process(state->filesystem_context, 9, rights,
                                               &capability);
    if (status != GHOSTOS_STATUS_NORMAL || (capability >> 32) == 0)
        return GHOSTOS_STATUS_NO_SPACE;
    state->shell_filesystem_capability = capability;
    state->shell_filesystem_rights = rights;
    return GHOSTOS_STATUS_NORMAL;
}

static uint32_t map_runtime_filesystem_operation(uint32_t operation)
{
    switch (operation) {
    case GHOSTOS_OP_SYN_FS_OPEN: return 1;
    case GHOSTOS_OP_SYN_FS_CLOSE: return 2;
    case GHOSTOS_OP_SYN_FS_READ: return 3;
    case GHOSTOS_OP_SYN_FS_WRITE: return 4;
    case GHOSTOS_OP_SYN_FS_METADATA: return 5;
    case GHOSTOS_OP_SYN_FS_DELETE: return 6;
    case GHOSTOS_OP_SYN_FS_LIST: return 8;
    case GHOSTOS_OP_SYN_FS_MKDIR: return 16;
    case GHOSTOS_OP_SYN_FS_RMDIR: return 17;
    case GHOSTOS_OP_SYN_FS_MAP: return 25;
    case GHOSTOS_OP_SYN_FS_UNMAP: return 26;
    default: return 0;
    }
}

ghostos_status ghostos_boot_shell_filesystem_request(ghostos_boot_state *state,
                                                     uint32_t operation,
                                                     uint16_t flags,
                                                     uint64_t capability,
                                                     uint64_t offset,
                                                     uint64_t length,
                                                     void *buffer,
                                                     uint64_t values[4])
{
    uint32_t fs_operation;
    uint32_t needed = 0;
    uint64_t authority;
    bool authority_operation;
    if (!state || !state->owns_filesystem || !state->filesystem.dispatch || !values)
        return GHOSTOS_STATUS_BUSY;
    fs_operation = map_runtime_filesystem_operation(operation);
    if (!fs_operation) return GHOSTOS_STATUS_INVALID_ARGUMENT;
    if (!state->shell_filesystem_capability) return GHOSTOS_STATUS_ACCESS_DENIED;
    switch (fs_operation) {
    case 1: needed = (flags & 1u ? GHOSTOS_FILESYSTEM_READ : 0u) |
                     (flags & 2u ? GHOSTOS_FILESYSTEM_WRITE : 0u); break;
    case 3: case 5: case 8: needed = GHOSTOS_FILESYSTEM_READ; break;
    case 4: needed = GHOSTOS_FILESYSTEM_WRITE; break;
    case 6: case 16: case 17: needed = GHOSTOS_FILESYSTEM_DELETE; break;
    case 25: needed = GHOSTOS_FILESYSTEM_READ |
                      ((flags & 2u) ? GHOSTOS_FILESYSTEM_WRITE : 0u); break;
    default: break;
    }
    if ((state->shell_filesystem_rights & needed) != needed)
        return GHOSTOS_STATUS_ACCESS_DENIED;
    authority_operation = fs_operation == 1 || fs_operation == 6 || fs_operation == 8 ||
                          fs_operation == 16 || fs_operation == 17;
    authority = authority_operation || (capability >> 32) == 0
        ? state->shell_filesystem_capability : capability;
    __builtin_memset(values, 0, 4 * sizeof(*values));
    return state->filesystem.dispatch(state->filesystem_context, 9, fs_operation,
                                      flags, authority, offset, length, buffer, values);
}

bool ghostos_boot_shell_filesystem_bind(ghostos_shell_filesystem *filesystem,
                                        ghostos_shell_filesystem_dispatch dispatch,
                                        void *context, uint64_t authority)
{
    if (!filesystem || !dispatch || authority == 0) return false;
    filesystem->dispatch = dispatch;
    filesystem->context = context;
    filesystem->shell_process_id = 9;
    filesystem->authority = authority;
    return true;
}

void ghostos_boot_shell_filesystem_unbind(ghostos_shell_filesystem *filesystem)
{
    if (!filesystem) return;
    filesystem->authority = 0;
}

bool ghostos_boot_shell_filesystem_call(ghostos_shell_filesystem *filesystem,
                                        uint32_t operation, uint16_t flags,
                                        uint64_t capability, uint64_t offset,
                                        uint64_t length, void *buffer,
                                        uint64_t values[4])
{
    if (!filesystem || !filesystem->dispatch || !filesystem->authority || !values)
        return false;
    if (operation == 12 || operation == 17 || operation == 18 || operation == 20 ||
        operation == 22) capability = filesystem->authority;
    return filesystem->dispatch(filesystem->context, filesystem->shell_process_id,
                                operation, flags, capability, offset, length,
                                buffer, values);
}
