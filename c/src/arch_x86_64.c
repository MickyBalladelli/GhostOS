#include "ghostos/arch_x86_64.h"
#include "ghostos/status.h"

#include <stdatomic.h>

static atomic_uint_fast64_t isolated_cores[2];
static ghostos_x86_interrupt_dispatcher interrupt_dispatcher;
static void *interrupt_dispatcher_context;
#if defined(__x86_64__) && defined(__ELF__)
static atomic_bool smap_enabled;
static atomic_bool apic_ready;
static atomic_uint_least32_t apic_hardware_id;
static atomic_uintptr_t apic_mmio_base = ATOMIC_VAR_INIT((uintptr_t)0xfee00000u);
#endif

#if defined(__x86_64__) && defined(__ELF__)
static uint32_t cpuid_leaf(uint32_t leaf,uint32_t subleaf,uint32_t *ebx,uint32_t *ecx,uint32_t *edx) {
#if defined(__x86_64__) && defined(__ELF__)
    uint32_t eax=leaf,b=0,c=subleaf,d=0;
    __asm__ volatile("cpuid":"+a"(eax),"=b"(b),"+c"(c),"=d"(d));
    if(ebx)*ebx=b;if(ecx)*ecx=c;if(edx)*edx=d;return eax;
#else
    (void)leaf;(void)subleaf;if(ebx)*ebx=0;if(ecx)*ecx=0;if(edx)*edx=0;return 0;
#endif
}
#endif

#if defined(__x86_64__) && defined(__ELF__)
#define X86_ASM(...) __asm__ volatile(__VA_ARGS__)
static uint64_t read_msr(uint32_t msr){uint32_t lo,hi;X86_ASM("rdmsr":"=a"(lo),"=d"(hi):"c"(msr));return ((uint64_t)hi<<32)|lo;}
static void write_msr(uint32_t msr,uint64_t value){X86_ASM("wrmsr"::"a"((uint32_t)value),"d"((uint32_t)(value>>32)),"c"(msr):"memory");}
static void outb(uint16_t port,uint8_t value){X86_ASM("outb %0, %1"::"a"(value),"Nd"(port));}
static void io_wait(void){outb(0x80,0);}
static volatile uint32_t *apic_register(uint32_t offset){uintptr_t base=atomic_load_explicit(&apic_mmio_base,memory_order_acquire);return (volatile uint32_t *)(base+offset);}
static uint32_t apic_read(uint32_t offset){return *apic_register(offset);}
static void apic_write(uint32_t offset,uint32_t value){*apic_register(offset)=value;}
static bool wait_delivery(void){for(uint32_t i=0;i<10000;i++){if((apic_read(0x300)&(1u<<12))==0)return true;X86_ASM("pause");}return (apic_read(0x300)&(1u<<12))==0;}
#endif

void ghostos_x86_enable_supervisor_protections(void){
#if defined(__x86_64__) && defined(__ELF__)
    uint32_t ebx=0;if(cpuid_leaf(0,0,0,0,0)<7)return;(void)cpuid_leaf(7,0,&ebx,0,0);uint64_t cr4;X86_ASM("mov %0, cr4":"=r"(cr4));if(ebx&(1u<<7))cr4|=UINT64_C(1)<<20;if(ebx&(1u<<20))cr4|=UINT64_C(1)<<21;X86_ASM("mov cr4, %0"::"r"(cr4):"memory");if(cr4&(UINT64_C(1)<<21))X86_ASM("clac":::"memory");atomic_store_explicit(&smap_enabled,(cr4&(UINT64_C(1)<<21))!=0,memory_order_release);
#endif
}
uintptr_t ghostos_x86_with_user_access(uintptr_t(*op)(void*),void*ctx){
#if defined(__x86_64__) && defined(__ELF__)
    bool smap=atomic_load_explicit(&smap_enabled,memory_order_acquire);if(smap)X86_ASM("stac":::"memory");uintptr_t result=op?op(ctx):0;if(smap)X86_ASM("clac":::"memory");return result;
#else
    return op?op(ctx):0;
#endif
}
bool ghostos_x86_supports_no_execute(void){
#if defined(__x86_64__) && defined(__ELF__)
    uint32_t edx=0;if(cpuid_leaf(0x80000000u,0,0,0,0)<0x80000001u)return false;(void)cpuid_leaf(0x80000001u,0,0,0,&edx);return (edx&(1u<<20))!=0;
#else
    return false;
#endif
}

