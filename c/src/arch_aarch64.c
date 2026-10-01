#include "ghostos/arch_aarch64.h"
#include "ghostos/status.h"

#include <stdatomic.h>
#include <stddef.h>

static atomic_uint_fast64_t isolated_cores;
static ghostos_aarch64_exception_hooks exception_hooks;

void ghostos_aarch64_set_core_isolated(uint8_t cpu, bool isolated) {
    if (cpu >= 64) return;
    uint64_t bit = UINT64_C(1) << cpu;
    if (isolated) atomic_fetch_or_explicit(&isolated_cores, bit, memory_order_release);
    else atomic_fetch_and_explicit(&isolated_cores, ~bit, memory_order_release);
}

bool ghostos_aarch64_core_isolated(uint8_t cpu) {
    if (cpu >= 64) return false;
    return (atomic_load_explicit(&isolated_cores, memory_order_acquire) & (UINT64_C(1) << cpu)) != 0;
}

void ghostos_aarch64_set_exception_hooks(ghostos_aarch64_exception_hooks hooks) { exception_hooks = hooks; }

void ghostos_aarch64_exception_dispatch(ghostos_aarch64_exception_frame *frame) {
    uint8_t ec = (uint8_t)((frame->esr >> 26) & 0x3f);
    if (ec == 0x15) {
        if (exception_hooks.syscall_dispatch) exception_hooks.syscall_dispatch(exception_hooks.context, frame->registers[0], frame->registers[1]);
        if (UINT64_MAX - frame->elr >= 4) frame->elr += 4;
        return;
    }
    if (exception_hooks.capture_exception) {
        ghostos_aarch64_register_state state = {{0}, 0, 0, frame->spsr, frame->far};
        for (size_t i = 0; i < 4; ++i) state.general[i] = frame->registers[i];
        exception_hooks.capture_exception(exception_hooks.context, state, frame->far, GHOSTOS_STATUS_CORRUPT, ec);
    }
    if (exception_hooks.log_exception) exception_hooks.log_exception(exception_hooks.context, ec);
#if defined(__aarch64__) && defined(__ELF__)
    ghostos_aarch64_halt();
#else
    if (exception_hooks.halt) exception_hooks.halt(exception_hooks.context);
#endif
}

#if defined(__aarch64__) && defined(__ELF__)

#define AARCH64_ASM(...) __asm__ volatile(__VA_ARGS__)

bool ghostos_aarch64_install_root(const uint64_t tables[GHOSTOS_AARCH64_TABLE_FRAME_COUNT], uint64_t offset) {
    uint64_t *l0 = (uint64_t *)(uintptr_t)(tables[0] + offset);
    uint64_t *l1 = (uint64_t *)(uintptr_t)(tables[1] + offset);
    for (size_t i = 0; i < 512; ++i) { l0[i] = 0; l1[i] = 0; }
    l0[0] = tables[1] | UINT64_C(3);
    for (size_t i = 0; i < 4; ++i) l1[i] = ((uint64_t)i * UINT64_C(1073741824)) | UINT64_C(1) | (UINT64_C(1) << 10) | (UINT64_C(3) << 8);
    uint64_t mair = UINT64_C(0xff);
    uint64_t tcr = (UINT64_C(16) << 0) | (UINT64_C(3) << 12) | (UINT64_C(1) << 10);
    AARCH64_ASM("msr mair_el1, %0" :: "r"(mair) : "memory");
    AARCH64_ASM("msr tcr_el1, %0\nisb" :: "r"(tcr) : "memory");
    AARCH64_ASM("msr ttbr0_el1, %0\ndsb ish\nisb" :: "r"(tables[0]) : "memory");
    uint64_t sctlr;
    AARCH64_ASM("mrs %0, sctlr_el1" : "=r"(sctlr));
    sctlr |= (UINT64_C(1) << 0) | (UINT64_C(1) << 2) | (UINT64_C(1) << 12);
    AARCH64_ASM("msr sctlr_el1, %0\nisb" :: "r"(sctlr) : "memory");
    return true;
}

