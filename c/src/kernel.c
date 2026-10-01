#include "ghostos/kernel.h"

static void boot_failure(ghostos_kernel_runtime *runtime, ghostos_status status,
    ghostos_kernel_boot_result result) {
    (void)result;
    ghostos_boot_diagnostics_fail(&runtime->diagnostics,status);
    ghostos_boot_diagnostic_fail(status);
    if (runtime->log_status) runtime->log_status(runtime->context,"kernel boot failed",status);
}

static uint64_t saturating_add_u64(uint64_t a,uint64_t b) { return UINT64_MAX-a<b?UINT64_MAX:a+b; }
static void checkpoint(ghostos_kernel_runtime *runtime,ghostos_boot_stage stage) {
    ghostos_boot_diagnostics_checkpoint(&runtime->diagnostics,stage);
    ghostos_boot_diagnostic_checkpoint(stage);
}
static void complete(ghostos_kernel_runtime *runtime) {
    ghostos_boot_diagnostics_complete(&runtime->diagnostics);
    ghostos_boot_diagnostic_complete();
}

bool ghostos_kernel_service_image_fits(size_t length) {
    return length <= GHOSTOS_KERNEL_SERVICE_CODE_BYTES;
}

ghostos_status ghostos_kernel_validate_boot_info(const ghostos_boot_info *boot_info) {
    return ghostos_boot_info_is_valid(boot_info) ? GHOSTOS_STATUS_NORMAL : GHOSTOS_STATUS_INVALID_ARGUMENT;
}

void ghostos_kernel_runtime_init(ghostos_kernel_runtime *runtime,
    const ghostos_arch_backend *architecture) {
    *runtime=(ghostos_kernel_runtime){0};
    if (architecture) runtime->architecture=*architecture;
    runtime->diagnostics=ghostos_boot_diagnostics_initial(1);
    atomic_init(&runtime->service_ready_mask,0);
}