#if defined(__x86_64__) && defined(__ELF__)
static bool supports_one_gib_pages(void){uint32_t edx=0;if(cpuid_leaf(0x80000000u,0,0,0,0)<0x80000001u)return false;(void)cpuid_leaf(0x80000001u,0,0,0,&edx);return (edx&(1u<<26))!=0;}
static void enable_no_execute(void){if(!ghostos_x86_supports_no_execute())return;uint64_t efer=read_msr(0xc0000080u)|(UINT64_C(1)<<11);write_msr(0xc0000080u,efer);}
#endif
static bool add_u64(uint64_t a,uint64_t b,uint64_t*out){if(UINT64_MAX-a<b)return false;*out=a+b;return true;}

bool ghostos_x86_install_root(const uint64_t tables[GHOSTOS_X86_64_TABLE_FRAME_COUNT],uint64_t offset){
#if defined(__x86_64__) && defined(__ELF__)
    enable_no_execute();uint64_t*pml4=(uint64_t*)(uintptr_t)(tables[0]+offset);uint64_t*pdpt=(uint64_t*)(uintptr_t)(tables[1]+offset);for(size_t i=0;i<512;i++){pml4[i]=0;pdpt[i]=0;}pml4[0]=tables[1]|3u;if(supports_one_gib_pages()){for(size_t i=0;i<512;i++)pdpt[i]=(uint64_t)i*UINT64_C(1073741824)|UINT64_C(0x83);}else{for(size_t g=0;g<4;g++){uint64_t*directory=(uint64_t*)(uintptr_t)(tables[g+2]+offset);for(size_t i=0;i<512;i++){uint64_t a=((uint64_t)(g*512+i))*UINT64_C(2097152);directory[i]=a|UINT64_C(0x83);}pdpt[g]=tables[g+2]|3u;}}X86_ASM("mov cr3, %0"::"r"(tables[0]):"memory");return true;
#else
    (void)tables;(void)offset;return false;
#endif
}
void ghostos_x86_activate_root(uint64_t root){
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("mov cr3, %0"::"r"(root):"memory");
#else
    (void)root;
#endif
}
void ghostos_x86_invalidate_page(uint64_t address){
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("invlpg (%0)"::"r"(address):"memory");
#else
    (void)address;
#endif
}
void ghostos_x86_invalidate_all(void){
#if defined(__x86_64__) && defined(__ELF__)
    uint64_t root;X86_ASM("mov %0, cr3":"=r"(root));X86_ASM("mov cr3, %0"::"r"(root):"memory");
#endif
}
void ghostos_x86_invalidate_range(uint64_t start,uint64_t length){uint64_t end;if(!add_u64(start,length,&end))return;for(uint64_t a=start;a<end;){ghostos_x86_invalidate_page(a);if(UINT64_MAX-a<4096)break;a+=4096;}}

#define X86_PRESENT UINT64_C(1)
#define X86_WRITABLE (UINT64_C(1)<<1)
#define X86_USER (UINT64_C(1)<<2)
#define X86_WRITE_THROUGH (UINT64_C(1)<<3)
#define X86_CACHE_DISABLE (UINT64_C(1)<<4)
#define X86_HUGE (UINT64_C(1)<<7)
#define X86_NX (UINT64_C(1)<<63)
#define X86_USER_BASE UINT64_C(0x0000008000000000)
#define X86_SERVICE_CODE X86_USER_BASE
#define X86_SERVICE_STACK_TOP (X86_USER_BASE+(uint64_t)GHOSTOS_X86_64_SERVICE_PAGE_COUNT*4096u)
#define X86_SERVICE_MMIO_BASE (((X86_SERVICE_STACK_TOP+GHOSTOS_X86_64_MMIO_STRIDE-1)/GHOSTOS_X86_64_MMIO_STRIDE)*GHOSTOS_X86_64_MMIO_STRIDE)

