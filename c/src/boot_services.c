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