ghostos_kernel_boot_result ghostos_kernel_boot(ghostos_kernel_runtime *runtime,
    const ghostos_boot_info *boot_info) {
    ghostos_boot_attempt reported={0}; bool has_reported=false;
    if (ghostos_boot_diagnostic_begin(&reported)) {
        runtime->previous_failure=reported;
        runtime->has_previous_failure=true;
    }
    if (runtime->boot_started) {
        ghostos_boot_diagnostics previous=runtime->diagnostics;
        ghostos_boot_diagnostics next;
        ghostos_boot_diagnostics_begin(&previous,true,&next,&reported,&has_reported);
        runtime->diagnostics=next;
    } else {
        runtime->diagnostics=ghostos_boot_diagnostics_initial(1);
        if (runtime->has_previous_failure) {
            runtime->diagnostics.last_failure=runtime->previous_failure;
            runtime->diagnostics.has_last_failure=true;
            reported=runtime->previous_failure;
            has_reported=true;
        }
        runtime->boot_started=true;
    }
    if (has_reported) { runtime->previous_failure=reported; runtime->has_previous_failure=true; }
    else runtime->has_previous_failure=false;
    if (ghostos_kernel_validate_boot_info(boot_info)!=GHOSTOS_STATUS_NORMAL) {
        boot_failure(runtime,GHOSTOS_STATUS_INVALID_ARGUMENT,GHOSTOS_KERNEL_BOOT_INVALID_INFO);
        return GHOSTOS_KERNEL_BOOT_INVALID_INFO;
    }
    if (runtime->disable_interrupts) runtime->disable_interrupts(runtime->context);
    ghostos_arch_disable_interrupts(&runtime->architecture);
    if (runtime->console_initialize) runtime->console_initialize(runtime->context,boot_info->framebuffer);
    checkpoint(runtime,GHOSTOS_BOOT_INFO_VALIDATED);

    uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT]={0};
    if (!runtime->memory_initialize || !runtime->memory_initialize(runtime->context,boot_info,tables)) {
        boot_failure(runtime,GHOSTOS_STATUS_NO_SPACE,GHOSTOS_KERNEL_BOOT_MEMORY_FAILED);
        return GHOSTOS_KERNEL_BOOT_MEMORY_FAILED;
    }
    ghostos_invariant_failure invariant=ghostos_invariant_failure_for(GHOSTOS_INVARIANT_PAGE_TABLE_TRANSITION);
    if (!ghostos_invariant_check_page_table_transition(tables,GHOSTOS_ARCH_TABLE_FRAME_COUNT,
            boot_info->physical_address_offset,&invariant) ||
        !runtime->validate_page_tables || !runtime->validate_page_tables(runtime->context,tables,boot_info->physical_address_offset) ||
        !ghostos_arch_initialize(&runtime->architecture,runtime->validate_page_tables,runtime->context,
            tables,boot_info->physical_address_offset)) {
        boot_failure(runtime,GHOSTOS_STATUS_INVALID_ARGUMENT,GHOSTOS_KERNEL_BOOT_ARCH_FAILED);
        return GHOSTOS_KERNEL_BOOT_ARCH_FAILED;
    }
    checkpoint(runtime,GHOSTOS_BOOT_MEMORY_READY);
    if ((runtime->time_initialize && !runtime->time_initialize(runtime->context)) ||
        (runtime->random_initialize && !runtime->random_initialize(runtime->context))) {
        boot_failure(runtime,GHOSTOS_STATUS_BUSY,GHOSTOS_KERNEL_BOOT_TIME_FAILED);
        return GHOSTOS_KERNEL_BOOT_TIME_FAILED;
    }
    checkpoint(runtime,GHOSTOS_BOOT_ARCHITECTURE_READY);
    if (runtime->hardware_initialize && !runtime->hardware_initialize(runtime->context,boot_info)) {
        boot_failure(runtime,GHOSTOS_STATUS_BUSY,GHOSTOS_KERNEL_BOOT_HARDWARE_FAILED);
        return GHOSTOS_KERNEL_BOOT_HARDWARE_FAILED;
    }
    checkpoint(runtime,GHOSTOS_BOOT_HARDWARE_READY);

    bool ring3=runtime->ring3_supported ? runtime->ring3_supported(runtime->context) :
        ghostos_arch_ring3_supported(&runtime->architecture);
    if (!ring3) {
        complete(runtime);
        if (runtime->run_shell) (void)runtime->run_shell(runtime->context,boot_info);
        return GHOSTOS_KERNEL_BOOT_SHELL_FALLBACK;
    }
    ghostos_status storage_status=runtime->storage_initialize ?
        runtime->storage_initialize(runtime->context,boot_info) : GHOSTOS_STATUS_NO_SPACE;
    if (storage_status!=GHOSTOS_STATUS_NORMAL) {
        boot_failure(runtime,storage_status,GHOSTOS_KERNEL_BOOT_STORAGE_FAILED);
        return GHOSTOS_KERNEL_BOOT_STORAGE_FAILED;
    }
    checkpoint(runtime,GHOSTOS_BOOT_STORAGE_READY);
    ghostos_status status=runtime->services_initialize ?
        runtime->services_initialize(runtime->context,&runtime->provisioning_required) : GHOSTOS_STATUS_NO_SPACE;
    if (status!=GHOSTOS_STATUS_NORMAL) {
        boot_failure(runtime,status,GHOSTOS_KERNEL_BOOT_SERVICES_FAILED);
        return GHOSTOS_KERNEL_BOOT_SERVICES_FAILED;
    }
    checkpoint(runtime,GHOSTOS_BOOT_SERVICES_READY);
    ghostos_status handoff_status=runtime->start_init ?
        runtime->start_init(runtime->context,boot_info) : GHOSTOS_STATUS_NO_SPACE;
    if (handoff_status!=GHOSTOS_STATUS_NORMAL) {
        boot_failure(runtime,handoff_status,GHOSTOS_KERNEL_BOOT_HANDOFF_FAILED);
        return GHOSTOS_KERNEL_BOOT_HANDOFF_FAILED;
    }
    checkpoint(runtime,GHOSTOS_BOOT_USER_HANDOFF);
    complete(runtime);
    runtime->scheduler_ready=true;
    return GHOSTOS_KERNEL_BOOT_OK;
}

bool ghostos_kernel_current_address_space(const ghostos_kernel_runtime *runtime, uint32_t *address_space) {
    return runtime && runtime->scheduler_ready && runtime->current_address_space &&
        runtime->current_address_space(runtime->context,address_space);
}