#if defined(__x86_64__) && defined(__ELF__)
static bool frame_valid(uint64_t f){return f&&f%4096==0;}
static bool frame_in_list(const uint64_t*a,size_t n,uint64_t f){for(size_t i=0;i<n;i++)if(a[i]==f)return true;return false;}
static bool service_mmio_overlap(uint64_t a,uint64_t n){uint64_t e;if(!n||!add_u64(a,n,&e))return true;return a<X86_SERVICE_STACK_TOP&&e>X86_USER_BASE;}
#endif

bool ghostos_x86_install_service_root(const uint64_t tables[GHOSTOS_X86_64_PROCESS_TABLE_FRAME_COUNT],const uint64_t pages[GHOSTOS_X86_64_SERVICE_PAGE_COUNT],uint64_t offset,const ghostos_x86_mmio_mapping*mappings,size_t count,uint64_t*root_out){
#if defined(__x86_64__) && defined(__ELF__)
    if(!ghostos_x86_supports_no_execute()||count>16)return false;for(size_t i=0;i<9;i++){if(!frame_valid(tables[i])||frame_in_list(tables,i,tables[i]))return false;for(size_t j=0;j<GHOSTOS_X86_64_SERVICE_PAGE_COUNT;j++)if(tables[i]==pages[j])return false;}for(size_t i=0;i<GHOSTOS_X86_64_SERVICE_PAGE_COUNT;i++){if(!frame_valid(pages[i])||frame_in_list(pages,i,pages[i]))return false;}
    for(size_t i=0;i<9;i++){uint8_t*p=(uint8_t*)(uintptr_t)(tables[i]+offset);for(size_t j=0;j<4096;j++)p[j]=0;}
    uint64_t*root=(uint64_t*)(uintptr_t)(tables[0]+offset);uint64_t*pdpt=(uint64_t*)(uintptr_t)(tables[1]+offset);root[0]=tables[1]|X86_PRESENT|X86_WRITABLE;for(size_t g=0;g<4;g++){uint64_t*d=(uint64_t*)(uintptr_t)(tables[g+2]+offset);pdpt[g]=tables[g+2]|X86_PRESENT|X86_WRITABLE;for(size_t i=0;i<512;i++)d[i]=((uint64_t)(g*512+i)*UINT64_C(2097152))|X86_PRESENT|X86_WRITABLE|X86_HUGE;}
    uint64_t*user_pdpt=(uint64_t*)(uintptr_t)(tables[6]+offset);uint64_t*user_pd=(uint64_t*)(uintptr_t)(tables[7]+offset);uint64_t*user_pt=(uint64_t*)(uintptr_t)(tables[8]+offset);root[1]=tables[6]|X86_PRESENT|X86_WRITABLE|X86_USER;user_pdpt[0]=tables[7]|X86_PRESENT|X86_WRITABLE|X86_USER;user_pd[0]=tables[8]|X86_PRESENT|X86_WRITABLE|X86_USER;uint64_t nx=ghostos_x86_supports_no_execute()?X86_NX:0;for(size_t i=0;i<GHOSTOS_X86_64_SERVICE_PAGE_COUNT;i++)user_pt[i]=pages[i]|X86_PRESENT|X86_USER|(i<GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT?0:X86_WRITABLE|nx);
    for(size_t i=0;i<count;i++){const ghostos_x86_mmio_mapping*m=&mappings[i];uint64_t va=m->virtual_address,len=m->physical.length,end;if(!len||m->physical.start%4096||len%4096||len>GHOSTOS_X86_64_MMIO_STRIDE||va%4096||va<X86_SERVICE_MMIO_BASE||service_mmio_overlap(va,len)||!add_u64(va,len,&end)||end>X86_USER_BASE+UINT64_C(2097152))return false;size_t first=(size_t)((va-X86_USER_BASE)/4096),num=(size_t)(len/4096);if(first+num>512)return false;for(size_t p=0;p<num;p++){uint64_t phys;if(!add_u64(m->physical.start,(uint64_t)p*4096,&phys))return false;user_pt[first+p]=phys|X86_PRESENT|X86_WRITABLE|X86_USER|X86_WRITE_THROUGH|X86_CACHE_DISABLE|nx;}}
    if(root_out)*root_out=tables[0];return true;
#else
    (void)tables;(void)pages;(void)offset;(void)mappings;(void)count;(void)root_out;return false;
#endif
}
uint64_t ghostos_x86_service_entry(void){return X86_SERVICE_CODE;}
uint64_t ghostos_x86_service_stack_top(void){return X86_SERVICE_STACK_TOP;}
bool ghostos_x86_service_user_range(uint64_t a,uint64_t n,bool writable){uint64_t end;if(!n||!add_u64(a,n,&end))return false;uint64_t start=writable?X86_USER_BASE+(uint64_t)GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT*4096:X86_USER_BASE;return a>=start&&end<=X86_SERVICE_STACK_TOP;}

