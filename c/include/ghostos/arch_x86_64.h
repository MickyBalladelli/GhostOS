#ifndef GHOSTOS_ARCH_X86_64_H
#define GHOSTOS_ARCH_X86_64_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_X86_64_TABLE_FRAME_COUNT 6
#define GHOSTOS_X86_64_PROCESS_TABLE_FRAME_COUNT 9
#define GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT 19
#define GHOSTOS_X86_64_SERVICE_STACK_PAGE_COUNT 16
#define GHOSTOS_X86_64_SERVICE_PAGE_COUNT 35
#define GHOSTOS_X86_64_SERVICE_IMAGE_BYTES (GHOSTOS_X86_64_SERVICE_CODE_PAGE_COUNT * 4096u)
#define GHOSTOS_X86_64_RESOURCE_STATE_OFFSET 8u
#define GHOSTOS_X86_64_MMIO_STRIDE UINT64_C(0x10000)
#define GHOSTOS_X86_64_RESCHEDULE_VECTOR UINT8_C(0xf0)
#define GHOSTOS_X86_64_TLB_VECTOR UINT8_C(0xf1)

typedef struct { uint64_t start, length; } ghostos_x86_physical_range;
typedef struct { ghostos_x86_physical_range physical; uint64_t virtual_address; } ghostos_x86_mmio_mapping;
typedef struct { uint64_t general[16], instruction_pointer, stack_pointer, flags, fault_address; } ghostos_x86_register_state;
typedef struct {
    uint64_t r15,r14,r13,r12,r11,r10,r9,r8,rbp,rdi,rsi,rdx,rcx,rbx,rax;
    uint64_t vector,error_code,rip,cs,rflags,rsp,ss;
} ghostos_x86_interrupt_frame;
typedef struct { uintptr_t stack_pointer, instruction_pointer; } ghostos_x86_user_context;
typedef enum { GHOSTOS_X86_IDLE_C0, GHOSTOS_X86_IDLE_C1, GHOSTOS_X86_IDLE_C2, GHOSTOS_X86_IDLE_C3 } ghostos_x86_idle_state;
typedef enum { GHOSTOS_X86_SERVICE_OK, GHOSTOS_X86_SERVICE_INVALID, GHOSTOS_X86_SERVICE_UNSUPPORTED } ghostos_x86_service_error;
typedef uint64_t (*ghostos_x86_interrupt_dispatcher)(void *context,uint64_t vector,uint64_t error_code,ghostos_x86_interrupt_frame *frame);
typedef void (*ghostos_x86_cpu_online_callback)(void *context,uint8_t cpu);
typedef bool (*ghostos_x86_random_u64)(void *context,uint64_t *value);
typedef void (*ghostos_x86_fatal_callback)(void *context,uint32_t status);

void ghostos_x86_enable_supervisor_protections(void);
uintptr_t ghostos_x86_with_user_access(uintptr_t (*operation)(void *),void *context);
bool ghostos_x86_supports_no_execute(void);
bool ghostos_x86_install_root(const uint64_t tables[GHOSTOS_X86_64_TABLE_FRAME_COUNT],uint64_t physical_offset);
void ghostos_x86_activate_root(uint64_t root);
void ghostos_x86_invalidate_page(uint64_t address);
void ghostos_x86_invalidate_all(void);
void ghostos_x86_invalidate_range(uint64_t start,uint64_t length);
bool ghostos_x86_install_service_root(const uint64_t tables[GHOSTOS_X86_64_PROCESS_TABLE_FRAME_COUNT],const uint64_t pages[GHOSTOS_X86_64_SERVICE_PAGE_COUNT],uint64_t physical_offset,const ghostos_x86_mmio_mapping *mappings,size_t mapping_count,uint64_t *root_out);
uint64_t ghostos_x86_service_entry(void);
uint64_t ghostos_x86_service_stack_top(void);
bool ghostos_x86_service_user_range(uint64_t address,uint64_t length,bool writable);
ghostos_x86_service_error ghostos_x86_write_service_image(const uint64_t pages[GHOSTOS_X86_64_SERVICE_PAGE_COUNT],uint64_t physical_offset,uint8_t role,const uint8_t *image,size_t image_length,ghostos_x86_random_u64 random,void *random_context,ghostos_x86_fatal_callback fatal,void *fatal_context);
void ghostos_x86_write_service_resources(const uint64_t pages[GHOSTOS_X86_64_SERVICE_PAGE_COUNT],uint64_t physical_offset,const void *manifest,size_t manifest_size);
void ghostos_x86_set_core_isolated(uint8_t cpu,bool isolated);
bool ghostos_x86_core_isolated(uint8_t cpu);
bool ghostos_x86_interrupts_init(uint8_t syscall_vector);
uint8_t ghostos_x86_current_cpu(void);
bool ghostos_x86_send_ipi(uint8_t target,uint8_t vector);
size_t ghostos_x86_start_application_processors(const uint32_t *targets,size_t count,uint8_t startup_vector,ghostos_x86_cpu_online_callback online,void *context);
void ghostos_x86_end_of_interrupt(void);
void ghostos_x86_interrupts_disable(void);
void ghostos_x86_interrupts_enable(void);
void ghostos_x86_set_interrupt_dispatcher(ghostos_x86_interrupt_dispatcher dispatcher,void *context);
uint64_t ghostos_x86_interrupt_dispatch(uint64_t vector,uint64_t error_code,ghostos_x86_interrupt_frame *frame);
_Noreturn void ghostos_x86_enter_user(ghostos_x86_user_context context,uint64_t root);
void ghostos_x86_halt(void);
void ghostos_x86_idle(ghostos_x86_idle_state state);
ghostos_x86_register_state ghostos_x86_capture_registers(uint64_t fault_address);

#endif
