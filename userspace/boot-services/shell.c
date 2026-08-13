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
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    OP_SYNFS_OPEN = 12,
    OP_SYNFS_CLOSE = 13,
    OP_SYNFS_READ = 14,
    OP_SYNFS_MKDIR = 17,
    OP_SYNFS_RMDIR = 18,
    OP_SYNFS_LIST = 20,
    OP_SYNFS_DELETE = 22,
    OP_TERMINAL_READ = 24,
    OP_TERMINAL_WRITE = 25,
    OP_LOGIN_START = 52,
    OP_SLEEP_UNTIL = 47,
    SHELL_ROLE = 9,
    OPEN_READ = 1,
    OPEN_WRITE = 2,
    OPEN_CREATE = 4,
    OPEN_EXCLUSIVE = 1 << 9,
    FLAG_RECURSIVE = 1 << 8,
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

static struct response call(u16 operation, u16 flags, u64 capability,
                            u64 first, u64 second, u64 third, u64 offset)
{
    struct request request = {0};
    struct response response = {0};
    request.operation = operation;
    request.abi_version = ABI_VERSION;
    request.flags = flags;
    request.capability = capability;
    request.arguments[0] = first;
    request.arguments[1] = second;
    request.arguments[2] = third;
    request.arguments[4] = offset;
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

static int equal_name(const char *left, const char *right)
{
    u64 index = 0;
    while (left[index] != 0 && right[index] != 0) {
        u8 a = (u8)left[index];
        u8 b = (u8)right[index];
        if (a >= 'a' && a <= 'z') {
            a = (u8)(a - ('a' - 'A'));
        }
        if (b >= 'a' && b <= 'z') {
            b = (u8)(b - ('a' - 'A'));
        }
        if (a != b) {
            return 0;
        }
        index++;
    }
    return left[index] == 0 && right[index] == 0;
}

static void write_bytes(const char *bytes, u64 count)
{
    while (count != 0) {
        u64 chunk = count > 4096 ? 4096 : count;
        call(OP_TERMINAL_WRITE, 0, 0, (u64)bytes, chunk, 0, 0);
        bytes += chunk;
        count -= chunk;
    }
}

static void write_text(const char *text)
{
    write_bytes(text, length(text));
}

static void write_hex(u32 value)
{
    char output[11] = "0x00000000";
    u32 index;
    for (index = 0; index < 8; index++) {
        u32 shift = (7 - index) * 4;
        u32 digit = (value >> shift) & 0xf;
        output[index + 2] = (char)(digit < 10 ? '0' + digit : 'a' + digit - 10);
    }
    write_bytes(output, 10);
}

static void write_status(u32 status)
{
    write_text("status=");
    write_hex(status);
    write_text("\n");
}

static int read_byte(u8 *byte)
{
    struct response response = call(OP_TERMINAL_READ, 0, 0, (u64)byte, 1, 0, 0);
    return response.status == 0 && response.values[0] == 1;
}

static void sleep_for(u64 duration_us)
{
    struct response clock = call(OP_CLOCK_NOW, 0, 0, 0, 0, 0, 0);
    if (clock.status == 0) {
        call(OP_SLEEP_UNTIL, 0, 0, clock.values[0] + duration_us, 0, 0, 0);
    }
}

static u64 next_word(char **cursor, char *word)
{
    u64 count = 0;
    while (**cursor == ' ' || **cursor == '\t') {
        (*cursor)++;
    }
    while (**cursor != 0 && **cursor != ' ' && **cursor != '\t') {
        if (count < 255) {
            word[count++] = **cursor;
        }
        (*cursor)++;
    }
    word[count] = 0;
    return count;
}

static void print_directory(char *path, u8 *buffer)
{
    u64 path_length = length(path);
    u64 continuation = 0;
    int printed = 0;
    for (;;) {
        u64 index;
        for (index = 0; index <= path_length; index++) {
            buffer[index] = (u8)path[index];
        }
        for (index = path_length + 1; index < 4096; index++) {
            buffer[index] = 0;
        }
        struct response response = call(OP_SYNFS_LIST, 0, 0, (u64)buffer, 4096, 1, continuation);
        if (response.status != 0) {
            write_status(response.status);
            return;
        }
        index = 0;
        while (index + 22 <= response.values[0]) {
            u16 name_length = (u16)buffer[index] | ((u16)buffer[index + 1] << 8);
            u64 record_length = 22 + name_length;
            if (index + record_length > response.values[0]) {
                write_text("filesystem returned a bad directory page\n");
                return;
            }
            write_bytes((const char *)&buffer[index + 22], name_length);
            write_text("\n");
            printed = 1;
            index += record_length;
        }
        continuation = response.values[1];
        if (continuation == 0) {
            break;
        }
    }
    if (!printed) {
        write_text("(empty)\n");
    }
}

static void type_file(char *path, u8 *buffer)
{
    u64 path_length = length(path);
    struct response opened = call(OP_SYNFS_OPEN, OPEN_READ, 0, (u64)path, path_length, 0, 0);
    if (opened.status != 0) {
        write_status(opened.status);
        return;
    }
    u64 offset = 0;
    for (;;) {
        struct response read = call(OP_SYNFS_READ, 0, opened.values[0], (u64)buffer, 4096, 1, offset);
        if (read.status != 0) {
            write_status(read.status);
            break;
        }
        if (read.values[0] == 0) {
            break;
        }
        write_bytes((const char *)buffer, read.values[0]);
        offset += read.values[0];
    }
    call(OP_SYNFS_CLOSE, 0, opened.values[0], 0, 0, 0, 0);
    write_text("\n");
}

static void execute_line(char *line, u8 *buffer)
{
    char command[256];
    char path[256];
    char *cursor = line;
    if (next_word(&cursor, command) == 0) {
        return;
    }
    if (equal_name(command, "DIRECTORY") || equal_name(command, "DIR")
        || equal_name(command, "LS")) {
        if (next_word(&cursor, path) == 0) {
            path[0] = '/';
            path[1] = 0;
        }
        print_directory(path, buffer);
        return;
    }
    if (equal_name(command, "LOGIN")) {
        struct response login = call(OP_LOGIN_START, 0, 0, 0, 0, 0, 0);
        if (login.status == 0) {
            write_text("Starting login...\n");
        } else {
            write_text("Login request failed\n");
            write_status(login.status);
        }
        return;
    }
    if (next_word(&cursor, path) == 0) {
        write_text("missing path\n");
        return;
    }
    u64 path_length = length(path);
    struct response response;
    if (equal_name(command, "CREATE")) {
        response = call(OP_SYNFS_OPEN, OPEN_READ | OPEN_WRITE | OPEN_CREATE | OPEN_EXCLUSIVE,
                        0, (u64)path, path_length, 0, 0);
        if (response.status == 0) {
            call(OP_SYNFS_CLOSE, 0, response.values[0], 0, 0, 0, 0);
        }
    } else if (equal_name(command, "TYPE") || equal_name(command, "CAT")) {
        type_file(path, buffer);
        return;
    } else if (equal_name(command, "MKDIR")) {
        response = call(OP_SYNFS_MKDIR, FLAG_RECURSIVE, 0, (u64)path, path_length, 0, 0);
    } else if (equal_name(command, "RMDIR") || equal_name(command, "RD")) {
        response = call(OP_SYNFS_RMDIR, 0, 0, (u64)path, path_length, 0, 0);
    } else if (equal_name(command, "DELETE") || equal_name(command, "DEL")) {
        response = call(OP_SYNFS_DELETE, 0, 0, (u64)path, path_length, 0, 0);
    } else {
        write_text("unknown filesystem command\n");
        return;
    }
    if (response.status != 0) {
        write_status(response.status);
    }
}

__attribute__((section(".text._start"), noreturn))
void _start(void)
{
    char line[512];
    u8 buffer[4096];
    u64 line_length = 0;
    u64 heartbeat = 0;
    u64 idle_polls = 0;
    u8 byte;

    call(OP_SERVICE_READY, 0, 0, SHELL_ROLE, 0, 0, 0);
    write_text("SynOS user shell\n$ ");
    for (;;) {
        if (!read_byte(&byte)) {
            sleep_for(1000);
            idle_polls++;
            if (idle_polls == 256) {
                call(OP_SERVICE_HEARTBEAT, 0, 0, SHELL_ROLE, ++heartbeat, 0, 0);
                idle_polls = 0;
            }
            continue;
        }
        idle_polls = 0;
        if (byte == '\r' || byte == '\n') {
            write_text("\n");
            line[line_length] = 0;
            execute_line(line, buffer);
            line_length = 0;
            write_text("$ ");
        } else if (byte == 8 || byte == 127) {
            if (line_length != 0) {
                line_length--;
                write_text("\b \b");
            }
        } else if (byte >= 32 && byte < 127 && line_length < sizeof(line) - 1) {
            line[line_length++] = (char)byte;
            write_bytes((const char *)&byte, 1);
        }
    }
}