void ghostos_kernel_cpu_idle(ghostos_kernel_runtime *runtime, uint64_t now_us) {
    if (!runtime) for (;;) atomic_signal_fence(memory_order_seq_cst);
    if (!runtime->scheduler_ready || !runtime->scheduler_idle_state) {
        ghostos_arch_halt(&runtime->architecture);
        return;
    }
    uint8_t state=runtime->scheduler_idle_state(runtime->context,saturating_add_u64(now_us,1000),1000);
    ghostos_arch_idle(&runtime->architecture,state);
}

_Noreturn void ghostos_kernel_halt(ghostos_kernel_runtime *runtime) {
    for (;;) ghostos_arch_halt(&runtime->architecture);
}

void ghostos_kernel_capture_exception(ghostos_kernel_runtime *runtime,
    ghostos_crash_register_state registers, uint64_t fault_address, ghostos_status status, uint16_t reason) {
    ghostos_boot_diagnostics_fail(&runtime->diagnostics,status);
    ghostos_boot_diagnostic_fail(status);
    ghostos_crash_capsule capsule={0};
    capsule.reason=reason; capsule.status=status;
    capsule.registers=registers; capsule.registers.fault_address=fault_address;
    ghostos_crash_context_init(&capsule.capabilities);
    if (runtime->scheduler_snapshot) (void)runtime->scheduler_snapshot(runtime->context,&capsule.scheduler);
    (void)ghostos_crash_capture_and_persist_once(&capsule,runtime->persist_crash,runtime->context);
}

_Noreturn void ghostos_kernel_fatal_halt(ghostos_kernel_runtime *runtime, ghostos_status status) {
    ghostos_crash_register_state registers={0};
    if (runtime->capture_registers) registers=runtime->capture_registers(runtime->context);
    ghostos_kernel_capture_exception(runtime,registers,0,status,1);
    if (runtime->log_status) runtime->log_status(runtime->context,"KERNEL HALT",status);
    ghostos_kernel_halt(runtime);
}

_Noreturn void ghostos_kernel_panic_report(ghostos_kernel_runtime *runtime) {
    ghostos_kernel_fatal_halt(runtime,GHOSTOS_STATUS_CORRUPT);
}

bool ghostos_kernel_service_mark_ready(ghostos_kernel_runtime *runtime, uint8_t role) {
    if (!runtime || role>=32) return false;
    uint32_t ready=atomic_load_explicit(&runtime->service_ready_mask,memory_order_acquire);
    if (role==9 && (ready&((UINT32_C(1)<<7)|(UINT32_C(1)<<14)))!=((UINT32_C(1)<<7)|(UINT32_C(1)<<14))) return false;
    atomic_fetch_or_explicit(&runtime->service_ready_mask,UINT32_C(1)<<role,memory_order_release);
    return true;
}
uint32_t ghostos_kernel_service_ready_mask(const ghostos_kernel_runtime *runtime) {
    return runtime ? atomic_load_explicit(&runtime->service_ready_mask,memory_order_acquire) : 0;
}
const char *ghostos_kernel_service_name(uint8_t role) {
    static const char *const names[15]={0,"ghostos-init","ghostos-fsd","ghostos-storaged",
        "ghostos-netd","ghostos-logd","ghostos-auditd","ghostos-authd","ghostos-pkgd",
        "ghostos-shell","ghostos-pcid","ghostos-ahcid","ghostos-nvmed","ghostos-ethernetd",
        "ghostos-logind"};
    return role<15?names[role]:0;
}

void ghostos_kernel_dispatch_request(ghostos_request request, uint32_t caller,
    ghostos_response *response, void *context,
    void (*dispatch)(void *context, ghostos_request request, uint32_t caller, ghostos_response *response)) {
    if (!response) return;
    if (request.abi_version!=GHOSTOS_ABI_SCHEMA_VERSION || request.reserved!=0) {
        *response=(ghostos_response){GHOSTOS_STATUS_INVALID_ARGUMENT,0,{0}};
        return;
    }
    if (!dispatch) { *response=(ghostos_response){GHOSTOS_STATUS_METHOD_NOT_ALLOWED,0,{0}}; return; }
    dispatch(context,request,caller,response);
}

