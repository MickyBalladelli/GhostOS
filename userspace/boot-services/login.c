typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

u64 __stack_chk_guard __attribute__((section(".stack_guard"))) = 0;

__attribute__((noreturn))
void __stack_chk_fail(void)
{
    __asm__ volatile("ud2");
    for (;;) {}
}

enum {
    ABI_VERSION = 1,
    OP_CLOCK_NOW = 2,
    OP_SLEEP_UNTIL = 47,
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    OP_TERMINAL_READ = 24,
    OP_TERMINAL_WRITE = 25,
    OP_LOGIN_COMPLETE = 50,
    OP_LOGIN_STATUS = 51,
    LOGIN_ROLE = 14,
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

static struct response call(u16 operation, u64 first, u64 second,
                            u64 third, u64 fourth)
{
    struct request request = {0};
    struct response response = {0};
    request.operation = operation;
    request.abi_version = ABI_VERSION;
    request.arguments[0] = first;
    request.arguments[1] = second;
    request.arguments[2] = third;
    request.arguments[3] = fourth;
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

static void write_bytes(const char *bytes, u64 count)
{
    while (count != 0) {
        u64 chunk = count > 4096 ? 4096 : count;
        call(OP_TERMINAL_WRITE, (u64)bytes, chunk, 0, 0);
        bytes += chunk;
        count -= chunk;
    }
}

static void write_text(const char *text)
{
    write_bytes(text, length(text));
}

static int read_byte(u8 *byte)
{
    struct response response = call(OP_TERMINAL_READ, (u64)byte, 1, 0, 0);
    return response.status == 0 && response.values[0] == 1;
}

static void idle(void)
{
    struct response clock = call(OP_CLOCK_NOW, 0, 0, 0, 0);
    if (clock.status == 0) {
        call(OP_SERVICE_HEARTBEAT, LOGIN_ROLE, 1, 0, 0);
        call(OP_SLEEP_UNTIL, clock.values[0] + 1000, 0, 0, 0);
    }
}

static int login_requested(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0);
    return response.status == 0 && response.values[0] != 0;
}

static int administrator_account_exists(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0);
    return response.status == 0 && response.values[1] != 0;
}

static void clear_bytes(char *bytes, u64 capacity)
{
    volatile u8 *target = (volatile u8 *)bytes;
    while (capacity != 0) {
        *target = 0;
        target++;
        capacity--;
    }
}

static u64 read_line(char *line, u64 capacity, int echo)
{
    u64 count = 0;
    u8 byte;
    for (;;) {
        if (!read_byte(&byte)) {
            idle();
            continue;
        }
        if (byte == '\r' || byte == '\n') {
            line[count] = 0;
            return count;
        }
        if (byte == 8 || byte == 127) {
            if (count != 0) {
                count--;
                if (echo) {
                    write_text("\b \b");
                }
            }
            continue;
        }
        if (byte < 32 || byte >= 127 || count + 1 >= capacity) {
            continue;
        }
        line[count++] = (char)byte;
        if (echo) {
            write_bytes((const char *)&byte, 1);
        }
    }
}

static u64 read_private_line(char *line, u64 capacity)
{
    return read_line(line, capacity, 0);
}

__attribute__((section(".text._start"), noreturn))
void _start(void)
{
    char username[128];
    char credential[256];

    call(OP_SERVICE_READY, LOGIN_ROLE, 0, 0, 0);
    write_text("SynOS login service\n");
    write_text("The terminal is locked until login completes.\n");
    if (!administrator_account_exists()) {
        write_text("No administrator account exists.\n");
        write_text("Complete first-boot administrator setup before logging in.\n");
    }

    for (;;) {
        while (!login_requested()) {
            idle();
        }
        write_text("login: ");
        u64 username_length = read_line(username, sizeof(username), 1);
        write_text("\ncredential: ");
        u64 credential_length = read_private_line(credential, sizeof(credential));
        write_text("\n");

        if (username_length == 0 || credential_length == 0) {
            write_text("Login failed. Try again.\n");
            clear_bytes(username, sizeof(username));
            clear_bytes(credential, sizeof(credential));
            continue;
        }

        struct response completed = call(
            OP_LOGIN_COMPLETE,
            username_length,
            credential_length,
            0,
            0
        );
        clear_bytes(username, sizeof(username));
        clear_bytes(credential, sizeof(credential));
        if (completed.status != 0) {
            write_text("Login failed. Try again.\n");
            continue;
        }
        write_text("Login accepted.\n");
    }
}