ghostos_x86_service_error ghostos_x86_write_service_image(const uint64_t pages[GHOSTOS_X86_64_SERVICE_PAGE_COUNT],uint64_t offset,uint8_t role,const uint8_t*image,size_t length,ghostos_x86_random_u64 random,void*random_context,ghostos_x86_fatal_callback fatal,void*fatal_context){
#if defined(__x86_64__) && defined(__ELF__)
    if(!image||length>GHOSTOS_X86_64_SERVICE_IMAGE_BYTES){if(fatal)fatal(fatal_context,GHOSTOS_STATUS_INVALID_ARGUMENT);return GHOSTOS_X86_SERVICE_INVALID;}for(size_t i=0;i<GHOSTOS_X86_64_SERVICE_PAGE_COUNT;i++){uint8_t*p=(uint8_t*)(uintptr_t)(pages[i]+offset);for(size_t j=0;j<4096;j++)p[j]=0;}for(size_t i=0;i<length;i++)((uint8_t*)(uintptr_t)(pages[i/4096]+offset))[i%4096]=image[i];uint64_t guard=0;if(!random||!random(random_context,&guard)||!guard){if(fatal)fatal(fatal_context,GHOSTOS_STATUS_CORRUPT);return GHOSTOS_X86_SERVICE_UNSUPPORTED;}size_t guard_offset=GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT*4096u-8u;*(uint64_t*)(uintptr_t)(pages[guard_offset/4096]+offset+guard_offset%4096)=guard;*(uint8_t*)(uintptr_t)(pages[GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT]+offset)=role;return GHOSTOS_X86_SERVICE_OK;
#else
    (void)pages;(void)offset;(void)role;(void)image;(void)length;(void)random;(void)random_context;(void)fatal;(void)fatal_context;return GHOSTOS_X86_SERVICE_UNSUPPORTED;
#endif
}
void ghostos_x86_write_service_resources(const uint64_t pages[GHOSTOS_X86_64_SERVICE_PAGE_COUNT],uint64_t offset,const void*manifest,size_t size){
#if defined(__x86_64__) && defined(__ELF__)
    const uint8_t*s=manifest;uint8_t*d=(uint8_t*)(uintptr_t)(pages[GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT]+offset+GHOSTOS_X86_64_RESOURCE_STATE_OFFSET);if(size>4096-GHOSTOS_X86_64_RESOURCE_STATE_OFFSET)size=4096-GHOSTOS_X86_64_RESOURCE_STATE_OFFSET;for(size_t i=0;i<size;i++)d[i]=s[i];
#else
    (void)pages;(void)offset;(void)manifest;(void)size;
#endif
}

#if defined(__x86_64__) && defined(__ELF__)
typedef struct __attribute__((packed)){uint32_t reserved0;uint64_t rsp0,rsp1,rsp2,reserved1,ist[7],reserved2;uint16_t reserved3,iomap_base;} x86_tss;
typedef struct __attribute__((packed)){uint16_t limit;uint64_t base;} x86_descriptor_pointer;
typedef struct {uint16_t low,selector,options,middle;uint32_t high,reserved;} x86_idt_entry;
typedef struct __attribute__((packed)){uint16_t limit;uint64_t base;} x86_idt_pointer;
static x86_idt_entry idt[256];
static uint64_t gdt[7];
static x86_tss tss={.iomap_base=sizeof(x86_tss)};
extern const uint64_t ghostos_isr_table[256];

