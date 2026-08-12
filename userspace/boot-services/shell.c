typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

enum {
    ABI_VERSION = 1,
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    OP_SHELL_POLL = 45,
    SHELL_ROLE = 9,
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
    u64 heartbeat = 0;
    u64 idle_polls = 0;

    call(OP_SERVICE_READY, SHELL_ROLE, 0);
    for (;;) {
        struct response poll = call(OP_SHELL_POLL, 0, 0);
        if (poll.values[0] != 0) {
            idle_polls = 0;
            continue;
        }
        idle_polls++;
        if (idle_polls == 256) {
            call(OP_SERVICE_HEARTBEAT, SHELL_ROLE, ++heartbeat);
            idle_polls = 0;
        }
    }
}
