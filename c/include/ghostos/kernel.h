#ifndef GHOSTOS_KERNEL_H
#define GHOSTOS_KERNEL_H

#include "ghostos/abi.h"
#include "ghostos/address_space.h"
#include "ghostos/arch.h"
#include "ghostos/boot_diagnostics.h"
#include "ghostos/boot_protocol.h"
#include "ghostos/capability.h"
#include "ghostos/contention.h"
#include "ghostos/crash.h"
#include "ghostos/dlm.h"
#include "ghostos/dma.h"
#include "ghostos/driver_capabilities.h"
#include "ghostos/hot_allocator.h"
#include "ghostos/invariants.h"
#include "ghostos/keyboard.h"
#include "ghostos/keyboard_stub.h"

#include <stdatomic.h>

#define GHOSTOS_KERNEL_SERVICE_CODE_PAGES 19u
#define GHOSTOS_KERNEL_SERVICE_STACK_PAGES 16u
#define GHOSTOS_KERNEL_SERVICE_CODE_BYTES (GHOSTOS_KERNEL_SERVICE_CODE_PAGES * GHOSTOS_PAGE_SIZE)
#define GHOSTOS_KERNEL_LOGIN_USERNAME_CAPACITY 32u
#define GHOSTOS_KERNEL_TPM_CHALLENGE_BYTES 32u
#define GHOSTOS_KERNEL_LOGIN_RATE_BASE_US UINT64_C(1000000)
#define GHOSTOS_KERNEL_LOGIN_RATE_MAX_US UINT64_C(60000000)
#define GHOSTOS_KERNEL_LOGIN_LOCK_THRESHOLD 5u
#define GHOSTOS_KERNEL_LOGIN_LOCK_DURATION_US UINT64_C(300000000)
#define GHOSTOS_KERNEL_LOGIN_SESSION_LIFETIME_US UINT64_C(900000000)
#define GHOSTOS_KERNEL_LOGIN_IDLE_TIMEOUT_US UINT64_C(300000000)

typedef enum {
    GHOSTOS_KERNEL_BOOT_OK,
    GHOSTOS_KERNEL_BOOT_INVALID_INFO,
    GHOSTOS_KERNEL_BOOT_MEMORY_FAILED,
    GHOSTOS_KERNEL_BOOT_ARCH_FAILED,
    GHOSTOS_KERNEL_BOOT_TIME_FAILED,
    GHOSTOS_KERNEL_BOOT_HARDWARE_FAILED,
    GHOSTOS_KERNEL_BOOT_STORAGE_FAILED,
    GHOSTOS_KERNEL_BOOT_SERVICES_FAILED,
    GHOSTOS_KERNEL_BOOT_HANDOFF_FAILED,
    GHOSTOS_KERNEL_BOOT_SHELL_FALLBACK
} ghostos_kernel_boot_result;

typedef struct {
    void *context;
    void (*disable_interrupts)(void *context);
    void (*console_initialize)(void *context, ghostos_framebuffer_info framebuffer);
    bool (*memory_initialize)(void *context, const ghostos_boot_info *boot_info,
        uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT]);
    bool (*validate_page_tables)(void *context, const uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT],
        uint64_t physical_offset);
    bool (*time_initialize)(void *context);
    bool (*random_initialize)(void *context);
    bool (*hardware_initialize)(void *context, const ghostos_boot_info *boot_info);
    bool (*ring3_supported)(void *context);
    ghostos_status (*storage_initialize)(void *context, const ghostos_boot_info *boot_info);
    ghostos_status (*services_initialize)(void *context, bool *provisioning_required);
    ghostos_status (*run_shell)(void *context, const ghostos_boot_info *boot_info);
    ghostos_status (*start_init)(void *context, const ghostos_boot_info *boot_info);
    void (*log_status)(void *context, const char *message, ghostos_status status);
    ghostos_crash_persist_fn persist_crash;
    ghostos_crash_register_state (*capture_registers)(void *context);
    bool (*scheduler_snapshot)(void *context, ghostos_crash_scheduler_state *snapshot);
    uint8_t (*scheduler_idle_state)(void *context, uint64_t deadline_us, uint64_t max_idle_us);
    bool (*current_address_space)(void *context, uint32_t *address_space);
    ghostos_arch_backend architecture;
    ghostos_boot_diagnostics diagnostics;
    bool has_previous_failure;
    ghostos_boot_attempt previous_failure;
    bool boot_started;
    bool provisioning_required;
    bool scheduler_ready;
    atomic_uint service_ready_mask;
} ghostos_kernel_runtime;