static void set_idt_entry(size_t i,uint64_t address,uint16_t selector,uint16_t privilege){idt[i]=(x86_idt_entry){(uint16_t)address,selector,(uint16_t)(0x8e00u|(privilege<<13)),(uint16_t)(address>>16),(uint32_t)(address>>32),0};}
static void install_gdt(void){uintptr_t rsp;X86_ASM("mov %0, rsp":"=r"(rsp));tss.rsp0=rsp;gdt[0]=0;gdt[1]=UINT64_C(0x00af9a000000ffff);gdt[2]=UINT64_C(0x00af92000000ffff);gdt[3]=UINT64_C(0x00affa000000ffff);gdt[4]=UINT64_C(0x00aff2000000ffff);uint64_t base=(uintptr_t)&tss,limit=sizeof(tss)-1;gdt[5]=(limit&0xffff)|((base&0x00ffffff)<<16)|(UINT64_C(0x89)<<40)|(((limit>>16)&0xf)<<48)|(((base>>24)&0xff)<<56);gdt[6]=base>>32;x86_descriptor_pointer p={(uint16_t)(sizeof(gdt)-1),(uintptr_t)gdt};X86_ASM("lgdt (%0)"::"r"(&p):"memory");X86_ASM("mov $0x10, %%ax\nmov %%ax, %%ds\nmov %%ax, %%es\nmov %%ax, %%ss\npushq $0x08\nleaq 1f(%%rip), %%rax\npushq %%rax\nlretq\n1:\nmov $0x28, %%ax\nltr %%ax":"=a"(rsp)::"memory");}
static void remap_pic(void){outb(0x20,0x11);io_wait();outb(0xa0,0x11);io_wait();outb(0x21,0x20);io_wait();outb(0xa1,0x28);io_wait();outb(0x21,0x04);io_wait();outb(0xa1,0x02);io_wait();outb(0x21,0x01);io_wait();outb(0xa1,0x01);io_wait();}
static void configure_pit(void){uint16_t d=11931;outb(0x43,0x36);outb(0x40,(uint8_t)d);outb(0x40,(uint8_t)(d>>8));}
static void init_local_apic(void){uint64_t base=read_msr(0x1b);if(!(base&(UINT64_C(1)<<11))||(base&(UINT64_C(1)<<10)))return;atomic_store_explicit(&apic_mmio_base,(uintptr_t)(base&UINT64_C(0xfffff000)),memory_order_release);atomic_store_explicit(&apic_hardware_id,apic_read(0x20)>>24,memory_order_release);apic_write(0xf0,0x1ff);apic_write(0x320,32|(1u<<16));apic_write(0xb0,0);atomic_store_explicit(&apic_ready,true,memory_order_release);}
#endif

