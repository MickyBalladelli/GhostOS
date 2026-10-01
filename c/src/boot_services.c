#include "ghostos/boot_services.h"

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
    return authorization_database == NULL || database_length == 0;
}

bool ghostos_boot_first_admin_record_valid(const uint8_t *username,
                                           size_t username_length,
                                           const uint8_t *public_key,
                                           size_t public_key_length)
{
    if (!username || !public_key || username_length == 0 || username_length > 32 ||
        public_key_length < 1 || public_key_length > 96) return false;
    for (size_t i = 0; i < username_length; ++i) {
        uint8_t ch = username[i];
        if (!((ch >= 'a' && ch <= 'z') || (ch >= 'A' && ch <= 'Z') ||
              (ch >= '0' && ch <= '9') || ch == '_' || ch == '-' || ch == '.' || ch == '$'))
            return false;
    }
    return true;
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
