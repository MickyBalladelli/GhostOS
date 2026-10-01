#include "ghostos/arch_riscv64.h"
#include "ghostos/status.h"

#include <stdatomic.h>
#include <stddef.h>

static atomic_uint_fast64_t isolated_cores;
static ghostos_riscv64_trap_hooks trap_hooks;

void ghostos_riscv64_set_core_isolated(uint8_t cpu,bool isolated){
    if(cpu>=64)return;uint64_t bit=UINT64_C(1)<<cpu;
    if(isolated)atomic_fetch_or_explicit(&isolated_cores,bit,memory_order_release);
    else atomic_fetch_and_explicit(&isolated_cores,~bit,memory_order_release);
}
bool ghostos_riscv64_core_isolated(uint8_t cpu){return cpu<64&&(atomic_load_explicit(&isolated_cores,memory_order_acquire)&(UINT64_C(1)<<cpu))!=0;}
void ghostos_riscv64_set_trap_hooks(ghostos_riscv64_trap_hooks hooks){trap_hooks=hooks;}

void ghostos_riscv64_trap_dispatch(ghostos_riscv64_trap_frame*frame){
    bool interrupt=(frame->scause>>63)!=0;uint64_t cause=frame->scause&~(UINT64_C(1)<<63);
    if(!interrupt&&cause==8){if(trap_hooks.syscall_dispatch)trap_hooks.syscall_dispatch(trap_hooks.context,frame->registers[10],frame->registers[11]);if(UINT64_MAX-frame->sepc>=4)frame->sepc+=4;return;}
    if(trap_hooks.capture_exception){ghostos_riscv64_register_state state={{0},0,0,frame->sstatus,frame->stval};for(size_t i=0;i<4;i++)state.general[i]=frame->registers[i+1];trap_hooks.capture_exception(trap_hooks.context,state,frame->stval,GHOSTOS_STATUS_CORRUPT,(uint16_t)cause);}
    if(trap_hooks.log_exception)trap_hooks.log_exception(trap_hooks.context,cause,interrupt);
#if defined(__riscv) && (__riscv_xlen == 64) && defined(__ELF__)
    ghostos_riscv64_halt();
#else
    if(trap_hooks.halt)trap_hooks.halt(trap_hooks.context);
#endif
}

#if defined(__riscv) && (__riscv_xlen == 64) && defined(__ELF__)

#define RISCV64_ASM(...) __asm__ volatile(__VA_ARGS__)

bool ghostos_riscv64_install_root(const uint64_t tables[GHOSTOS_RISCV64_TABLE_FRAME_COUNT],uint64_t offset){
    uint64_t*table=(uint64_t*)(uintptr_t)(tables[0]+offset);for(size_t i=0;i<512;i++)table[i]=0;
    for(size_t i=0;i<4;i++){uint64_t ppn=((uint64_t)i*UINT64_C(1073741824))>>12;table[i]=(ppn<<10)|UINT64_C(1)| (UINT64_C(1)<<1)|(UINT64_C(1)<<2)|(UINT64_C(1)<<3)|(UINT64_C(1)<<6)|(UINT64_C(1)<<7);}
    uint64_t satp=(UINT64_C(8)<<60)|(tables[0]>>12);RISCV64_ASM("csrw satp, %0\nsfence.vma zero, zero"::"r"(satp):"memory");return true;
}
void ghostos_riscv64_activate_root(uint64_t root){uint64_t satp=(UINT64_C(8)<<60)|(root>>12);RISCV64_ASM("csrw satp, %0"::"r"(satp):"memory");ghostos_riscv64_invalidate_all();}
void ghostos_riscv64_invalidate_page(uint64_t address){RISCV64_ASM("sfence.vma %0, zero"::"r"(address):"memory");}
void ghostos_riscv64_invalidate_all(void){RISCV64_ASM("sfence.vma zero, zero":::"memory");}
void ghostos_riscv64_invalidate_range(uint64_t start,uint64_t length){if(UINT64_MAX-start<length)return;uint64_t end=start+length;for(uint64_t a=start;a<end;){ghostos_riscv64_invalidate_page(a);if(UINT64_MAX-a<UINT64_C(4096))break;a+=UINT64_C(4096);}}
void ghostos_riscv64_interrupts_disable(void){uint64_t mask=UINT64_C(1)<<1;RISCV64_ASM("csrc sstatus, %0"::"r"(mask):"memory");}
void ghostos_riscv64_interrupts_enable(void){uint64_t mask=UINT64_C(1)<<1;RISCV64_ASM("csrs sstatus, %0"::"r"(mask):"memory");}
extern void ghostos_riscv64_trap_entry(void);
bool ghostos_riscv64_interrupts_init(void){uintptr_t entry=(uintptr_t)ghostos_riscv64_trap_entry;RISCV64_ASM("csrw stvec, %0"::"r"(entry):"memory");return true;}
uint8_t ghostos_riscv64_current_cpu(void){uintptr_t id;RISCV64_ASM("csrr %0, mhartid":"=r"(id));return (uint8_t)(id&((uintptr_t)0x7f));}
bool ghostos_riscv64_send_ipi(uint8_t target,uint8_t vector){(void)target;(void)vector;return false;}
void ghostos_riscv64_end_of_interrupt(void){}
ghostos_riscv64_register_state ghostos_riscv64_capture_registers(uint64_t fault){ghostos_riscv64_register_state s;RISCV64_ASM("mv %0, x1":"=r"(s.general[0]));RISCV64_ASM("mv %0, x2":"=r"(s.general[1]));RISCV64_ASM("mv %0, x3":"=r"(s.general[2]));RISCV64_ASM("mv %0, x4":"=r"(s.general[3]));for(size_t i=4;i<16;i++)s.general[i]=0;RISCV64_ASM("mv %0, sp":"=r"(s.stack_pointer));RISCV64_ASM("auipc %0, 0":"=r"(s.instruction_pointer));RISCV64_ASM("csrr %0, sstatus":"=r"(s.flags));s.fault_address=fault;return s;}
_Noreturn void ghostos_riscv64_enter_user(ghostos_riscv64_user_context c,uint64_t root){ghostos_riscv64_activate_root(root);uint64_t status;RISCV64_ASM("csrr %0, sstatus":"=r"(status));status&=~(UINT64_C(1)<<8);status|=UINT64_C(1)<<5;RISCV64_ASM("csrw sepc, %0\ncsrw sstatus, %1\nmv sp, %2\nsret"::"r"(c.instruction_pointer),"r"(status),"r"(c.stack_pointer):"memory");__builtin_unreachable();}
void ghostos_riscv64_idle(void){RISCV64_ASM("wfi":::"memory");}
_Noreturn void ghostos_riscv64_halt(void){for(;;)RISCV64_ASM("wfi":::"memory");}