void ghostos_kernel_login_init(ghostos_kernel_login *l) {
    atomic_flag_clear(&l->lock);
    atomic_init(&l->authorized,false); atomic_init(&l->login_requested,true);
    atomic_init(&l->administrator_exists,false); atomic_init(&l->bootstrap_proof,false);
    atomic_init(&l->bridge_active,false); atomic_init(&l->passkey_challenge_ready,false);
    atomic_init(&l->tpm_challenge_ready,false); atomic_init(&l->failed_attempts,0);
    atomic_init(&l->retry_after_us,0); atomic_init(&l->locked_until_us,0);
    atomic_init(&l->session_expires_us,0); atomic_init(&l->last_activity_us,0);
    atomic_init(&l->identity,0); atomic_init(&l->revocation_epoch,1); atomic_init(&l->username_length,0);
    for (size_t i=0;i<GHOSTOS_KERNEL_LOGIN_USERNAME_CAPACITY;i++) atomic_init(&l->username[i],0);
    for (size_t i=0;i<GHOSTOS_KERNEL_TPM_CHALLENGE_BYTES;i++) {
        atomic_init(&l->passkey_challenge[i],0); atomic_init(&l->tpm_challenge[i],0);
    }
}

bool ghostos_kernel_login_valid_username(const uint8_t *username,size_t length) {
    if (!username || !length || length>GHOSTOS_KERNEL_LOGIN_USERNAME_CAPACITY) return false;
    for (size_t i=0;i<length;i++) {
        uint8_t c=username[i];
        if (!((c>='a'&&c<='z')||(c>='A'&&c<='Z')||(c>='0'&&c<='9')||c=='.'||c=='_'||c=='-'||c=='$')) return false;
    }
    return true;
}