void ghostos_aarch64_activate_root(uint64_t root) {
    AARCH64_ASM("msr ttbr0_el1, %0" :: "r"(root) : "memory");
    ghostos_aarch64_invalidate_all();
    AARCH64_ASM("isb" ::: "memory");
}

void ghostos_aarch64_invalidate_page(uint64_t address) {
    uint64_t page = address >> 12;
    AARCH64_ASM("dsb ishst\ntlbi vae1is, %0\ndsb ish\nisb" :: "r"(page) : "memory");
}

void ghostos_aarch64_invalidate_all(void) { AARCH64_ASM("dsb ishst\ntlbi vmalle1is\ndsb ish\nisb" ::: "memory"); }

void ghostos_aarch64_invalidate_range(uint64_t start, uint64_t length) {
    if (UINT64_MAX - start < length) return;
    uint64_t end = start + length;
    for (uint64_t address = start; address < end;) {
        ghostos_aarch64_invalidate_page(address);
        if (UINT64_MAX - address < UINT64_C(4096)) break;
        address += UINT64_C(4096);
    }
}

void ghostos_aarch64_interrupts_disable(void) { AARCH64_ASM("msr daifset, #2" ::: "memory"); }
void ghostos_aarch64_interrupts_enable(void) { AARCH64_ASM("msr daifclr, #2" ::: "memory"); }

extern const uint8_t ghostos_aarch64_vectors[];

bool ghostos_aarch64_interrupts_init(void) {
    uintptr_t address = (uintptr_t)ghostos_aarch64_vectors;
    AARCH64_ASM("msr vbar_el1, %0\nisb" :: "r"(address) : "memory");
    return true;
}

uint8_t ghostos_aarch64_current_cpu(void) {
    uint64_t id;
    AARCH64_ASM("mrs %0, mpidr_el1" : "=r"(id));
    uint8_t cpu = (uint8_t)(id & 0xff);
    return cpu < 128 ? cpu : 0;
}

bool ghostos_aarch64_send_ipi(uint8_t target, uint8_t vector) { (void)target; (void)vector; return false; }
void ghostos_aarch64_end_of_interrupt(void) {}

ghostos_aarch64_register_state ghostos_aarch64_capture_registers(uint64_t fault_address) {
    ghostos_aarch64_register_state s;
    AARCH64_ASM("mov %0, x0" : "=r"(s.general[0]));
    AARCH64_ASM("mov %0, x1" : "=r"(s.general[1]));
    AARCH64_ASM("mov %0, x2" : "=r"(s.general[2]));
    AARCH64_ASM("mov %0, x3" : "=r"(s.general[3]));
    for (size_t i = 4; i < 16; ++i) s.general[i] = 0;
    AARCH64_ASM("mov %0, sp" : "=r"(s.stack_pointer));
    AARCH64_ASM("adr %0, ." : "=r"(s.instruction_pointer));
    AARCH64_ASM("mrs %0, spsr_el1" : "=r"(s.flags));
    s.fault_address = fault_address;
    return s;
}

_Noreturn void ghostos_aarch64_enter_user(ghostos_aarch64_user_context context, uint64_t root) {
    ghostos_aarch64_activate_root(root);
    __asm__ volatile("msr sp_el0, %0\nmsr elr_el1, %1\nmsr spsr_el1, xzr\nisb\neret" :: "r"(context.stack_pointer), "r"(context.instruction_pointer) : "memory");
    __builtin_unreachable();
}

void ghostos_aarch64_idle(void) { AARCH64_ASM("wfi" ::: "memory"); }
_Noreturn void ghostos_aarch64_halt(void) { for (;;) AARCH64_ASM("wfi" ::: "memory"); }

