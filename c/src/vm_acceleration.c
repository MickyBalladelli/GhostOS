#include "ghostos/vm_acceleration.h"
#include <string.h>
#if defined(__linux__)
#include <sys/ioctl.h>
#endif

const char *ghostos_vm_acceleration_path(uint32_t path) {
    static const char *const paths[] = {"/dev/kvm", "/dev/HAXM", "/dev/haxm"};
    return path < 3 ? paths[path] : "";
}
const char *ghostos_vm_acceleration_reason(uint32_t reason) {
    static const char *const reasons[] = {
        "host accelerator handle acquired", "KVM requires Linux and /dev/kvm",
        "KVM API version is too old", "HAXM device node was not found",
        "HVF requires macOS", "WHPX requires Windows"
    };
    return reason < sizeof(reasons) / sizeof(reasons[0]) ? reasons[reason] : "";
}
const char *ghostos_vm_acceleration_description(bool native_handle) {
    return native_handle ? "native host handle acquired; portable software execution remains active"
        : "portable software execution";
}
int32_t ghostos_vm_acceleration_kvm_version(int32_t fd) {
#if defined(__linux__)
    return ioctl(fd, 0xae00UL);
#else
    (void)fd;
    return -1;
#endif
}
static uint32_t probe(uint32_t backend, uint32_t host, const ghostos_vm_acceleration_io *io,
    ghostos_vm_acceleration_session *out) {
    switch (backend) {
        case GHOSTOS_VM_ACCEL_KVM:
            if (host != GHOSTOS_VM_HOST_LINUX) return GHOSTOS_VM_ACCEL_KVM_HOST;
            if (!io->open(io->context, 0)) return GHOSTOS_VM_ACCEL_IO;
            out->api_version = io->api_version(io->context);
            if (out->api_version < 12) {
                io->close(io->context);
                return GHOSTOS_VM_ACCEL_KVM_OLD;
            }
            out->path = 0;
            return GHOSTOS_VM_ACCEL_AVAILABLE;
        case GHOSTOS_VM_ACCEL_HAXM:
            for (uint32_t path = 1; path <= 2; ++path) {
                if (io->exists(io->context, path) && io->open(io->context, path)) {
                    out->path = path;
                    return GHOSTOS_VM_ACCEL_AVAILABLE;
                }
            }
            return GHOSTOS_VM_ACCEL_HAXM_MISSING;
        case GHOSTOS_VM_ACCEL_HVF:
            return host == GHOSTOS_VM_HOST_MACOS ? GHOSTOS_VM_ACCEL_AVAILABLE : GHOSTOS_VM_ACCEL_HVF_HOST;
        case GHOSTOS_VM_ACCEL_WHPX:
            return host == GHOSTOS_VM_HOST_WINDOWS ? GHOSTOS_VM_ACCEL_AVAILABLE : GHOSTOS_VM_ACCEL_WHPX_HOST;
        default: return GHOSTOS_VM_ACCEL_IO;
    }
}
bool ghostos_vm_acceleration_open(uint32_t requested, uint32_t host,
    const ghostos_vm_acceleration_io *io, ghostos_vm_acceleration_session *out) {
    memset(out, 0, sizeof(*out)); out->requested = requested;
    if (requested == GHOSTOS_VM_ACCEL_SOFTWARE) return true;
    if (requested == GHOSTOS_VM_ACCEL_AUTO) {
        uint32_t first;
        switch (host) {
            case GHOSTOS_VM_HOST_LINUX: first = GHOSTOS_VM_ACCEL_KVM; break;
            case GHOSTOS_VM_HOST_MACOS: first = GHOSTOS_VM_ACCEL_HVF; break;
            case GHOSTOS_VM_HOST_WINDOWS: first = GHOSTOS_VM_ACCEL_WHPX; break;
            default: return true;
        }
        const uint32_t backends[] = {first, GHOSTOS_VM_ACCEL_HAXM};
        for (size_t i = 0; i < 2; ++i) {
            uint32_t reason = probe(backends[i], host, io, out);
            io->attempt(io->context, backends[i], reason == GHOSTOS_VM_ACCEL_AVAILABLE, reason);
            if (reason == GHOSTOS_VM_ACCEL_AVAILABLE) { out->active = backends[i]; return true; }
        }
        return true;
    }
    uint32_t reason = probe(requested, host, io, out);
    if (reason != GHOSTOS_VM_ACCEL_AVAILABLE) {
        out->error_backend = requested; out->error_reason = reason;
        return false;
    }
    out->active = requested;
    io->attempt(io->context, requested, true, GHOSTOS_VM_ACCEL_AVAILABLE);
    return true;
}
void ghostos_vm_acceleration_status_get(uint32_t requested, uint32_t active,
    ghostos_vm_acceleration_status *out) {
    out->native_handle = active != GHOSTOS_VM_ACCEL_SOFTWARE;
    out->fallback = requested == GHOSTOS_VM_ACCEL_SOFTWARE ? 0 : out->native_handle ? 2 : 1;
    out->feature_count = out->native_handle ? 5 : 4;
    out->limitation_count = out->native_handle ? 3 : 1;
}