static uint8_t ascii_lower(uint8_t c) { return c>='A'&&c<='Z' ? (uint8_t)(c-'A'+'a') : c; }
uint64_t ghostos_kernel_login_identity(const uint8_t *username,size_t length) {
    if (!username && length) return 0;
    uint64_t identity=UINT64_C(0xcbf29ce484222325);
    for (size_t i=0;i<length;i++) { identity^=ascii_lower(username[i]); identity*=UINT64_C(0x100000001b3); }
    return identity ? identity : 1;
}
void ghostos_kernel_login_set_username(ghostos_kernel_login *l,const uint8_t *username,size_t length) {
    if (!ghostos_kernel_login_valid_username(username,length)) return;
    for (size_t i=0;i<GHOSTOS_KERNEL_LOGIN_USERNAME_CAPACITY;i++)
        atomic_store_explicit(&l->username[i],i<length?ascii_lower(username[i]):0,memory_order_relaxed);
    atomic_store_explicit(&l->username_length,(unsigned)length,memory_order_release);
}
void ghostos_kernel_login_clear_username(ghostos_kernel_login *l) {
    atomic_store_explicit(&l->username_length,0,memory_order_release);
    for (size_t i=0;i<GHOSTOS_KERNEL_LOGIN_USERNAME_CAPACITY;i++) atomic_store_explicit(&l->username[i],0,memory_order_relaxed);
}
void ghostos_kernel_login_record_failure(ghostos_kernel_login *l,const ghostos_kernel_login_ops *ops,uint64_t now) {
    unsigned failures=atomic_load_explicit(&l->failed_attempts,memory_order_relaxed);
    while (failures<UINT32_MAX && !atomic_compare_exchange_weak_explicit(&l->failed_attempts,
            &failures,failures+1,memory_order_acq_rel,memory_order_relaxed)) {}
    if (failures<UINT32_MAX) ++failures;
    unsigned shift=failures?failures-1:0; if (shift>6) shift=6;
    uint64_t delay=GHOSTOS_KERNEL_LOGIN_RATE_BASE_US<<shift;
    if (delay>GHOSTOS_KERNEL_LOGIN_RATE_MAX_US) delay=GHOSTOS_KERNEL_LOGIN_RATE_MAX_US;
    atomic_store_explicit(&l->retry_after_us,UINT64_MAX-now<delay?UINT64_MAX:now+delay,memory_order_release);
    if (ops && ops->audit) ops->audit(ops->context,2,0,GHOSTOS_STATUS_ACCESS_DENIED);
    if (failures>=GHOSTOS_KERNEL_LOGIN_LOCK_THRESHOLD) {
        uint64_t until=UINT64_MAX-now<GHOSTOS_KERNEL_LOGIN_LOCK_DURATION_US?UINT64_MAX:now+GHOSTOS_KERNEL_LOGIN_LOCK_DURATION_US;
        atomic_store_explicit(&l->locked_until_us,until,memory_order_release);
        if (ops && ops->audit) ops->audit(ops->context,5,0,GHOSTOS_STATUS_ACCESS_DENIED);
    }
}
void ghostos_kernel_login_clear_failures(ghostos_kernel_login *l) {
    atomic_store_explicit(&l->failed_attempts,0,memory_order_release);
    atomic_store_explicit(&l->retry_after_us,0,memory_order_release);
    atomic_store_explicit(&l->locked_until_us,0,memory_order_release);
}
uint64_t ghostos_kernel_login_lock_until(ghostos_kernel_login *l,uint64_t now) {
    uint64_t until=atomic_load_explicit(&l->locked_until_us,memory_order_acquire);
    if (!until || now<until) return until;
    if (atomic_compare_exchange_strong_explicit(&l->locked_until_us,&until,0,memory_order_acq_rel,memory_order_acquire)) {
        atomic_store_explicit(&l->failed_attempts,0,memory_order_release);
        atomic_store_explicit(&l->retry_after_us,0,memory_order_release);
        return 0;
    }
    return until;
}
bool ghostos_kernel_login_rate_limited(const ghostos_kernel_login *l,uint64_t now) {
    return now<atomic_load_explicit(&l->retry_after_us,memory_order_acquire);
}
void ghostos_kernel_login_clear_challenges(ghostos_kernel_login *l) {
    atomic_store_explicit(&l->passkey_challenge_ready,false,memory_order_release);
    atomic_store_explicit(&l->tpm_challenge_ready,false,memory_order_release);
    for (size_t i=0;i<GHOSTOS_KERNEL_TPM_CHALLENGE_BYTES;i++) {
        atomic_store_explicit(&l->passkey_challenge[i],0,memory_order_relaxed);
        atomic_store_explicit(&l->tpm_challenge[i],0,memory_order_relaxed);
    }
}
static uint16_t read_le16(const uint8_t *p) { return (uint16_t)(p[0]|((uint16_t)p[1]<<8)); }
bool ghostos_kernel_login_valid_tpm_quote(const ghostos_kernel_login *l,const uint8_t *q,size_t n) {
    const size_t header=46;
    if (!q || n<header || q[0]!='S'||q[1]!='Y'||q[2]!='T'||q[3]!='Q'||q[4]!=1||q[5]||q[6]||q[7]||
        !atomic_load_explicit(&l->tpm_challenge_ready,memory_order_acquire)) return false;
    for (size_t i=0;i<GHOSTOS_KERNEL_TPM_CHALLENGE_BYTES;i++)
        if (q[8+i]!=atomic_load_explicit(&l->tpm_challenge[i],memory_order_relaxed)) return false;
    size_t quote_length=read_le16(q+40), signature_length=read_le16(q+42), digest_length=read_le16(q+44);
    if (quote_length<8 || signature_length<16 || digest_length!=32 ||
        quote_length>n-header || signature_length>n-header-quote_length || digest_length!=n-header-quote_length-signature_length) return false;
    size_t end=header+quote_length;
    if (q[header]!=0xff || q[header+1]!=0x54 || q[header+2]!=0x43 || q[header+3]!=0x47 ||
        q[header+4]!=0x80 || q[header+5]!=0x18) return false;
    for (size_t i=end+signature_length;i<n;i++) if (q[i]) return true;
    return false;
}