__asm__(
    ".section .text\n"
    ".balign 2048\n"
    ".global ghostos_aarch64_vectors\n"
    "ghostos_aarch64_vectors:\n"
    ".rept 16\n"
    "b ghostos_aarch64_exception_entry\n"
    ".space 124\n"
    ".endr\n"
    ".balign 16\n"
    "ghostos_aarch64_exception_entry:\n"
    "sub sp, sp, #288\n"
    "stp x0, x1, [sp, #0]\n stp x2, x3, [sp, #16]\n stp x4, x5, [sp, #32]\n stp x6, x7, [sp, #48]\n"
    "stp x8, x9, [sp, #64]\n stp x10, x11, [sp, #80]\n stp x12, x13, [sp, #96]\n stp x14, x15, [sp, #112]\n"
    "stp x16, x17, [sp, #128]\n stp x18, x19, [sp, #144]\n stp x20, x21, [sp, #160]\n stp x22, x23, [sp, #176]\n"
    "stp x24, x25, [sp, #192]\n stp x26, x27, [sp, #208]\n stp x28, x29, [sp, #224]\n str x30, [sp, #240]\n"
    "mrs x16, elr_el1\n str x16, [sp, #248]\n mrs x16, spsr_el1\n str x16, [sp, #256]\n"
    "mrs x16, esr_el1\n str x16, [sp, #264]\n mrs x16, far_el1\n str x16, [sp, #272]\n"
    "mov x0, sp\n bl ghostos_aarch64_exception_dispatch\n ldr x16, [sp, #248]\n msr elr_el1, x16\n"
    "ldr x16, [sp, #256]\n msr spsr_el1, x16\n ldp x0, x1, [sp, #0]\n ldp x2, x3, [sp, #16]\n"
    "ldp x4, x5, [sp, #32]\n ldp x6, x7, [sp, #48]\n ldp x8, x9, [sp, #64]\n ldp x10, x11, [sp, #80]\n"
    "ldp x12, x13, [sp, #96]\n ldp x14, x15, [sp, #112]\n ldp x16, x17, [sp, #128]\n"
    "ldp x18, x19, [sp, #144]\n ldp x20, x21, [sp, #160]\n ldp x22, x23, [sp, #176]\n"
    "ldp x24, x25, [sp, #192]\n ldp x26, x27, [sp, #208]\n ldp x28, x29, [sp, #224]\n"
    "ldr x30, [sp, #240]\n add sp, sp, #288\n eret\n"
);

#else

bool ghostos_aarch64_install_root(const uint64_t tables[GHOSTOS_AARCH64_TABLE_FRAME_COUNT], uint64_t offset) { (void)tables; (void)offset; return false; }
void ghostos_aarch64_activate_root(uint64_t root) { (void)root; }
void ghostos_aarch64_invalidate_page(uint64_t address) { (void)address; }
void ghostos_aarch64_invalidate_all(void) {}
void ghostos_aarch64_invalidate_range(uint64_t start, uint64_t length) { (void)start; (void)length; }
void ghostos_aarch64_interrupts_disable(void) {}
void ghostos_aarch64_interrupts_enable(void) {}
bool ghostos_aarch64_interrupts_init(void) { return false; }
uint8_t ghostos_aarch64_current_cpu(void) { return 0; }
bool ghostos_aarch64_send_ipi(uint8_t target, uint8_t vector) { (void)target; (void)vector; return false; }
void ghostos_aarch64_end_of_interrupt(void) {}
ghostos_aarch64_register_state ghostos_aarch64_capture_registers(uint64_t fault_address) { ghostos_aarch64_register_state s = {{0},0,0,0,fault_address}; return s; }
_Noreturn void ghostos_aarch64_enter_user(ghostos_aarch64_user_context context, uint64_t root) { (void)context; (void)root; for (;;) atomic_signal_fence(memory_order_seq_cst); }
void ghostos_aarch64_idle(void) {}
_Noreturn void ghostos_aarch64_halt(void) { for (;;) atomic_signal_fence(memory_order_seq_cst); }

#endif