void ghostos_x86_set_core_isolated(uint8_t cpu,bool isolated){size_t word=cpu/64;uint64_t bit=UINT64_C(1)<<(cpu%64);if(word>=2)return;if(isolated)atomic_fetch_or_explicit(&isolated_cores[word],bit,memory_order_relaxed);else atomic_fetch_and_explicit(&isolated_cores[word],~bit,memory_order_relaxed);}
bool ghostos_x86_core_isolated(uint8_t cpu){size_t word=cpu/64;uint64_t bit=UINT64_C(1)<<(cpu%64);return word<2&&(atomic_load_explicit(&isolated_cores[word],memory_order_relaxed)&bit)!=0;}
bool ghostos_x86_interrupts_init(uint8_t syscall_vector){
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("cli":::"memory");install_gdt();for(size_t i=0;i<256;i++)set_idt_entry(i,ghostos_isr_table[i],0x08,(uint16_t)(i==syscall_vector?3:0));remap_pic();configure_pit();init_local_apic();outb(0x21,0xfc);outb(0xa1,0xff);x86_idt_pointer p={(uint16_t)(sizeof(idt)-1),(uintptr_t)idt};X86_ASM("lidt (%0)"::"r"(&p):"memory");return true;
#else
    (void)syscall_vector;return false;
#endif
}
uint8_t ghostos_x86_current_cpu(void){
#if defined(__x86_64__) && defined(__ELF__)
    uint32_t id=atomic_load_explicit(&apic_hardware_id,memory_order_acquire);return (uint8_t)(id&0x7f);
#else
    return 0;
#endif
}
bool ghostos_x86_send_ipi(uint8_t target,uint8_t vector){
#if defined(__x86_64__) && defined(__ELF__)
    if(!atomic_load_explicit(&apic_ready,memory_order_acquire)||vector<32)return false;apic_write(0x310,(uint32_t)target<<24);apic_write(0x300,vector);return wait_delivery();
#else
    (void)target;(void)vector;return false;
#endif
}
size_t ghostos_x86_start_application_processors(const uint32_t*targets,size_t count,uint8_t startup,ghostos_x86_cpu_online_callback online,void*context){
#if defined(__x86_64__) && defined(__ELF__)
    if(!atomic_load_explicit(&apic_ready,memory_order_acquire)||!targets)return 0;size_t started=0;uint32_t current=atomic_load_explicit(&apic_hardware_id,memory_order_acquire);for(size_t i=0;i<count;i++){uint32_t id=targets[i];if(id==current)continue;apic_write(0x310,id<<24);apic_write(0x300,0x4500);if(!wait_delivery())continue;apic_write(0x300,0x4600|startup);bool first=wait_delivery();apic_write(0x300,0x4600|startup);if(first&&wait_delivery()){started++;if(online)online(context,(uint8_t)(id&0x7f));}}return started;
#else
    (void)targets;(void)count;(void)startup;(void)online;(void)context;return 0;
#endif
}
void ghostos_x86_end_of_interrupt(void){
#if defined(__x86_64__) && defined(__ELF__)
    if(atomic_load_explicit(&apic_ready,memory_order_acquire))apic_write(0xb0,0);
#endif
}
void ghostos_x86_interrupts_disable(void){
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("cli":::"memory");
#endif
}
void ghostos_x86_interrupts_enable(void){
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("sti":::"memory");
#endif
}
void ghostos_x86_set_interrupt_dispatcher(ghostos_x86_interrupt_dispatcher d,void*c){interrupt_dispatcher=d;interrupt_dispatcher_context=c;}
uint64_t ghostos_x86_interrupt_dispatch(uint64_t v,uint64_t e,ghostos_x86_interrupt_frame*f){return interrupt_dispatcher?interrupt_dispatcher(interrupt_dispatcher_context,v,e,f):0;}

_Noreturn void ghostos_x86_enter_user(ghostos_x86_user_context c,uint64_t root){
#if defined(__x86_64__) && defined(__ELF__)
    ghostos_x86_activate_root(root);uint64_t uc=0x1b,ud=0x23,flags=0x202;X86_ASM("pushq %0\npushq %1\npushq %2\npushq %3\npushq %4\niretq"::"r"(ud),"r"((uint64_t)c.stack_pointer),"r"(flags),"r"(uc),"r"((uint64_t)c.instruction_pointer):"memory");__builtin_unreachable();
#else
    (void)c;(void)root;for(;;)atomic_signal_fence(memory_order_seq_cst);
#endif
}
void ghostos_x86_halt(void){
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("hlt":::"memory");
#endif
}

