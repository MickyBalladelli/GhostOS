#ifndef GHOSTOS_ADDRESS_SPACE_H
#define GHOSTOS_ADDRESS_SPACE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_PAGE_SIZE UINT64_C(4096)
#define GHOSTOS_KERNEL_SPACE_START UINT64_C(0)
#define GHOSTOS_KERNEL_SPACE_END UINT64_C(0x100000000)
#define GHOSTOS_USER_SPACE_START UINT64_C(0x0000008000000000)
#define GHOSTOS_USER_SPACE_END UINT64_C(0x00007ffffffff000)
#define GHOSTOS_MAX_ADDRESS_SPACE_REGIONS 64
#define GHOSTOS_MAX_ADDRESS_SPACES 64

typedef enum {
    GHOSTOS_AS_OK = 0, GHOSTOS_AS_CAPACITY, GHOSTOS_AS_INVALID_ROOT,
    GHOSTOS_AS_INVALID_REQUEST, GHOSTOS_AS_INVALID_MAPPING,
    GHOSTOS_AS_MAPPING_OVERFLOW, GHOSTOS_AS_MAPPING_CONFLICT,
    GHOSTOS_AS_NOT_FOUND, GHOSTOS_AS_INVALID_CONTEXT,
    GHOSTOS_AS_NOT_COPY_ON_WRITE
} ghostos_as_error;

typedef enum { GHOSTOS_ACCESS_READ, GHOSTOS_ACCESS_WRITE, GHOSTOS_ACCESS_EXECUTE } ghostos_memory_access;
typedef enum { GHOSTOS_ISOLATION_OK, GHOSTOS_ISOLATION_NOT_FOUND, GHOSTOS_ISOLATION_SAME_SPACE,
    GHOSTOS_ISOLATION_SHARED_TABLE, GHOSTOS_ISOLATION_SHARED_BACKING } ghostos_isolation_error;
typedef enum { GHOSTOS_STACK_OK, GHOSTOS_STACK_NOT_GUARD, GHOSTOS_STACK_COLLISION,
    GHOSTOS_STACK_OVERFLOW, GHOSTOS_STACK_CAPACITY } ghostos_stack_error;

typedef struct { uint64_t base, size; } ghostos_mapping;
typedef struct { uint64_t size, alignment, preferred_base; bool has_preferred_base, fixed; } ghostos_mapping_request;
typedef struct { uint64_t address, file_size, memory_size; uint8_t permissions; } ghostos_runtime_segment;
#ifndef GHOSTOS_PHYSICAL_RANGE_TYPE_DEFINED
#define GHOSTOS_PHYSICAL_RANGE_TYPE_DEFINED
typedef struct { uint64_t start, length; } ghostos_physical_range;
#endif
typedef struct { uint64_t frame; } ghostos_page_table_root;
typedef struct { uint64_t page, new_stack_base, new_guard_base; bool has_new_guard_base; } ghostos_stack_growth;
typedef struct { uint64_t entry, stack_pointer, tls_pointer, heap_base, heap_size; bool has_tls_pointer; } ghostos_process_context;

enum { GHOSTOS_PERM_READ = 1, GHOSTOS_PERM_WRITE = 2, GHOSTOS_PERM_EXECUTE = 4 };

typedef struct {
    bool used, has_permissions, has_backing, copy_on_write, guard, stack;
    ghostos_mapping owner;
    uint64_t base, size, authority;
    uint8_t permissions;
    ghostos_physical_range backing;
} ghostos_as_region;

typedef struct {
    uint32_t id;
    ghostos_page_table_root root;
    uint64_t aslr_state;
    ghostos_as_region regions[GHOSTOS_MAX_ADDRESS_SPACE_REGIONS];
} ghostos_address_space;

typedef struct {
    bool used;
    ghostos_address_space space;
} ghostos_as_slot;

