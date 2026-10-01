#include "ghostos/arch.h"

#include <stdatomic.h>

ghostos_architecture_evidence ghostos_arch_evidence(void) {
#if defined(__x86_64__) && defined(__ELF__)
    return (ghostos_architecture_evidence){"x86_64",true,true,true,true};
#elif defined(__aarch64__) && defined(__ELF__)
    return (ghostos_architecture_evidence){"aarch64",false,true,true,true};
#elif defined(__riscv) && (__riscv_xlen == 64) && defined(__ELF__)
    return (ghostos_architecture_evidence){"riscv64",false,true,true,true};
#else
    return (ghostos_architecture_evidence){"unsupported",false,false,false,false};
#endif
}

bool ghostos_arch_initialize(const ghostos_arch_backend *b,ghostos_arch_validate_tables validate,void *validate_context,const uint64_t tables[GHOSTOS_ARCH_TABLE_FRAME_COUNT],uint64_t offset) {
    if(!b||!b->install_root||!b->init_interrupts||!validate||!validate(validate_context,tables,offset))return false;
    if(!b->install_root(b->context,tables,offset))return false;
    if(b->x86_supervisor_protections){if(!b->enable_supervisor_protections)return false;b->enable_supervisor_protections(b->context);}
    b->init_interrupts(b->context);
    return true;
}

void ghostos_arch_idle(const ghostos_arch_backend*b,uint8_t state){if(b&&b->idle)b->idle(b->context,state);}
void ghostos_arch_disable_interrupts(const ghostos_arch_backend*b){if(b&&b->disable_interrupts)b->disable_interrupts(b->context);}
void ghostos_arch_enable_interrupts(const ghostos_arch_backend*b){if(b&&b->enable_interrupts)b->enable_interrupts(b->context);}
void ghostos_arch_invalidate_tlb_range(const ghostos_arch_backend*b,uint64_t start,uint64_t length){if(b&&b->invalidate_range)b->invalidate_range(b->context,start,length);}
uintptr_t ghostos_arch_with_user_access(const ghostos_arch_backend*b,ghostos_arch_operation op,void*ctx){if(!op)return 0;return b&&b->with_user_access?b->with_user_access(b->context,op,ctx):op(ctx);}
bool ghostos_arch_ring3_supported(const ghostos_arch_backend*b){return b&&b->ring3_supported?b->ring3_supported(b->context):true;}

typedef struct { const void *source; void *destination; size_t size; } copy_context;
static uintptr_t copy_bytes(void*context){copy_context*c=context;unsigned char*d=c->destination;const unsigned char*s=c->source;for(size_t i=0;i<c->size;i++)d[i]=s[i];return 1;}
static bool valid_copy(const void*user,const void*local,size_t size,size_t alignment){if(!alignment||(alignment&(alignment-1))||(!user&&size)||(!local&&size))return false;return !size||((uintptr_t)user&(alignment-1))==0;}

bool ghostos_arch_read_user(const ghostos_arch_backend*b,const void*user,void*out,size_t size,size_t alignment){if(!valid_copy(user,out,size,alignment))return false;copy_context c={user,out,size};return ghostos_arch_with_user_access(b,copy_bytes,&c)!=0;}
bool ghostos_arch_write_user(const ghostos_arch_backend*b,void*user,const void*input,size_t size,size_t alignment){if(!valid_copy(user,input,size,alignment))return false;copy_context c={input,user,size};return ghostos_arch_with_user_access(b,copy_bytes,&c)!=0;}

_Noreturn void ghostos_arch_enter_user(const ghostos_arch_backend*b,uintptr_t stack,uintptr_t instruction,uint64_t root){if(b&&b->enter_user)b->enter_user(b->context,stack,instruction,root);ghostos_arch_halt(b);}
_Noreturn void ghostos_arch_halt(const ghostos_arch_backend*b){if(b&&b->halt)b->halt(b->context);for(;;)atomic_signal_fence(memory_order_seq_cst);}