__asm__(
    ".section .text\n.balign 4\n.global ghostos_riscv64_trap_entry\n"
    "ghostos_riscv64_trap_entry:\naddi sp, sp, -288\n"
    "sd x0, 0(sp)\nsd x1, 8(sp)\nsd x2, 16(sp)\nsd x3, 24(sp)\nsd x4, 32(sp)\nsd x5, 40(sp)\nsd x6, 48(sp)\nsd x7, 56(sp)\n"
    "sd x8, 64(sp)\nsd x9, 72(sp)\nsd x10, 80(sp)\nsd x11, 88(sp)\nsd x12, 96(sp)\nsd x13, 104(sp)\nsd x14, 112(sp)\nsd x15, 120(sp)\n"
    "sd x16, 128(sp)\nsd x17, 136(sp)\nsd x18, 144(sp)\nsd x19, 152(sp)\nsd x20, 160(sp)\nsd x21, 168(sp)\nsd x22, 176(sp)\nsd x23, 184(sp)\n"
    "sd x24, 192(sp)\nsd x25, 200(sp)\nsd x26, 208(sp)\nsd x27, 216(sp)\nsd x28, 224(sp)\nsd x29, 232(sp)\nsd x30, 240(sp)\nsd x31, 248(sp)\n"
    "csrr t0, sepc\nsd t0, 256(sp)\ncsrr t0, sstatus\nsd t0, 264(sp)\ncsrr t0, scause\nsd t0, 272(sp)\ncsrr t0, stval\nsd t0, 280(sp)\n"
    "mv a0, sp\ncall ghostos_riscv64_trap_dispatch\nld t0, 256(sp)\ncsrw sepc, t0\nld t0, 264(sp)\ncsrw sstatus, t0\n"
    "ld x1, 8(sp)\nld x2, 16(sp)\nld x3, 24(sp)\nld x4, 32(sp)\nld x5, 40(sp)\nld x6, 48(sp)\nld x7, 56(sp)\n"
    "ld x8, 64(sp)\nld x9, 72(sp)\nld x10, 80(sp)\nld x11, 88(sp)\nld x12, 96(sp)\nld x13, 104(sp)\nld x14, 112(sp)\nld x15, 120(sp)\n"
    "ld x16, 128(sp)\nld x17, 136(sp)\nld x18, 144(sp)\nld x19, 152(sp)\nld x20, 160(sp)\nld x21, 168(sp)\nld x22, 176(sp)\nld x23, 184(sp)\n"
    "ld x24, 192(sp)\nld x25, 200(sp)\nld x26, 208(sp)\nld x27, 216(sp)\nld x28, 224(sp)\nld x29, 232(sp)\nld x30, 240(sp)\nld x31, 248(sp)\n"
    "addi sp, sp, 288\nsret\n"
);

#else

bool ghostos_riscv64_install_root(const uint64_t t[GHOSTOS_RISCV64_TABLE_FRAME_COUNT],uint64_t o){(void)t;(void)o;return false;}
void ghostos_riscv64_activate_root(uint64_t r){(void)r;}
void ghostos_riscv64_invalidate_page(uint64_t a){(void)a;}
void ghostos_riscv64_invalidate_all(void){}
void ghostos_riscv64_invalidate_range(uint64_t a,uint64_t n){(void)a;(void)n;}
void ghostos_riscv64_interrupts_disable(void){}
void ghostos_riscv64_interrupts_enable(void){}
bool ghostos_riscv64_interrupts_init(void){return false;}
uint8_t ghostos_riscv64_current_cpu(void){return 0;}
bool ghostos_riscv64_send_ipi(uint8_t t,uint8_t v){(void)t;(void)v;return false;}
void ghostos_riscv64_end_of_interrupt(void){}
ghostos_riscv64_register_state ghostos_riscv64_capture_registers(uint64_t f){ghostos_riscv64_register_state s={{0},0,0,0,f};return s;}
_Noreturn void ghostos_riscv64_enter_user(ghostos_riscv64_user_context c,uint64_t r){(void)c;(void)r;for(;;)atomic_signal_fence(memory_order_seq_cst);}
void ghostos_riscv64_idle(void){}
_Noreturn void ghostos_riscv64_halt(void){for(;;)atomic_signal_fence(memory_order_seq_cst);}

#endif
