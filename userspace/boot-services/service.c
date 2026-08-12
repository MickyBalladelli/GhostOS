typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

enum {
    ABI_VERSION = 1,
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    SERVICE_STATE = 0x0000008000004000ULL,
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

static void call(u16 operation, u64 first, u64 second)
{
    struct request request = {0};
    struct response response = {0};
    request.operation = operation;
    request.abi_version = ABI_VERSION;
    request.arguments[0] = first;
    request.arguments[1] = second;
    __asm__ volatile("int $0x80" : : "D"(&request), "S"(&response) : "rax", "memory");
}

__attribute__((section(".text._start"), noreturn))
void _start(void)
{
    volatile const u8 *state = (volatile const u8 *)SERVICE_STATE;
    u64 role = *state;
    u64 heartbeat = 0;

    call(OP_SERVICE_READY, role, 0);
    for (;;) {
        heartbeat++;
        call(OP_SERVICE_HEARTBEAT, role, heartbeat);
    }
}