typedef struct {
    atomic_flag lock;
    atomic_bool authorized, login_requested, administrator_exists, bootstrap_proof, bridge_active;
    atomic_bool passkey_challenge_ready, tpm_challenge_ready;
    atomic_uint failed_attempts;
    atomic_uint_fast64_t retry_after_us, locked_until_us, session_expires_us;
    atomic_uint_fast64_t last_activity_us, identity, revocation_epoch;
    atomic_uint username_length;
    atomic_uchar username[GHOSTOS_KERNEL_LOGIN_USERNAME_CAPACITY];
    atomic_uchar passkey_challenge[GHOSTOS_KERNEL_TPM_CHALLENGE_BYTES];
    atomic_uchar tpm_challenge[GHOSTOS_KERNEL_TPM_CHALLENGE_BYTES];
} ghostos_kernel_login;

typedef struct {
    void *context;
    bool (*set_shell_filesystem_rights)(void *context, uint32_t rights);
    bool (*authorize_shell_session)(void *context, uint64_t identity,
        uint64_t expires_at_us, uint64_t revocation_epoch);
    void (*revoke_shell_session)(void *context, uint64_t revocation_epoch);
    void (*audit)(void *context, uint64_t action, uint64_t identity, ghostos_status status);
    void (*drain_console_input)(void *context);
} ghostos_kernel_login_ops;

typedef struct {
    bool is_kernel, online, smp, interrupts, user_mode, isolation;
    uint32_t cpu_count;
    size_t memory_region_count;
    uint64_t monotonic_time_us;
    bool realtime_ready, entropy_ready, provisioning_required;
} ghostos_kernel_system_info;

bool ghostos_kernel_service_image_fits(size_t length);
ghostos_status ghostos_kernel_validate_boot_info(const ghostos_boot_info *boot_info);
void ghostos_kernel_runtime_init(ghostos_kernel_runtime *runtime,
    const ghostos_arch_backend *architecture);
ghostos_kernel_boot_result ghostos_kernel_boot(ghostos_kernel_runtime *runtime,
    const ghostos_boot_info *boot_info);
bool ghostos_kernel_current_address_space(const ghostos_kernel_runtime *runtime, uint32_t *address_space);
void ghostos_kernel_cpu_idle(ghostos_kernel_runtime *runtime, uint64_t now_us);
_Noreturn void ghostos_kernel_halt(ghostos_kernel_runtime *runtime);
_Noreturn void ghostos_kernel_fatal_halt(ghostos_kernel_runtime *runtime, ghostos_status status);
_Noreturn void ghostos_kernel_panic_report(ghostos_kernel_runtime *runtime);
void ghostos_kernel_capture_exception(ghostos_kernel_runtime *runtime,
    ghostos_crash_register_state registers, uint64_t fault_address, ghostos_status status, uint16_t reason);
bool ghostos_kernel_service_mark_ready(ghostos_kernel_runtime *runtime, uint8_t role);
uint32_t ghostos_kernel_service_ready_mask(const ghostos_kernel_runtime *runtime);
const char *ghostos_kernel_service_name(uint8_t role);
void ghostos_kernel_dispatch_request(ghostos_request request, uint32_t caller,
    ghostos_response *response, void *context,
    void (*dispatch)(void *context, ghostos_request request, uint32_t caller, ghostos_response *response));
void ghostos_kernel_login_init(ghostos_kernel_login *login);
bool ghostos_kernel_login_valid_username(const uint8_t *username, size_t length);
uint64_t ghostos_kernel_login_identity(const uint8_t *username, size_t length);
void ghostos_kernel_login_set_username(ghostos_kernel_login *login, const uint8_t *username, size_t length);
void ghostos_kernel_login_clear_username(ghostos_kernel_login *login);
void ghostos_kernel_login_record_failure(ghostos_kernel_login *login,
    const ghostos_kernel_login_ops *ops, uint64_t now_us);
void ghostos_kernel_login_clear_failures(ghostos_kernel_login *login);
uint64_t ghostos_kernel_login_lock_until(ghostos_kernel_login *login, uint64_t now_us);
bool ghostos_kernel_login_rate_limited(const ghostos_kernel_login *login, uint64_t now_us);
void ghostos_kernel_login_clear_challenges(ghostos_kernel_login *login);
bool ghostos_kernel_login_valid_tpm_quote(const ghostos_kernel_login *login,
    const uint8_t *quote, size_t length);
bool ghostos_kernel_login_start_session(ghostos_kernel_login *login,
    const ghostos_kernel_login_ops *ops, const uint8_t *username, size_t length, uint64_t now_us);
void ghostos_kernel_login_revoke_session(ghostos_kernel_login *login,
    const ghostos_kernel_login_ops *ops);
bool ghostos_kernel_login_session_matches(const ghostos_kernel_login *login,
    uint64_t identity, uint64_t expires_at_us, uint64_t epoch, uint64_t now_us);
bool ghostos_kernel_login_session_active(ghostos_kernel_login *login,
    const ghostos_kernel_login_ops *ops, uint64_t now_us);
void ghostos_kernel_login_record_activity(ghostos_kernel_login *login, uint64_t now_us);

#endif
