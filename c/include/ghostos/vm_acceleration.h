#ifndef GHOSTOS_VM_ACCELERATION_H
#define GHOSTOS_VM_ACCELERATION_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

enum { GHOSTOS_VM_ACCEL_SOFTWARE, GHOSTOS_VM_ACCEL_AUTO, GHOSTOS_VM_ACCEL_KVM,
    GHOSTOS_VM_ACCEL_HAXM, GHOSTOS_VM_ACCEL_HVF, GHOSTOS_VM_ACCEL_WHPX };
enum { GHOSTOS_VM_HOST_OTHER, GHOSTOS_VM_HOST_LINUX, GHOSTOS_VM_HOST_MACOS, GHOSTOS_VM_HOST_WINDOWS };
enum { GHOSTOS_VM_ACCEL_AVAILABLE, GHOSTOS_VM_ACCEL_KVM_HOST,
    GHOSTOS_VM_ACCEL_KVM_OLD, GHOSTOS_VM_ACCEL_HAXM_MISSING,
    GHOSTOS_VM_ACCEL_HVF_HOST, GHOSTOS_VM_ACCEL_WHPX_HOST, GHOSTOS_VM_ACCEL_IO };

typedef struct {
    bool (*exists)(void *, uint32_t path);
    bool (*open)(void *, uint32_t path);
    int32_t (*api_version)(void *);
    void (*close)(void *);
    void (*attempt)(void *, uint32_t backend, bool available, uint32_t reason);
    void *context;
} ghostos_vm_acceleration_io;

typedef struct {
    uint32_t requested, active, error_backend, error_reason;
    int32_t api_version;
    uint32_t path;
} ghostos_vm_acceleration_session;
typedef struct {
    uint32_t fallback, feature_count, limitation_count;
    bool native_handle;
} ghostos_vm_acceleration_status;

/* Paths: 0 /dev/kvm, 1 /dev/HAXM, 2 /dev/haxm. A successful open
 * transfers the retained host handle to the caller. Close is called only
 * for rejected KVM versions. Callbacks must not unwind or reenter. */
bool ghostos_vm_acceleration_open(uint32_t requested, uint32_t host,
    const ghostos_vm_acceleration_io *io, ghostos_vm_acceleration_session *out);
void ghostos_vm_acceleration_status_get(uint32_t requested, uint32_t active,
    ghostos_vm_acceleration_status *out);
const char *ghostos_vm_acceleration_reason(uint32_t reason);
const char *ghostos_vm_acceleration_description(bool native_handle);
const char *ghostos_vm_acceleration_path(uint32_t path);
int32_t ghostos_vm_acceleration_kvm_version(int32_t fd);

#endif