static const char *backend_name(uint32_t value) {
    static const char *const names[] = {"software", "auto", "kvm", "haxm", "hvf", "whpx"};
    return value < 6 ? names[value] : "";
}
static const char *feature_name(uint32_t value) {
    static const char *const names[] = {"portable-cpu-execution", "guest-device-emulation",
        "deterministic-replay", "snapshot-restore", "native-host-handle"};
    return value < 5 ? names[value] : "";
}
static const char *limitation_name(uint32_t value) {
    static const char *const names[] = {"native-execution-not-integrated",
        "native-memory-and-device-virtualization-not-integrated", "native-handle-reopened-on-snapshot-restore"};
    return value < 3 ? names[value] : "";
}
static const char *fallback_name(uint32_t value) {
    static const char *const names[] = {"none", "no-native-backend", "native-execution-unavailable"};
    return value < 3 ? names[value] : "";
}
static bool text(ghostos_vm_acceleration_write_fn write, void *context, const char *value) {
    return write(context, (const uint8_t *)value, strlen(value));
}
bool ghostos_vm_acceleration_format(const ghostos_vm_acceleration_report *r,
    ghostos_vm_acceleration_write_fn write, void *context) {
    if (!text(write, context, "requested=") || !text(write, context, backend_name(r->requested))
        || !text(write, context, " host=") || !text(write, context, backend_name(r->active))
        || !text(write, context, " execution=") || !text(write, context, backend_name(r->execution))
        || !text(write, context, " fallback=") || !text(write, context, fallback_name(r->fallback))
        || !text(write, context, " supported=")) return false;
    for (size_t i = 0; i < r->feature_count; ++i) {
        if (i && !text(write, context, ",")) return false;
        if (!text(write, context, feature_name(r->features[i]))) return false;
    }
    if (!text(write, context, " limitations=")) return false;
    for (size_t i = 0; i < r->limitation_count; ++i) {
        if (i && !text(write, context, ",")) return false;
        if (!text(write, context, limitation_name(r->limitations[i]))) return false;
    }
    if (r->attempt_count && !text(write, context, " attempts=")) return false;
    for (size_t i = 0; i < r->attempt_count; ++i) {
        const ghostos_vm_acceleration_attempt *a = &r->attempts[i];
        if (i && !text(write, context, ",")) return false;
        if (!text(write, context, backend_name(a->backend)) || !text(write, context, ":")
            || !text(write, context, a->available ? "available(" : "unavailable(")
            || !write(context, a->reason, a->reason_length) || !text(write, context, ")")) return false;
    }
    return text(write, context, " (") && write(context, r->description, r->description_length)
        && text(write, context, ")");
}