typedef struct { ghostos_as_slot slots[GHOSTOS_MAX_ADDRESS_SPACES]; } ghostos_address_space_table;

bool ghostos_as_is_user_range(uint64_t address, uint64_t size);
bool ghostos_as_is_kernel_range(uint64_t address, uint64_t size);
bool ghostos_as_page_table_root(uint64_t frame, ghostos_page_table_root *out);
void ghostos_as_init(ghostos_address_space *space, uint32_t id, ghostos_page_table_root root, uint64_t aslr_seed);
bool ghostos_as_can_access(const ghostos_address_space *space, uint64_t address, uint64_t size, ghostos_memory_access access);
ghostos_as_error ghostos_as_reserve(ghostos_address_space *space, ghostos_mapping_request request, ghostos_mapping *out);
ghostos_as_error ghostos_as_claim(ghostos_address_space *space, ghostos_mapping mapping);
ghostos_as_error ghostos_as_map_backing(ghostos_address_space *space, uint64_t authority, ghostos_physical_range backing, bool writable, ghostos_mapping *out);
ghostos_as_error ghostos_as_record_stack(ghostos_address_space *space, ghostos_mapping mapping, uint64_t base, uint64_t size, uint8_t guard_pages);
ghostos_stack_error ghostos_as_stack_growth_page(const ghostos_address_space *space, uint64_t address, uint64_t *page);
ghostos_stack_error ghostos_as_grow_stack(ghostos_address_space *space, uint64_t address, ghostos_stack_growth *out);
ghostos_as_error ghostos_as_clone_cow(ghostos_address_space *parent, uint32_t child_id, ghostos_page_table_root child_root, ghostos_address_space *child);
ghostos_as_error ghostos_as_cow_mapping(const ghostos_address_space *space, uint64_t address, uint64_t *page, uint64_t *frame);
ghostos_as_error ghostos_as_can_replace_cow_page(const ghostos_address_space *space, uint64_t address);
ghostos_as_error ghostos_as_replace_cow_page(ghostos_address_space *space, uint64_t address, uint64_t new_frame, ghostos_physical_range *old_frame);
ghostos_as_error ghostos_as_unmap_backing(ghostos_address_space *space, uint64_t authority, uint64_t address, uint64_t length, ghostos_physical_range *out);
ghostos_as_error ghostos_as_map_segment(ghostos_address_space *space, ghostos_mapping mapping, ghostos_runtime_segment segment, size_t source_length);
ghostos_as_error ghostos_as_zero_fill(const ghostos_address_space *space, ghostos_mapping mapping, uint64_t address, uint64_t length);
ghostos_as_error ghostos_as_relocate(const ghostos_address_space *space, ghostos_mapping mapping, uint64_t address);
ghostos_as_error ghostos_as_protect(ghostos_address_space *space, ghostos_mapping mapping, uint64_t address, uint64_t length, uint8_t permissions);
ghostos_as_error ghostos_as_record_region(ghostos_address_space *space, ghostos_mapping mapping, uint64_t base, uint64_t size, uint8_t permissions);
ghostos_as_error ghostos_as_install_context(const ghostos_address_space *space, ghostos_process_context context);
void ghostos_as_release(ghostos_address_space *space, ghostos_mapping mapping);
void ghostos_as_table_init(ghostos_address_space_table *table);
ghostos_as_error ghostos_as_table_create(ghostos_address_space_table *table, uint32_t id, ghostos_page_table_root root, uint64_t aslr_seed);
ghostos_address_space *ghostos_as_table_get(ghostos_address_space_table *table, uint32_t id);
const ghostos_address_space *ghostos_as_table_get_const(const ghostos_address_space_table *table, uint32_t id);
ghostos_isolation_error ghostos_as_table_check_isolation(const ghostos_address_space_table *table, uint32_t first, uint32_t second);
ghostos_as_error ghostos_as_table_destroy(ghostos_address_space_table *table, uint32_t id);

#endif