static void login_lock(ghostos_kernel_login *l) { while (atomic_flag_test_and_set_explicit(&l->lock,memory_order_acquire)) atomic_signal_fence(memory_order_seq_cst); }
static void login_unlock(ghostos_kernel_login *l) { atomic_flag_clear_explicit(&l->lock,memory_order_release); }
static void revoke_locked(ghostos_kernel_login *l,const ghostos_kernel_login_ops *ops) {
    atomic_store_explicit(&l->authorized,false,memory_order_release); atomic_store_explicit(&l->login_requested,true,memory_order_release);
    atomic_store_explicit(&l->session_expires_us,0,memory_order_release); atomic_store_explicit(&l->last_activity_us,0,memory_order_release);
    atomic_store_explicit(&l->identity,0,memory_order_release);
    uint64_t epoch=atomic_fetch_add_explicit(&l->revocation_epoch,1,memory_order_acq_rel)+1;
    if (!epoch) { atomic_store_explicit(&l->revocation_epoch,1,memory_order_release); epoch=1; }
    if (ops && ops->set_shell_filesystem_rights) (void)ops->set_shell_filesystem_rights(ops->context,0);
    if (ops && ops->revoke_shell_session) ops->revoke_shell_session(ops->context,epoch);
    ghostos_kernel_login_clear_challenges(l); ghostos_kernel_login_clear_username(l);
}
void ghostos_kernel_login_revoke_session(ghostos_kernel_login *l,const ghostos_kernel_login_ops *ops) {
    login_lock(l); revoke_locked(l,ops); login_unlock(l);
}
bool ghostos_kernel_login_session_matches(const ghostos_kernel_login *l,uint64_t identity,
    uint64_t expires,uint64_t epoch,uint64_t now) {
    uint64_t last=atomic_load_explicit(&l->last_activity_us,memory_order_acquire);
    return atomic_load_explicit(&l->authorized,memory_order_acquire) && identity &&
        atomic_load_explicit(&l->identity,memory_order_acquire)==identity &&
        atomic_load_explicit(&l->session_expires_us,memory_order_acquire)==expires &&
        atomic_load_explicit(&l->revocation_epoch,memory_order_acquire)==epoch && now<expires &&
        (now>=last?now-last:0)<GHOSTOS_KERNEL_LOGIN_IDLE_TIMEOUT_US;
}
static bool session_active_locked(ghostos_kernel_login *l,const ghostos_kernel_login_ops *ops,uint64_t now) {
    if (!atomic_load_explicit(&l->authorized,memory_order_acquire)) return false;
    uint64_t identity=atomic_load_explicit(&l->identity,memory_order_acquire);
    uint64_t expires=atomic_load_explicit(&l->session_expires_us,memory_order_acquire);
    uint64_t epoch=atomic_load_explicit(&l->revocation_epoch,memory_order_acquire);
    if (!ghostos_kernel_login_session_matches(l,identity,expires,epoch,now)) {
        if (ops && ops->audit) ops->audit(ops->context,4,identity,GHOSTOS_STATUS_ACCESS_DENIED);
        revoke_locked(l,ops);
        return false;
    }
    return true;
}
bool ghostos_kernel_login_session_active(ghostos_kernel_login *l,const ghostos_kernel_login_ops *ops,uint64_t now) {
    login_lock(l); bool active=session_active_locked(l,ops,now); login_unlock(l); return active;
}
bool ghostos_kernel_login_start_session(ghostos_kernel_login *l,const ghostos_kernel_login_ops *ops,
    const uint8_t *username,size_t length,uint64_t now) {
    if (!ghostos_kernel_login_valid_username(username,length) || !ops ||
        !ops->set_shell_filesystem_rights || !ops->authorize_shell_session) return false;
    uint64_t expires=UINT64_MAX-now<GHOSTOS_KERNEL_LOGIN_SESSION_LIFETIME_US?UINT64_MAX:now+GHOSTOS_KERNEL_LOGIN_SESSION_LIFETIME_US;
    uint64_t epoch=atomic_load_explicit(&l->revocation_epoch,memory_order_acquire); if (!epoch) epoch=1;
    uint64_t identity=ghostos_kernel_login_identity(username,length);
    login_lock(l);
    bool ready=ops->set_shell_filesystem_rights(ops->context,15) &&
        ops->authorize_shell_session(ops->context,identity,expires,epoch);
    if (!ready) {
        (void)ops->set_shell_filesystem_rights(ops->context,0);
        login_unlock(l); return false;
    }
    atomic_store_explicit(&l->last_activity_us,now,memory_order_release);
    atomic_store_explicit(&l->session_expires_us,expires,memory_order_release);
    atomic_store_explicit(&l->identity,identity,memory_order_release);
    atomic_store_explicit(&l->login_requested,false,memory_order_release);
    atomic_store_explicit(&l->authorized,true,memory_order_release);
    if (ops->audit) ops->audit(ops->context,1,identity,GHOSTOS_STATUS_NORMAL);
    login_unlock(l);
    if (ops->drain_console_input) ops->drain_console_input(ops->context);
    return true;
}
void ghostos_kernel_login_record_activity(ghostos_kernel_login *l,uint64_t now) {
    atomic_store_explicit(&l->last_activity_us,now,memory_order_release);
}
