typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

u64 __stack_chk_guard __attribute__((section(".stack_guard"))) = 0;

void *memset(void *destination, int value, u64 count)
{
    u8 *bytes = (u8 *)destination;
    while (count != 0) {
        *bytes++ = (u8)value;
        count--;
    }
    return destination;
}

__attribute__((noreturn))
void __stack_chk_fail(void)
{
    __asm__ volatile("ud2");
    for (;;) {}
}

enum {
    ABI_VERSION = 1,
    OP_CLOCK_NOW = 2,
    OP_RANDOM_GET = 23,
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    OP_REALTIME_NOW = 46,
    OP_SLEEP_UNTIL = 47,
    SERVICE_STATE = 0x0000008000013000ULL,
    SERVICE_RESOURCE_STATE = SERVICE_STATE + 8,
    SERVICE_RESOURCE_MAGIC = 0x53594e4f44525653ULL,
    SERVICE_RESOURCE_VERSION = 1,
};

struct request {
    u16 operation;
    u16 abi_version;
    u16 flags;
    u16 reserved;
    u64 capability;
    u64 arguments[6];
};

struct response {
    u32 status;
    u32 flags;
    u64 values[4];
};

struct service_resource {
    u32 kind;
    u32 reserved;
    u64 capability;
    u64 dma_capability;
    u64 physical;
    u64 virtual_address;
    u64 length;
};

struct service_resource_manifest {
    u64 magic;
    u32 version;
    u32 count;
    struct service_resource resources[16];
};

static struct response call(u16 operation, u64 first, u64 second)
{
    struct request request = {0};
    struct response response = {0};
    request.operation = operation;
    request.abi_version = ABI_VERSION;
    request.arguments[0] = first;
    request.arguments[1] = second;
    __asm__ volatile("int $0x80" : : "D"(&request), "S"(&response) : "rax", "memory");
    return response;
}

__attribute__((section(".text._start"), noreturn))
void _start(void)
{
    volatile const u8 *state = (volatile const u8 *)SERVICE_STATE;
    volatile const struct service_resource_manifest *resources =
        (volatile const struct service_resource_manifest *)SERVICE_RESOURCE_STATE;
    u64 role = *state;
    u64 heartbeat = 0;
    u64 random_probe = 0;

    if (resources->magic != SERVICE_RESOURCE_MAGIC
        || resources->version != SERVICE_RESOURCE_VERSION
        || resources->count > 16) {
        resources = (volatile const struct service_resource_manifest *)0;
    }

    call(OP_SERVICE_READY, role, 0);
    call(OP_REALTIME_NOW, 0, 0);
    call(OP_RANDOM_GET, (u64)&random_probe, sizeof(random_probe));
    for (;;) {
        struct response clock = call(OP_CLOCK_NOW, 0, 0);
        call(OP_SLEEP_UNTIL, clock.values[0] + 10000, 0);
        heartbeat++;
        if (resources != 0 && resources->count != 0) {
            heartbeat += resources->count;
        }
        call(OP_SERVICE_HEARTBEAT, role, heartbeat);
    }
}