#if defined(__x86_64__) && defined(__ELF__)
static bool mwait_hint(ghostos_x86_idle_state state,uint32_t*out){uint32_t max=cpuid_leaf(0,0,0,0,0),ecx=0;if(max<5)return false;(void)cpuid_leaf(1,0,0,&ecx,0);if(!(ecx&(1u<<3)))return false;uint32_t edx=0;(void)cpuid_leaf(5,0,0,0,&edx);uint32_t rank=(uint32_t)state;if(((edx>>(rank*4))&0xf)==0)return false;*out=rank<<4;return true;}
#endif
void ghostos_x86_idle(ghostos_x86_idle_state state){
#if defined(__x86_64__) && defined(__ELF__)
    uint64_t flags;X86_ASM("pushfq; pop %0":"=r"(flags));bool was_enabled=(flags&(1u<<9))!=0;if(!was_enabled)X86_ASM("sti":::"memory");switch(state){case GHOSTOS_X86_IDLE_C0:X86_ASM("pause":::"memory");break;case GHOSTOS_X86_IDLE_C1:X86_ASM("hlt":::"memory");break;case GHOSTOS_X86_IDLE_C2:case GHOSTOS_X86_IDLE_C3:{uint32_t hint;if(mwait_hint(state,&hint))X86_ASM("mwait"::"a"(hint),"c"(0):"memory");else X86_ASM("hlt":::"memory");break;}}if(!was_enabled)X86_ASM("cli":::"memory");
#else
    (void)state;atomic_signal_fence(memory_order_seq_cst);
#endif
}
ghostos_x86_register_state ghostos_x86_capture_registers(uint64_t fault){
    ghostos_x86_register_state s;
#if defined(__x86_64__) && defined(__ELF__)
    X86_ASM("mov %%rax, %0":"=m"(s.general[0]));X86_ASM("mov %%rbx, %0":"=m"(s.general[1]));X86_ASM("mov %%rcx, %0":"=m"(s.general[2]));X86_ASM("mov %%rdx, %0":"=m"(s.general[3]));X86_ASM("mov %%rsi, %0":"=m"(s.general[4]));X86_ASM("mov %%rdi, %0":"=m"(s.general[5]));X86_ASM("mov %%rbp, %0":"=m"(s.general[6]));X86_ASM("mov %%r8, %0":"=m"(s.general[7]));X86_ASM("mov %%r9, %0":"=m"(s.general[8]));X86_ASM("mov %%r10, %0":"=m"(s.general[9]));X86_ASM("mov %%r11, %0":"=m"(s.general[10]));X86_ASM("mov %%r12, %0":"=m"(s.general[11]));X86_ASM("mov %%r13, %0":"=m"(s.general[12]));X86_ASM("mov %%r14, %0":"=m"(s.general[13]));X86_ASM("mov %%r15, %0":"=m"(s.general[14]));s.general[15]=0;X86_ASM("lea (%%rip), %0":"=r"(s.instruction_pointer));X86_ASM("mov %%rsp, %0":"=r"(s.stack_pointer));X86_ASM("pushfq; pop %0":"=r"(s.flags));
#else
    for(size_t i=0;i<16;i++)s.general[i]=0;s.instruction_pointer=0;s.stack_pointer=0;s.flags=0;
#endif
    s.fault_address=fault;return s;
}

#if defined(__x86_64__) && defined(__ELF__)
__asm__(
    ".intel_syntax noprefix\n.section .text\n.code64\n.altmacro\n.extern ghostos_x86_interrupt_dispatch\n"
    ".macro ISR_NOERR vector\n.global ghostos_isr_\\vector\nghostos_isr_\\vector:\n push 0\n push \\vector\n jmp ghostos_isr_common\n.endm\n"
    ".macro ISR_ERR vector\n.global ghostos_isr_\\vector\nghostos_isr_\\vector:\n push \\vector\n jmp ghostos_isr_common\n.endm\n"
    ".set vector, 0\n.rept 256\n.if vector == 8 || (vector >= 10 && vector <= 14) || vector == 17 || vector == 21 || vector == 29 || vector == 30\nISR_ERR %vector\n.else\nISR_NOERR %vector\n.endif\n.set vector, vector+1\n.endr\n"
    "ghostos_isr_common:\ncld\npush rax\npush rbx\npush rcx\npush rdx\npush rsi\npush rdi\npush rbp\npush r8\npush r9\npush r10\npush r11\npush r12\npush r13\npush r14\npush r15\n"
    "mov rdi, [rsp+120]\nmov rsi, [rsp+128]\nmov rdx, rsp\ncall ghostos_x86_interrupt_dispatch\nmov [rsp+120], rax\npop r15\npop r14\npop r13\npop r12\npop r11\npop r10\npop r9\npop r8\npop rbp\npop rdi\npop rsi\npop rdx\npop rcx\npop rbx\npop rax\ncmp qword ptr [rsp], 0\njne 1f\nadd rsp, 16\niretq\n1:\nadd rsp, 16\nmov rcx, [rsp]\nmov rdx, [rsp+24]\nmov rsp, rdx\njmp rcx\n"
    ".section .rodata\n.align 8\n.global ghostos_isr_table\nghostos_isr_table:\n.macro ISR_TABLE_ENTRY vector\n.quad ghostos_isr_\\vector\n.endm\n.set vector,0\n.rept 256\nISR_TABLE_ENTRY %vector\n.set vector,vector+1\n.endr\n.att_syntax prefix\n"
);
#endif
