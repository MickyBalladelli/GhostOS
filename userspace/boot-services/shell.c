typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

enum {
    ABI_VERSION = 1,
    OP_YIELD = 1,
    OP_TERMINAL_READ = 24,
    OP_TERMINAL_WRITE = 25,
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    OP_SYSTEM_INFO = 44,
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

static u64 length(const char *text)
{
    u64 count = 0;
    while (text[count] != 0) {
        count++;
    }
    return count;
}

static void write_bytes(const char *text, u64 count)
{
    call(OP_TERMINAL_WRITE, (u64)text, count);
}

static void write_text(const char *text)
{
    write_bytes(text, length(text));
}

static u8 upper(u8 byte)
{
    if (byte >= 'a' && byte <= 'z') {
        return (u8)(byte - ('a' - 'A'));
    }
    return byte;
}

static int equals(const char *line, u64 count, const char *command)
{
    u64 index = 0;
    while (command[index] != 0) {
        if (index >= count || upper((u8)line[index]) != (u8)command[index]) {
            return 0;
        }
        index++;
    }
    return index == count;
}

static void write_u64(u64 value)
{
    char digits[20];
    u64 count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value != 0);
    for (u64 left = 0, right = count - 1; left < right; left++, right--) {
        char swap = digits[left];
        digits[left] = digits[right];
        digits[right] = swap;
    }
    write_bytes(digits, count);
}

static void show_services(void)
{
    struct response info = call(OP_SYSTEM_INFO, 0, 0);
    u64 ready = info.values[1];
#define SHOW_SERVICE(role, name) \
    write_text("  " name); \
    write_text((ready & (1ULL << (role))) != 0 ? "  READY\r\n" : "  STARTING\r\n")
    SHOW_SERVICE(1, "synos-init");
    SHOW_SERVICE(2, "synos-fsd");
    SHOW_SERVICE(3, "synos-storaged");
    SHOW_SERVICE(4, "synos-netd");
    SHOW_SERVICE(5, "synos-logd");
    SHOW_SERVICE(6, "synos-auditd");
    SHOW_SERVICE(7, "synos-authd");
    SHOW_SERVICE(8, "synos-pkgd");
    SHOW_SERVICE(9, "synos-shell");
#undef SHOW_SERVICE
}

static void execute(const char *line, u64 count)
{
    while (count != 0 && line[count - 1] == ' ') {
        count--;
    }
    if (count == 0) {
        return;
    }
    if (equals(line, count, "HELP")) {
        write_text("Ring 3 shell commands:\r\n"
                   "  HELP\r\n"
                   "  SHOW-SYSTEM\r\n"
                   "  SHOW-SERVICES\r\n"
                   "  UPTIME\r\n");
        return;
    }
    if (equals(line, count, "SHOW-SERVICES")) {
        show_services();
        return;
    }
    if (equals(line, count, "UPTIME")) {
        struct response info = call(OP_SYSTEM_INFO, 0, 0);
        write_text("uptime-us=");
        write_u64(info.values[0]);
        write_text("\r\n");
        return;
    }
    if (equals(line, count, "SHOW-SYSTEM")) {
        struct response info = call(OP_SYSTEM_INFO, 0, 0);
        write_text("SynOS microkernel\r\n"
                   "shell-mode=Ring 3\r\n"
                   "shell-address-space=");
        write_u64(info.values[3]);
        write_text("\r\nservice-heartbeats=");
        write_u64(info.values[2]);
        write_text("\r\n");
        return;
    }
    write_text("unknown command; type HELP\r\n");
}

__attribute__((section(".text._start"), noreturn))
void _start(void)
{
    char line[128];
    u64 count = 0;
    u64 heartbeat = 0;

    call(OP_SERVICE_READY, SHELL_ROLE, 0);
    write_text("SynOS Ring 3 service shell ready\r\nsynos> ");
    for (;;) {
        char byte = 0;
        struct response input = call(OP_TERMINAL_READ, (u64)&byte, 1);
        if (input.values[0] == 0) {
            call(OP_SERVICE_HEARTBEAT, SHELL_ROLE, ++heartbeat);
            call(OP_YIELD, 0, 0);
            continue;
        }
        if (byte == '\n') {
            continue;
        }
        if (byte == '\r') {
            write_text("\r\n");
            execute(line, count);
            count = 0;
            write_text("synos> ");
            continue;
        }
        if (byte == 8 || byte == 127) {
            if (count != 0) {
                count--;
                write_text("\b \b");
            }
            continue;
        }
        if ((u8)byte >= 32 && (u8)byte < 127 && count < sizeof(line)) {
            line[count++] = byte;
            write_bytes(&byte, 1);
        }
    }
}
