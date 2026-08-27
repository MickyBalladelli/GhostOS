typedef unsigned char u8;
typedef unsigned short u16;
typedef unsigned int u32;
typedef unsigned long long u64;

#define STATUS_NORMAL 0x00010009U

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
    OP_SERVICE_READY = 42,
    OP_SERVICE_HEARTBEAT = 43,
    OP_SYSTEM_INFO = 44,
    OP_GHOSTFS_OPEN = 12,
    OP_GHOSTFS_CLOSE = 13,
    OP_GHOSTFS_READ = 14,
    OP_GHOSTFS_WRITE = 15,
    OP_GHOSTFS_MKDIR = 17,
    OP_GHOSTFS_RMDIR = 18,
    OP_GHOSTFS_LIST = 20,
    OP_GHOSTFS_DELETE = 22,
    OP_TERMINAL_READ = 24,
    OP_TERMINAL_WRITE = 25,
    OP_LOGIN_STATUS = 51,
    OP_LOGIN_START = 52,
    OP_LOGIN_BOOTSTRAP_USERNAME = 58,
    OP_LOGIN_BOOTSTRAP_CREDENTIAL = 59,
    OP_LOGIN_BOOTSTRAP_CONFIRM = 60,
    OP_LOGIN_BOOTSTRAP_RECOVERY = 61,
    OP_LOGIN_REVOKE_IDENTITY = 62,
    OP_SHUTDOWN = 63,
    OP_WATCHDOG_DIAGNOSTICS = 64,
    OP_LOGIN_BRIDGE_READ = 65,
    OP_LOGIN_LOGOUT = 56,
    OP_LOGIN_WHOAMI = 57,
    OP_SLEEP_UNTIL = 47,
    SHELL_ROLE = 9,
    SHELL_REQUIRED_SERVICES = (1u << 2) | (1u << 3) | (1u << 4)
        | (1u << 5) | (1u << 6) | (1u << 7) | (1u << 8)
        | (1u << 10) | (1u << 13) | (1u << 14),
    OPEN_READ = 1,
    OPEN_WRITE = 2,
    OPEN_CREATE = 4,
    OPEN_EXCLUSIVE = 1 << 9,
    FLAG_RECURSIVE = 1 << 8,
    SHELL_POLL_DELAY_US = 100,
};

static const char ACCOUNT_AUTHORIZATION_PATH[] = "/system/security/authorization";
static int bridge_transport_active = 0;
enum {
    ACCOUNT_RECORD_LEGACY_VERSION = 1,
    ACCOUNT_RECORD_VERSION = 2,
    ACCOUNT_USERNAME_CAPACITY = 32,
    ACCOUNT_CREDENTIAL_CAPACITY = 96,
    ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES = 3,
    ACCOUNT_CREDENTIAL_MAX = 4,
    ACCOUNT_RECORD_HEADER_BYTES = 36,
    ACCOUNT_DATABASE_CAPACITY = 4096,
    ACCOUNT_RECORD_PENDING = 0,
    ACCOUNT_RECORD_DELETED = 4,
    ACCOUNT_RECORD_DISABLED_OFFSET = 4,
    ACCOUNT_RECORD_LOCKED_OFFSET = 8,
    ACCOUNT_RECORD_EXPIRED_OFFSET = 12,
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

__attribute__((noinline))
static struct response call_ex(u16 operation, u16 flags, u64 capability,
                               u64 first, u64 second, u64 third, u64 offset,
                               u64 extra)
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
    request.arguments[5] = extra;
    __asm__ volatile(
        "int $0x80"
        : "+m"(request), "+m"(response)
        : "D"(&request), "S"(&response)
        : "rax", "rcx", "rdx", "r8", "r9", "r10", "r11", "cc", "memory"
    );
    return response;
}

__attribute__((noinline))
static struct response call(u16 operation, u16 flags, u64 capability,
                            u64 first, u64 second, u64 third, u64 offset)
{
    return call_ex(operation, flags, capability, first, second, third, offset, 0);
}

static u64 length(const char *text)
{
    u64 count = 0;
    while (text[count] != 0) {
        count++;
    }
    return count;
}

__attribute__((noinline))
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

__attribute__((noinline))
static int command_named(const char *command, u64 command_length, const char *name, u64 name_length)
{
    return command_length == name_length && equal_name(command, name);
}

static void write_text(const char *text);

static int make_absolute_path(char *path)
{
    u64 path_length;
    u64 index;
    if (path[0] == '/') {
        return 0;
    }
    path_length = length(path);
    if (path_length == 0 || path_length >= 255) {
        return -1;
    }
    index = path_length;
    while (index != 0) {
        path[index] = path[index - 1];
        index--;
    }
    path[0] = '/';
    path[path_length + 1] = 0;
    return 0;
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

static int require_absolute_path(char *path)
{
    if (make_absolute_path(path) != 0) {
        write_text("the path is invalid. Check its spelling and format.\n");
        return -1;
    }
    return 0;
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
    u32 facility = (status >> 16) & 0xfff;
    u32 code = (status >> 3) & 0x1fff;
    write_text("Reason: ");
    if (facility == 1 && code == 3) {
        write_text("the request was not valid.");
    } else if (facility == 1 && code == 4) {
        write_text("the requested item was not found.");
    } else if (facility == 1 && code == 5) {
        write_text("the filesystem has no free space.");
    } else if (facility == 3 && code == 2) {
        write_text("that item already exists.");
    } else if (facility == 3 && code == 5) {
        write_text("the directory is not empty.");
    } else if (facility == 3 && code == 6) {
        write_text("the path is invalid. Check its spelling and format.");
    } else if (facility == 3 && code == 7) {
        write_text("the path does not name a directory.");
    } else if (facility == 3 && code == 8) {
        write_text("the filesystem is read-only.");
    } else if (facility == 7 && code == 1) {
        write_text("access was denied.");
    } else {
        write_text("the system rejected the request.");
    }
    write_text("\n");
}

static int read_byte(u8 *byte)
{
    struct response response = call(OP_TERMINAL_READ, 0, 0, (u64)byte, 1, 0, 0);
    return response.status == STATUS_NORMAL && response.values[0] == 1;
}

static int read_bridge_byte(u8 *byte)
{
    struct response response = call(OP_LOGIN_BRIDGE_READ, 0, 0, (u64)byte, 1, 0, 0);
    if (response.status == STATUS_NORMAL && response.values[0] == 1) {
        bridge_transport_active = response.values[1] != 0;
    }
    return response.status == STATUS_NORMAL && response.values[0] == 1;
}

static int login_authorized(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0, 0, 0);
    return response.status == STATUS_NORMAL && response.values[3] != 0;
}

static int first_run_mode(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0, 0, 0);
    return response.status == STATUS_NORMAL && response.values[1] == 0;
}

static int shell_ready(void)
{
    struct response response = call(OP_SERVICE_READY, 0, 0, SHELL_ROLE, 0, 0, 0);
    return response.status == STATUS_NORMAL;
}

static int shell_dependencies_ready(void)
{
    struct response response = call(OP_SYSTEM_INFO, 0, 0, 0, 0, 0, 0);
    return response.status == STATUS_NORMAL
        && (response.values[1] & SHELL_REQUIRED_SERVICES) == SHELL_REQUIRED_SERVICES;
}

static void write_prompt(int authorized, int first_run)
{
    if (authorized) {
        write_text("$ ");
    } else if (first_run) {
        write_text("\x1b[1;33mGHOSTOS\x1b[90m::\x1b[35mFIRST-RUN\x1b[0m> ");
    } else {
        write_text("\x1b[1;33mGHOSTOS\x1b[90m::\x1b[31mLOCKED\x1b[0m> ");
    }
}

static int update_prompt(int *prompt_authorized, int *prompt_first_run)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0, 0, 0);
    if (response.status != STATUS_NORMAL) {
        return *prompt_authorized > 0 || *prompt_first_run > 0;
    }
    int authorized = response.values[3] != 0;
    int first_run = response.values[1] == 0;
    if (*prompt_authorized != authorized || *prompt_first_run != first_run) {
        if (*prompt_authorized >= 0 || *prompt_first_run >= 0) {
            write_text("\n");
        }
        write_prompt(authorized, first_run);
        *prompt_authorized = authorized;
        *prompt_first_run = first_run;
    }
    return authorized || first_run;
}

static void sleep_for(u64 duration_us)
{
    struct response clock = call(OP_CLOCK_NOW, 0, 0, 0, 0, 0, 0);
    if (clock.status == STATUS_NORMAL) {
        call(OP_SLEEP_UNTIL, 0, 0, clock.values[0] + duration_us, 0, 0, 0);
    }
}

__attribute__((noinline))
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

static void watchdog_command(char *cursor)
{
    char action[256];
    u64 action_length = next_word(&cursor, action);
    u64 mode = 0;
    if (action_length != 0 && equal_name(action, "ON")) {
        mode = 1;
    } else if (action_length != 0 && equal_name(action, "OFF")) {
        mode = 2;
    } else if (action_length != 0 && !equal_name(action, "STATUS")) {
        write_text("Use: WATCHDOG STATUS|ON|OFF\n");
        return;
    }
    struct response response = call(OP_WATCHDOG_DIAGNOSTICS, 0, 0, mode, 0, 0, 0);
    if (response.status != STATUS_NORMAL) {
        write_text("Watchdog request failed\n");
        write_status(response.status);
        return;
    }
    write_text("Watchdog diagnostics: ");
    write_text(response.values[0] != 0 ? "on" : "off");
    write_text(" (timeout=");
    write_hex((u32)(response.values[1] / 1000000));
    write_text("s)\n");
}

__attribute__((noinline))
static void print_directory(const char *path, u8 *buffer)
{
    char prefix[256];
    u64 path_length = length(path);
    u64 continuation = 0;
    u64 index;
    if (path_length == 0 || path_length >= sizeof(prefix)) {
        prefix[0] = '/';
        prefix[1] = 0;
        path_length = 1;
    } else {
        for (index = 0; index <= path_length; index++) {
            prefix[index] = path[index];
        }
    }
    for (;;) {
        volatile u8 *out = buffer;
        memset(buffer, 0, 4096);
        for (index = 0; index <= path_length; index++) {
            out[index] = (u8)prefix[index];
        }
        struct response response = call_ex(
            OP_GHOSTFS_LIST, 0, 0, (u64)buffer, 4096, 1, continuation, path_length
        );
        if (response.status != STATUS_NORMAL) {
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
            if (name_length != 0) {
                write_bytes((const char *)&buffer[index + 22], name_length);
                write_text("\n");
            }
            index += record_length;
        }
        continuation = response.values[1];
        if (continuation == 0) {
            break;
        }
    }
}

static void type_file(char *path, u8 *buffer)
{
    u64 path_length = length(path);
    struct response opened = call(OP_GHOSTFS_OPEN, OPEN_READ, 0, (u64)path, path_length, 0, 0);
    if (opened.status != STATUS_NORMAL) {
        write_status(opened.status);
        return;
    }
    u64 offset = 0;
    for (;;) {
        struct response read = call(OP_GHOSTFS_READ, 0, opened.values[0], (u64)buffer, 4096, 1, offset);
        if (read.status != STATUS_NORMAL) {
            write_status(read.status);
            break;
        }
        if (read.values[0] == 0) {
            break;
        }
        write_bytes((const char *)buffer, read.values[0]);
        offset += read.values[0];
    }
    call(OP_GHOSTFS_CLOSE, 0, opened.values[0], 0, 0, 0, 0);
    write_text("\n");
}

static void print_help(void)
{
    write_text("GhostOS commands:\n");
    write_text("  HELP, ?, COMMANDS       Show this list\n");
    write_text("  DIRECTORY|DIR|LS [path] List a directory\n");
    write_text("  CREATE <path>           Create a file\n");
    write_text("  TYPE|CAT <path>         Read a file\n");
    write_text("  MKDIR <path>            Create a directory\n");
    write_text("  RMDIR|RD <path>         Remove a directory\n");
    write_text("  DELETE|DEL <path>       Delete a file\n");
    write_text("  WHOAMI                  Show logged-in user\n");
    write_text("  LOGIN                   Start login\n");
    write_text("  LOGOUT                  Lock the shell\n");
    write_text("  CREDENTIAL ...          Manage credentials\n");
    write_text("  ACCOUNT ...             Manage accounts\n");
    write_text("  WATCHDOG STATUS|ON|OFF  Watchdog diagnostics\n");
    write_text("  SHUTDOWN                Power off\n");
}

static int valid_account_username(const u8 *username, u64 username_length)
{
    u64 index;
    static const char *reserved[] = {
        ".", "..", "account", "anonymous", "daemon", "guest", "kernel",
        "nobody", "operator", "root", "service", "system"
    };
    if (username_length == 0 || username_length > ACCOUNT_USERNAME_CAPACITY) {
        return 0;
    }
    for (index = 0; index < username_length; index++) {
        u8 byte = username[index];
        if (!(byte >= 'a' && byte <= 'z')
            && !(byte >= 'A' && byte <= 'Z')
            && !(byte >= '0' && byte <= '9')
            && byte != '.' && byte != '_' && byte != '-' && byte != '$') {
            return 0;
        }
    }
    for (index = 0; index < sizeof(reserved) / sizeof(reserved[0]); index++) {
        u64 reserved_length = length(reserved[index]);
        u64 reserved_index;
        if (reserved_length != username_length) {
            continue;
        }
        for (reserved_index = 0; reserved_index < reserved_length; reserved_index++) {
            u8 byte = username[reserved_index];
            if (byte >= 'A' && byte <= 'Z') {
                byte = (u8)(byte + ('a' - 'A'));
            }
            if (byte != (u8)reserved[index][reserved_index]) {
                break;
            }
        }
        if (reserved_index == reserved_length) {
            return 0;
        }
    }
    return 1;
}

static void normalize_account_username(
    const char *username,
    u64 username_length,
    u8 *normalized
)
{
    u64 index;
    for (index = 0; index < username_length; index++) {
        u8 byte = (u8)username[index];
        if (byte >= 'A' && byte <= 'Z') {
            byte = (u8)(byte + ('a' - 'A'));
        }
        normalized[index] = byte;
    }
}

static int revoke_account_sessions(const char *username, u64 username_length)
{
    u8 normalized[ACCOUNT_USERNAME_CAPACITY] = {0};
    normalize_account_username(username, username_length, normalized);
    struct response response = call(
        OP_LOGIN_REVOKE_IDENTITY,
        0,
        0,
        (u64)normalized,
        username_length,
        0,
        0
    );
    return response.status == STATUS_NORMAL;
}

static const char *account_credential_name(u8 kind)
{
    if (kind >= ACCOUNT_RECORD_EXPIRED_OFFSET) {
        kind = (u8)(kind - ACCOUNT_RECORD_EXPIRED_OFFSET);
    } else if (kind >= ACCOUNT_RECORD_LOCKED_OFFSET) {
        kind = (u8)(kind - ACCOUNT_RECORD_LOCKED_OFFSET);
    } else if (kind > ACCOUNT_RECORD_DISABLED_OFFSET) {
        kind = (u8)(kind - ACCOUNT_RECORD_DISABLED_OFFSET);
    }
    if (kind == 1) {
        return "passkey";
    }
    if (kind == 2) {
        return "tpm";
    }
    if (kind == 3) {
        return "ssh";
    }
    return 0;
}

static int account_record_is_enabled(u8 kind)
{
    return kind >= 1 && kind <= 3;
}

static int account_record_is_disabled(u8 kind)
{
    return kind >= ACCOUNT_RECORD_DISABLED_OFFSET + 1
        && kind <= ACCOUNT_RECORD_DISABLED_OFFSET + 3;
}

static int account_record_is_locked(u8 kind)
{
    return kind >= ACCOUNT_RECORD_LOCKED_OFFSET + 1
        && kind <= ACCOUNT_RECORD_LOCKED_OFFSET + 3;
}

static int account_record_is_expired(u8 kind)
{
    return kind >= ACCOUNT_RECORD_EXPIRED_OFFSET + 1
        && kind <= ACCOUNT_RECORD_EXPIRED_OFFSET + 3;
}

static int account_record_has_credential(u8 kind)
{
    return account_record_is_enabled(kind)
        || account_record_is_disabled(kind)
        || account_record_is_locked(kind)
        || account_record_is_expired(kind);
}

static u8 account_credential_kind(u8 kind)
{
    if (kind >= ACCOUNT_RECORD_EXPIRED_OFFSET) {
        return (u8)(kind - ACCOUNT_RECORD_EXPIRED_OFFSET);
    }
    if (kind >= ACCOUNT_RECORD_LOCKED_OFFSET) {
        return (u8)(kind - ACCOUNT_RECORD_LOCKED_OFFSET);
    }
    if (kind > ACCOUNT_RECORD_DISABLED_OFFSET) {
        return (u8)(kind - ACCOUNT_RECORD_DISABLED_OFFSET);
    }
    return kind;
}

static u8 account_state_offset(u8 kind)
{
    if (kind >= ACCOUNT_RECORD_EXPIRED_OFFSET) {
        return ACCOUNT_RECORD_EXPIRED_OFFSET;
    }
    if (kind >= ACCOUNT_RECORD_LOCKED_OFFSET) {
        return ACCOUNT_RECORD_LOCKED_OFFSET;
    }
    if (kind > ACCOUNT_RECORD_DISABLED_OFFSET) {
        return ACCOUNT_RECORD_DISABLED_OFFSET;
    }
    return 0;
}

static const char *account_state_name(u8 kind)
{
    if (kind == ACCOUNT_RECORD_PENDING) {
        return "pending";
    }
    if (account_record_is_disabled(kind)) {
        return "disabled";
    }
    if (account_record_is_locked(kind)) {
        return "locked";
    }
    if (account_record_is_expired(kind)) {
        return "expired";
    }
    return "active";
}

static u64 account_record_credential_count(const u8 *record)
{
    if (record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_PENDING
        || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
        return 0;
    }
    if (record[0] == ACCOUNT_RECORD_LEGACY_VERSION) {
        return 1;
    }
    if (record[ACCOUNT_RECORD_HEADER_BYTES] == 0
        || record[ACCOUNT_RECORD_HEADER_BYTES] > ACCOUNT_CREDENTIAL_MAX) {
        return 0;
    }
    return record[ACCOUNT_RECORD_HEADER_BYTES];
}

static int account_record_credential(
    const u8 *record,
    u64 index,
    u8 *id,
    u8 *kind,
    const u8 **material,
    u64 *material_length
)
{
    if (record[0] == ACCOUNT_RECORD_LEGACY_VERSION) {
        if (index != 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_PENDING) {
            return 0;
        }
        *id = 1;
        *kind = account_credential_kind(record[ACCOUNT_USERNAME_CAPACITY + 2]);
        *material = record + ACCOUNT_RECORD_HEADER_BYTES;
        *material_length = record[ACCOUNT_RECORD_HEADER_BYTES - 1];
        return 1;
    }
    u64 count = account_record_credential_count(record);
    u64 offset = ACCOUNT_RECORD_HEADER_BYTES + 1;
    for (u64 entry = 0; entry < count; entry++) {
        u8 entry_length = record[offset + 2];
        if (entry == index) {
            *id = record[offset];
            *kind = record[offset + 1];
            *material = record + offset + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES;
            *material_length = entry_length;
            return 1;
        }
        offset += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + entry_length;
    }
    return 0;
}

static int account_username_bytes_match(
    const u8 *left,
    u64 left_length,
    const u8 *right,
    u64 right_length
)
{
    u64 index;
    if (left_length != right_length) {
        return 0;
    }
    for (index = 0; index < left_length; index++) {
        u8 left_byte = left[index];
        u8 right_byte = right[index];
        if (left_byte >= 'a' && left_byte <= 'z') {
            left_byte = (u8)(left_byte - ('a' - 'A'));
        }
        if (right_byte >= 'a' && right_byte <= 'z') {
            right_byte = (u8)(right_byte - ('a' - 'A'));
        }
        if (left_byte != right_byte) {
            return 0;
        }
    }
    return 1;
}

static int account_username_matches(
    const u8 *stored,
    u64 stored_length,
    const char *requested
)
{
    return account_username_bytes_match(
        stored,
        stored_length,
        (const u8 *)requested,
        length(requested)
    );
}

static int valid_account_record(const u8 *buffer, u64 record_bytes)
{
    u64 username_length = buffer[1];
    u64 credential_length = buffer[ACCOUNT_RECORD_HEADER_BYTES - 1];
    if (record_bytes < ACCOUNT_RECORD_HEADER_BYTES
        || record_bytes > ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY
        || (buffer[0] != ACCOUNT_RECORD_LEGACY_VERSION
            && buffer[0] != ACCOUNT_RECORD_VERSION)
        || !valid_account_username(buffer + 2, username_length)) {
        return 0;
    }
    if (buffer[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_PENDING
        || buffer[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
        return credential_length == 0 && record_bytes == ACCOUNT_RECORD_HEADER_BYTES;
    }
    if (account_credential_name(buffer[ACCOUNT_USERNAME_CAPACITY + 2]) == 0
        || credential_length == 0
        || credential_length > ACCOUNT_CREDENTIAL_CAPACITY
        || record_bytes != ACCOUNT_RECORD_HEADER_BYTES + credential_length) {
        return 0;
    }
    if (buffer[0] == ACCOUNT_RECORD_LEGACY_VERSION) {
        return 1;
    }
    u64 count = buffer[ACCOUNT_RECORD_HEADER_BYTES];
    u64 offset = ACCOUNT_RECORD_HEADER_BYTES + 1;
    u8 first_kind = 0;
    if (count == 0 || count > ACCOUNT_CREDENTIAL_MAX) {
        return 0;
    }
    for (u64 index = 0; index < count; index++) {
        if (offset + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES > record_bytes
            || buffer[offset] == 0
            || account_credential_name(buffer[offset + 1]) == 0
            || buffer[offset + 2] == 0
            || offset + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + buffer[offset + 2] > record_bytes) {
            return 0;
        }
        for (u64 previous = ACCOUNT_RECORD_HEADER_BYTES + 1;
             previous < offset;
             previous += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + buffer[previous + 2]) {
            if (buffer[previous] == buffer[offset]) {
                return 0;
            }
        }
        if (index == 0) {
            first_kind = buffer[offset + 1];
        }
        offset += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + buffer[offset + 2];
    }
    return offset == record_bytes
        && account_credential_kind(buffer[ACCOUNT_USERNAME_CAPACITY + 2]) == first_kind;
}

static u64 account_record_size(const u8 *buffer, u64 remaining)
{
    u64 credential_length;
    u64 record_bytes;
    if (remaining < ACCOUNT_RECORD_HEADER_BYTES) {
        return 0;
    }
    credential_length = buffer[ACCOUNT_RECORD_HEADER_BYTES - 1];
    record_bytes = ACCOUNT_RECORD_HEADER_BYTES + credential_length;
    if (record_bytes > remaining) {
        return 0;
    }
    return valid_account_record(buffer, record_bytes) ? record_bytes : 0;
}

static int account_record_is_latest(
    const u8 *buffer,
    u64 database_bytes,
    u64 offset,
    u64 record_bytes
)
{
    u64 next_offset = offset + record_bytes;
    while (next_offset < database_bytes) {
        const u8 *next = buffer + next_offset;
        u64 next_bytes = account_record_size(next, database_bytes - next_offset);
        if (next_bytes == 0) {
            return -1;
        }
        if (account_username_bytes_match(
            buffer + offset + 2,
            buffer[offset + 1],
            next + 2,
            next[1]
        )) {
            return 0;
        }
        next_offset += next_bytes;
    }
    return 1;
}

static int read_account_database(u8 *buffer, u64 *database_bytes)
{
    struct response opened = call(
        OP_GHOSTFS_OPEN,
        OPEN_READ,
        0,
        (u64)ACCOUNT_AUTHORIZATION_PATH,
        length(ACCOUNT_AUTHORIZATION_PATH),
        0,
        0
    );
    if (opened.status != STATUS_NORMAL) {
        return 0;
    }

    struct response read = call(
        OP_GHOSTFS_READ,
        0,
        opened.values[0],
        (u64)buffer,
        4096,
        1,
        0
    );
    call(OP_GHOSTFS_CLOSE, 0, opened.values[0], 0, 0, 0, 0);
    if (read.status != STATUS_NORMAL) {
        return -1;
    }
    *database_bytes = read.values[0];
    return *database_bytes != 0;
}

static int append_account_record(
    const u8 *record,
    u64 record_bytes,
    u64 database_bytes,
    u32 *failure_status
)
{
    struct response opened = call(
        OP_GHOSTFS_OPEN,
        OPEN_READ | OPEN_WRITE,
        0,
        (u64)ACCOUNT_AUTHORIZATION_PATH,
        length(ACCOUNT_AUTHORIZATION_PATH),
        0,
        0
    );
    if (opened.status != STATUS_NORMAL) {
        *failure_status = opened.status;
        return 0;
    }
    struct response written = call(
        OP_GHOSTFS_WRITE,
        0,
        opened.values[0],
        (u64)record,
        record_bytes,
        0,
        database_bytes
    );
    call(OP_GHOSTFS_CLOSE, 0, opened.values[0], 0, 0, 0, 0);
    *failure_status = written.status;
    return written.status == STATUS_NORMAL && written.values[0] == record_bytes;
}

static void list_accounts(u8 *buffer)
{
    u64 database_bytes = 0;
    u64 offset = 0;
    u64 count = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Account list unavailable.\n");
        return;
    }
    if (read == 0) {
        write_text("No local accounts.\n");
        return;
    }
    if (database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is corrupt.\n");
        return;
    }
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        const char *state = account_state_name(record[ACCOUNT_USERNAME_CAPACITY + 2]);
        const char *credential = record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_PENDING
            ? "none"
            : account_credential_name(record[ACCOUNT_USERNAME_CAPACITY + 2]);
        write_text("username=");
        write_bytes((const char *)(record + 2), record[1]);
        write_text(" state=");
        write_text(state);
        write_text(" credential=");
        write_text(credential);
        write_text("\n");
        offset += record_bytes;
        count++;
    }
    if (count == 0) {
        write_text("No local accounts.\n");
    }
}

static void show_account(const char *username, u8 *buffer)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    u64 username_length = length(username);
    if (!valid_account_username((const u8 *)username, username_length)) {
        write_text("Invalid username.\n");
        return;
    }

    u64 database_bytes = 0;
    u64 offset = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Account details unavailable.\n");
        return;
    }
    if (read == 0) {
        write_text("Account not found.\n");
        return;
    }
    if (database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is corrupt.\n");
        return;
    }
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        if (account_username_matches(record + 2, record[1], username)) {
            write_text("username=");
            write_bytes((const char *)(record + 2), record[1]);
            write_text("\nstate=");
            write_text(account_state_name(record[ACCOUNT_USERNAME_CAPACITY + 2]));
            write_text("\nscope=local\ncredential=");
            write_text(record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_PENDING
                ? "none"
                : account_credential_name(record[ACCOUNT_USERNAME_CAPACITY + 2]));
            write_text("\n");
            return;
        }
        offset += record_bytes;
    }
    write_text("Account not found.\n");
}

static void list_credentials(const char *username, u8 *buffer)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    u64 username_length = length(username);
    if (!valid_account_username((const u8 *)username, username_length)) {
        write_text("Invalid username.\n");
        return;
    }

    u64 database_bytes = 0;
    u64 offset = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Credential list unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account not found.\n");
        return;
    }
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        if (account_username_matches(record + 2, record[1], username)) {
            write_text("username=");
            write_bytes((const char *)(record + 2), record[1]);
            write_text(" credential_count=");
            write_hex((u32)account_record_credential_count(record));
            write_text(" state=");
            write_text(account_state_name(record[ACCOUNT_USERNAME_CAPACITY + 2]));
            for (u64 index = 0; index < account_record_credential_count(record); index++) {
                u8 id = 0;
                u8 kind = 0;
                const u8 *material = 0;
                u64 material_length = 0;
                if (!account_record_credential(
                    record,
                    index,
                    &id,
                    &kind,
                    &material,
                    &material_length
                )) {
                    write_text("\nAccount database is corrupt.\n");
                    return;
                }
                (void)material;
                (void)material_length;
                write_text(" credential[");
                write_hex((u32)index);
                write_text("]=id=");
                write_hex(id);
                write_text(" kind=");
                write_text(account_credential_name(kind));
                write_text(" material=public-only");
            }
            write_text("\n");
            return;
        }
        offset += record_bytes;
    }
    write_text("Account not found.\n");
}

static u64 read_credential_line_from(char *line, u64 capacity, int echo, int bridge)
{
    u64 count = 0;
    u8 byte;
    for (;;) {
        if (!(bridge ? read_bridge_byte(&byte) : read_byte(&byte))) {
            sleep_for(SHELL_POLL_DELAY_US);
            continue;
        }
        if (bridge && count == 0 && byte == 0) {
            u8 length_low;
            u8 length_high;
            while (!read_bridge_byte(&length_low)) {
                sleep_for(SHELL_POLL_DELAY_US);
            }
            while (!read_bridge_byte(&length_high)) {
                sleep_for(SHELL_POLL_DELAY_US);
            }
            u64 framed_length = (u64)length_low | ((u64)length_high << 8);
            if (framed_length == 0 || framed_length >= capacity) {
                return 0;
            }
            while (count < framed_length) {
                while (!read_bridge_byte((u8 *)&line[count])) {
                    sleep_for(SHELL_POLL_DELAY_US);
                }
                count++;
            }
            line[count] = 0;
            write_text("\n");
            return count;
        }
        if (byte == '\r' || byte == '\n') {
            line[count] = 0;
            write_text("\n");
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
        if (byte >= 32 && byte < 127 && count + 1 < capacity) {
            line[count++] = (char)byte;
            if (echo) {
                write_bytes((const char *)&byte, 1);
            }
        }
    }
}

static u64 read_credential_line(char *line, u64 capacity)
{
    return read_credential_line_from(line, capacity, 1, 0);
}

static u64 read_private_credential_line(char *line, u64 capacity)
{
    return read_credential_line_from(line, capacity, 0, 0);
}

static u64 read_bridge_credential_line(char *line, u64 capacity)
{
    return read_credential_line_from(line, capacity, 0, 1);
}

static int hex_value(u8 byte)
{
    if (byte >= '0' && byte <= '9') {
        return (int)(byte - '0');
    }
    if (byte >= 'a' && byte <= 'f') {
        return (int)(byte - 'a' + 10);
    }
    if (byte >= 'A' && byte <= 'F') {
        return (int)(byte - 'A' + 10);
    }
    return -1;
}

static u64 decode_hex_material(
    const char *text,
    u64 text_length,
    u8 *output,
    u64 capacity
)
{
    u64 index;
    u64 output_length = 0;
    if (text_length == 0 || (text_length & 1) != 0) {
        return 0;
    }
    for (index = 0; index < text_length; index += 2) {
        int high = hex_value((u8)text[index]);
        int low = hex_value((u8)text[index + 1]);
        if (high < 0 || low < 0 || output_length >= capacity) {
            return 0;
        }
        output[output_length++] = (u8)((high << 4) | low);
    }
    return output_length;
}

static int canonical_passkey_material(const u8 *material)
{
    return material[0] == 0xa5
        && material[1] == 0x01
        && material[2] == 0x02
        && material[3] == 0x03
        && material[4] == 0x26
        && material[5] == 0x20
        && material[6] == 0x01
        && material[7] == 0x21
        && material[8] == 0x58
        && material[9] == 0x20
        && material[42] == 0x22
        && material[43] == 0x58
        && material[44] == 0x20;
}

static void add_credential(const char *username, u8 *buffer)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    u64 username_length = length(username);
    if (!valid_account_username((const u8 *)username, username_length)) {
        write_text("Invalid username.\n");
        return;
    }

    u64 database_bytes = 0;
    u64 offset = 0;
    u8 target[ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    int found = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Credential enrollment unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account not found.\n");
        return;
    }
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest != 0
            && record[ACCOUNT_USERNAME_CAPACITY + 2] != ACCOUNT_RECORD_DELETED
            && account_username_matches(record + 2, record[1], username)) {
            found = 1;
            for (u64 index = 0; index < record_bytes; index++) {
                target[index] = record[index];
            }
        }
        offset += record_bytes;
    }
    if (!found) {
        write_text("Account not found.\n");
        return;
    }
    u64 credential_count = account_record_credential_count(target);
    if (credential_count >= ACCOUNT_CREDENTIAL_MAX) {
        write_text("Account credential capacity reached.\n");
        return;
    }

    u8 ids[ACCOUNT_CREDENTIAL_MAX] = {0};
    u8 kinds[ACCOUNT_CREDENTIAL_MAX] = {0};
    u8 material_lengths[ACCOUNT_CREDENTIAL_MAX] = {0};
    const u8 *materials[ACCOUNT_CREDENTIAL_MAX] = {0};
    for (offset = 0; offset < credential_count; offset++) {
        u64 existing_material_length = 0;
        if (!account_record_credential(
            target,
            offset,
            &ids[offset],
            &kinds[offset],
            &materials[offset],
            &existing_material_length
        )) {
            write_text("Account database is corrupt.\n");
            return;
        }
        material_lengths[offset] = (u8)existing_material_length;
    }

    char kind[256];
    char material[256];
    u8 material_bytes[ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    write_text("Credential type [PASSKEY/TPM/SSH]: ");
    u64 kind_length = read_credential_line(kind, sizeof(kind));
    write_text("Credential public material: ");
    u64 material_length = read_private_credential_line(material, sizeof(material));
    u8 kind_id = 0;
    if (equal_name(kind, "PASSKEY")) {
        kind_id = 1;
    } else if (equal_name(kind, "TPM")) {
        kind_id = 2;
    } else if (equal_name(kind, "SSH")) {
        kind_id = 3;
    }
    u8 new_id = 1;
    for (u8 candidate = 1; candidate <= ACCOUNT_CREDENTIAL_MAX; candidate++) {
        int used = 0;
        for (offset = 0; offset < credential_count; offset++) {
            if (ids[offset] == candidate) {
                used = 1;
            }
        }
        if (!used) {
            new_id = candidate;
            break;
        }
    }
    u64 stored_material_length = material_length;
    if (kind_id == 1) {
        stored_material_length = decode_hex_material(
            material,
            material_length,
            material_bytes,
            sizeof(material_bytes)
        );
    } else {
        for (offset = 0; offset < material_length && offset < sizeof(material_bytes); offset++) {
            material_bytes[offset] = (u8)material[offset];
        }
    }
    u64 credential_payload = 1 + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + stored_material_length;
    for (offset = 0; offset < credential_count; offset++) {
        credential_payload += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + material_lengths[offset];
    }
    if (kind_length == 0 || stored_material_length == 0
        || credential_payload > ACCOUNT_CREDENTIAL_CAPACITY || kind_id == 0) {
        write_text("Credential rejected.\n");
        return;
    }
    if (database_bytes + ACCOUNT_RECORD_HEADER_BYTES + credential_payload > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is full.\n");
        return;
    }
    u8 next_record[ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    for (offset = 0; offset < ACCOUNT_RECORD_HEADER_BYTES; offset++) {
        next_record[offset] = target[offset];
    }
    next_record[0] = ACCOUNT_RECORD_VERSION;
    next_record[ACCOUNT_USERNAME_CAPACITY + 2] = (u8)(
        account_state_offset(target[ACCOUNT_USERNAME_CAPACITY + 2])
        + (credential_count == 0 ? kind_id : kinds[0])
    );
    next_record[ACCOUNT_RECORD_HEADER_BYTES - 1] = (u8)credential_payload;
    next_record[ACCOUNT_RECORD_HEADER_BYTES] = (u8)(credential_count + 1);
    u64 next_offset = ACCOUNT_RECORD_HEADER_BYTES + 1;
    for (offset = 0; offset < credential_count; offset++) {
        next_record[next_offset] = ids[offset];
        next_record[next_offset + 1] = kinds[offset];
        next_record[next_offset + 2] = material_lengths[offset];
        for (u64 index = 0; index < material_lengths[offset]; index++) {
            next_record[next_offset + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + index]
                = materials[offset][index];
        }
        next_offset += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + material_lengths[offset];
    }
    next_record[next_offset] = new_id;
    next_record[next_offset + 1] = kind_id;
    next_record[next_offset + 2] = (u8)stored_material_length;
    for (offset = 0; offset < stored_material_length; offset++) {
        next_record[next_offset + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + offset]
            = material_bytes[offset];
    }
    u32 failure_status = 0;
    if (!append_account_record(
        next_record,
        ACCOUNT_RECORD_HEADER_BYTES + credential_payload,
        database_bytes,
        &failure_status
    )) {
        write_text("Credential enrollment failed.\n");
        write_status(failure_status);
        return;
    }
    write_text("Credential added; account is active.\n");
}

static int parse_credential_id(const char *text, u64 text_length, u8 *id)
{
    u64 index = 0;
    u32 value = 0;
    u32 base = 10;
    if (text_length == 0) {
        return 0;
    }
    if (text_length > 2 && text[0] == '0' && (text[1] == 'x' || text[1] == 'X')) {
        index = 2;
        base = 16;
    }
    if (index == text_length) {
        return 0;
    }
    for (; index < text_length; index++) {
        u8 byte = (u8)text[index];
        u8 digit;
        if (byte >= '0' && byte <= '9') {
            digit = (u8)(byte - '0');
        } else if (byte >= 'a' && byte <= 'f') {
            digit = (u8)(byte - 'a' + 10);
        } else if (byte >= 'A' && byte <= 'F') {
            digit = (u8)(byte - 'A' + 10);
        } else {
            return 0;
        }
        if (digit >= base) {
            return 0;
        }
        value = value * base + digit;
        if (value == 0 || value > 255) {
            return 0;
        }
    }
    *id = (u8)value;
    return 1;
}

static void remove_credential(
    const char *username,
    const char *id_text,
    u8 *buffer
)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    u64 username_length = length(username);
    u64 id_length = length(id_text);
    u8 id_to_remove = 0;
    if (!valid_account_username((const u8 *)username, username_length)
        || !parse_credential_id(id_text, id_length, &id_to_remove)) {
        write_text("Invalid credential or username.\n");
        return;
    }

    u64 database_bytes = 0;
    u64 offset = 0;
    u8 target[ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    int found = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Credential removal unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account not found.\n");
        return;
    }
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest != 0
            && record[ACCOUNT_USERNAME_CAPACITY + 2] != ACCOUNT_RECORD_DELETED
            && account_username_matches(record + 2, record[1], username)) {
            found = 1;
            for (u64 index = 0; index < record_bytes; index++) {
                target[index] = record[index];
            }
        }
        offset += record_bytes;
    }
    if (!found) {
        write_text("Account not found.\n");
        return;
    }
    u64 credential_count = account_record_credential_count(target);
    if (credential_count == 0) {
        write_text("Account has no credentials.\n");
        return;
    }

    u8 ids[ACCOUNT_CREDENTIAL_MAX] = {0};
    u8 kinds[ACCOUNT_CREDENTIAL_MAX] = {0};
    u8 material_lengths[ACCOUNT_CREDENTIAL_MAX] = {0};
    const u8 *materials[ACCOUNT_CREDENTIAL_MAX] = {0};
    u64 remove_index = credential_count;
    for (offset = 0; offset < credential_count; offset++) {
        u64 material_length = 0;
        if (!account_record_credential(
            target,
            offset,
            &ids[offset],
            &kinds[offset],
            &materials[offset],
            &material_length
        )) {
            write_text("Account database is corrupt.\n");
            return;
        }
        material_lengths[offset] = (u8)material_length;
        if (ids[offset] == id_to_remove) {
            remove_index = offset;
        }
    }
    if (remove_index == credential_count) {
        write_text("Credential not found.\n");
        return;
    }
    if (credential_count <= 1) {
        write_text("Cannot remove the last credential.\n");
        return;
    }

    u64 remaining_count = credential_count - 1;
    u64 payload_bytes = 1;
    u64 first_remaining = remove_index == 0 ? 1 : 0;
    for (offset = 0; offset < credential_count; offset++) {
        if (offset != remove_index) {
            payload_bytes += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + material_lengths[offset];
        }
    }
    if (database_bytes + ACCOUNT_RECORD_HEADER_BYTES + payload_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is full.\n");
        return;
    }
    target[0] = ACCOUNT_RECORD_VERSION;
    target[ACCOUNT_USERNAME_CAPACITY + 2] = (u8)(
        account_state_offset(target[ACCOUNT_USERNAME_CAPACITY + 2])
        + kinds[first_remaining]
    );
    target[ACCOUNT_RECORD_HEADER_BYTES - 1] = (u8)payload_bytes;
    target[ACCOUNT_RECORD_HEADER_BYTES] = (u8)remaining_count;
    u64 target_offset = ACCOUNT_RECORD_HEADER_BYTES + 1;
    for (offset = 0; offset < credential_count; offset++) {
        if (offset == remove_index) {
            continue;
        }
        target[target_offset] = ids[offset];
        target[target_offset + 1] = kinds[offset];
        target[target_offset + 2] = material_lengths[offset];
        for (u64 index = 0; index < material_lengths[offset]; index++) {
            target[target_offset + ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + index]
                = materials[offset][index];
        }
        target_offset += ACCOUNT_CREDENTIAL_ENTRY_HEADER_BYTES + material_lengths[offset];
    }
    u32 failure_status = 0;
    if (!append_account_record(
        target,
        ACCOUNT_RECORD_HEADER_BYTES + payload_bytes,
        database_bytes,
        &failure_status
    )) {
        write_text("Credential removal failed.\n");
        write_status(failure_status);
        return;
    }
    write_text("Credential removed.\n");
}

static void create_account(const char *username, u8 *buffer)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    u64 username_length = length(username);
    if (!valid_account_username((const u8 *)username, username_length)) {
        write_text("Invalid username.\n");
        return;
    }

    u64 database_bytes = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Account database unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database unavailable.\n");
        return;
    }

    u64 offset = 0;
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        if (account_username_matches(record + 2, record[1], username)) {
            write_text("Account already exists.\n");
            return;
        }
        offset += record_bytes;
    }
    if (database_bytes + ACCOUNT_RECORD_HEADER_BYTES > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is full.\n");
        return;
    }

    u8 record[ACCOUNT_RECORD_HEADER_BYTES] = {0};
    u8 normalized_username[ACCOUNT_USERNAME_CAPACITY] = {0};
    normalize_account_username(username, username_length, normalized_username);
    record[0] = ACCOUNT_RECORD_VERSION;
    record[1] = (u8)username_length;
    for (offset = 0; offset < username_length; offset++) {
        record[2 + offset] = normalized_username[offset];
    }
    u32 failure_status = 0;
    if (!append_account_record(
        record,
        ACCOUNT_RECORD_HEADER_BYTES,
        database_bytes,
        &failure_status
    )) {
        write_text("Account create failed.\n");
        write_status(failure_status);
        return;
    }
    write_text("Account created in pending setup state.\n");
}

static void delete_account(
    const char *username,
    const char *confirmation,
    u8 *buffer
)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    if (!equal_name(confirmation, "CONFIRM")) {
        write_text("Confirmation required. Use: ACCOUNT DELETE <username> CONFIRM\n");
        return;
    }
    u64 username_length = length(username);
    if (!valid_account_username((const u8 *)username, username_length)) {
        write_text("Invalid username.\n");
        return;
    }

    u64 database_bytes = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Account database unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account not found.\n");
        return;
    }

    u64 offset = 0;
    u64 administrator_count = 0;
    int found = 0;
    int target_is_administrator = 0;
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        int is_administrator = account_record_is_enabled(record[ACCOUNT_USERNAME_CAPACITY + 2]);
        if (is_administrator) {
            administrator_count++;
        }
        if (account_username_matches(record + 2, record[1], username)) {
            found = 1;
            target_is_administrator = is_administrator;
        }
        offset += record_bytes;
    }
    if (!found) {
        write_text("Account not found.\n");
        return;
    }
    if (target_is_administrator && administrator_count <= 1) {
        write_text("Cannot delete the last administrator.\n");
        return;
    }
    if (database_bytes + ACCOUNT_RECORD_HEADER_BYTES > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is full.\n");
        return;
    }

    u8 tombstone[ACCOUNT_RECORD_HEADER_BYTES] = {0};
    u8 normalized_username[ACCOUNT_USERNAME_CAPACITY] = {0};
    normalize_account_username(username, username_length, normalized_username);
    tombstone[0] = ACCOUNT_RECORD_VERSION;
    tombstone[1] = (u8)username_length;
    for (offset = 0; offset < username_length; offset++) {
        tombstone[2 + offset] = normalized_username[offset];
    }
    tombstone[ACCOUNT_USERNAME_CAPACITY + 2] = ACCOUNT_RECORD_DELETED;
    u32 failure_status = 0;
    if (!append_account_record(
        tombstone,
        ACCOUNT_RECORD_HEADER_BYTES,
        database_bytes,
        &failure_status
    )) {
        write_text("Account delete failed.\n");
        write_status(failure_status);
        return;
    }
    if (!revoke_account_sessions(username, username_length)) {
        write_text("Account deleted, but session revocation failed.\n");
        return;
    }
    write_text("Account deleted.\n");
}

static void set_account_enabled(
    const char *username,
    int enable,
    u8 *buffer
)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }
    u64 username_length = length(username);
    if (!valid_account_username((const u8 *)username, username_length)) {
        write_text("Invalid username.\n");
        return;
    }

    u64 database_bytes = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Account database unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account not found.\n");
        return;
    }

    u64 offset = 0;
    u64 administrator_count = 0;
    u64 target_bytes = 0;
    u8 target_kind = ACCOUNT_RECORD_DELETED;
    int found = 0;
    u8 next_record[ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        if (account_record_is_enabled(record[ACCOUNT_USERNAME_CAPACITY + 2])) {
            administrator_count++;
        }
        if (account_username_matches(record + 2, record[1], username)) {
            found = 1;
            target_bytes = record_bytes;
            target_kind = record[ACCOUNT_USERNAME_CAPACITY + 2];
            for (u64 index = 0; index < record_bytes; index++) {
                next_record[index] = record[index];
            }
        }
        offset += record_bytes;
    }
    if (!found) {
        write_text("Account not found.\n");
        return;
    }

    u8 next_kind = target_kind;
    if (enable) {
        if (account_record_is_enabled(target_kind)) {
            write_text("Account is already enabled.\n");
            return;
        }
        if (!account_record_has_credential(target_kind)) {
            write_text("Account is pending credential setup.\n");
            return;
        }
        next_kind = account_credential_kind(target_kind);
    } else {
        if (account_record_is_disabled(target_kind)) {
            write_text("Account is already disabled.\n");
            return;
        }
        if (!account_record_has_credential(target_kind)) {
            write_text("Account is pending credential setup.\n");
            return;
        }
        if (account_record_is_enabled(target_kind) && administrator_count <= 1) {
            write_text("Cannot disable the last administrator.\n");
            return;
        }
        next_kind = (u8)(account_credential_kind(target_kind) + ACCOUNT_RECORD_DISABLED_OFFSET);
    }
    next_record[ACCOUNT_USERNAME_CAPACITY + 2] = next_kind;
    if (database_bytes + target_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is full.\n");
        return;
    }

    u32 failure_status = 0;
    if (!append_account_record(
        next_record,
        target_bytes,
        database_bytes,
        &failure_status
    )) {
        write_text(enable ? "Account enable failed.\n" : "Account disable failed.\n");
        write_status(failure_status);
        return;
    }
    if (!enable && !revoke_account_sessions(username, username_length)) {
        write_text("Account disabled, but session revocation failed.\n");
        return;
    }
    write_text(enable ? "Account enabled.\n" : "Account disabled.\n");
}

static void rename_account(
    const char *old_username,
    const char *new_username,
    u8 *buffer
)
{
    if (!login_authorized()) {
        write_text("Access denied.\n");
        return;
    }

    u64 old_length = length(old_username);
    u64 new_length = length(new_username);
    if (!valid_account_username((const u8 *)old_username, old_length)
        || !valid_account_username((const u8 *)new_username, new_length)) {
        write_text("Invalid username.\n");
        return;
    }
    if (account_username_bytes_match(
        (const u8 *)old_username,
        old_length,
        (const u8 *)new_username,
        new_length
    )) {
        write_text("Account already uses that name.\n");
        return;
    }

    u64 database_bytes = 0;
    int read = read_account_database(buffer, &database_bytes);
    if (read < 0) {
        write_text("Account database unavailable.\n");
        return;
    }
    if (read == 0 || database_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account not found.\n");
        return;
    }

    u64 offset = 0;
    u64 target_bytes = 0;
    u8 renamed_record[ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    int found = 0;
    int new_name_in_use = 0;
    while (offset < database_bytes) {
        u8 *record = buffer + offset;
        u64 record_bytes = account_record_size(record, database_bytes - offset);
        if (record_bytes == 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        int latest = account_record_is_latest(buffer, database_bytes, offset, record_bytes);
        if (latest < 0) {
            write_text("Account database is corrupt.\n");
            return;
        }
        if (latest == 0 || record[ACCOUNT_USERNAME_CAPACITY + 2] == ACCOUNT_RECORD_DELETED) {
            offset += record_bytes;
            continue;
        }
        if (account_username_matches(record + 2, record[1], old_username)) {
            found = 1;
            target_bytes = record_bytes;
            for (u64 index = 0; index < record_bytes; index++) {
                renamed_record[index] = record[index];
            }
        }
        if (account_username_matches(record + 2, record[1], new_username)) {
            new_name_in_use = 1;
        }
        offset += record_bytes;
    }
    if (new_name_in_use) {
        write_text("Account already exists.\n");
        return;
    }
    if (!found) {
        write_text("Account not found.\n");
        return;
    }
    if (database_bytes + ACCOUNT_RECORD_HEADER_BYTES + target_bytes > ACCOUNT_DATABASE_CAPACITY) {
        write_text("Account database is full.\n");
        return;
    }

    renamed_record[1] = (u8)new_length;
    u8 normalized_new_username[ACCOUNT_USERNAME_CAPACITY] = {0};
    normalize_account_username(new_username, new_length, normalized_new_username);
    for (offset = 0; offset < ACCOUNT_USERNAME_CAPACITY; offset++) {
        renamed_record[2 + offset] = 0;
    }
    for (offset = 0; offset < new_length; offset++) {
        renamed_record[2 + offset] = normalized_new_username[offset];
    }

    u8 tombstone[ACCOUNT_RECORD_HEADER_BYTES] = {0};
    u8 normalized_old_username[ACCOUNT_USERNAME_CAPACITY] = {0};
    normalize_account_username(old_username, old_length, normalized_old_username);
    tombstone[0] = ACCOUNT_RECORD_VERSION;
    tombstone[1] = (u8)old_length;
    for (offset = 0; offset < old_length; offset++) {
        tombstone[2 + offset] = normalized_old_username[offset];
    }
    tombstone[ACCOUNT_USERNAME_CAPACITY + 2] = ACCOUNT_RECORD_DELETED;

    u32 failure_status = 0;
    if (!append_account_record(
        renamed_record,
        target_bytes,
        database_bytes,
        &failure_status
    )) {
        write_text("Account rename failed.\n");
        write_status(failure_status);
        return;
    }
    if (!append_account_record(
        tombstone,
        ACCOUNT_RECORD_HEADER_BYTES,
        database_bytes + target_bytes,
        &failure_status
    )) {
        write_text("Account rename incomplete; new identity is retained.\n");
        write_status(failure_status);
        return;
    }
    write_text("Account renamed; credential identity preserved.\n");
}

__attribute__((noinline))
static void execute_line(char *line, u8 *buffer)
{
    char command[256];
    char path[256];
    char *cursor = line;
    u64 command_length;
    while (*cursor == ' ' || *cursor == '\t' || *cursor == '$') {
        cursor++;
    }
    command_length = next_word(&cursor, command);
    if (command_length == 0) {
        return;
    }
    if (command_named(command, command_length, "DIRECTORY", 9)
        || command_named(command, command_length, "DIR", 3)
        || command_named(command, command_length, "LS", 2)) {
        char argument[256];
        u64 argument_length = next_word(&cursor, argument);
        if (argument_length == 0) {
            char root[2];
            root[0] = '/';
            root[1] = 0;
            print_directory(root, buffer);
            return;
        }
        if (require_absolute_path(argument) != 0) {
            return;
        }
        print_directory(argument, buffer);
        return;
    }
    if (equal_name(command, "LOGIN")) {
        struct response login = call(OP_LOGIN_START, 0, 0, 0, 0, 0, 0);
        if (login.status == STATUS_NORMAL) {
            write_text("Starting login...\n");
        } else {
            write_text("Login request failed\n");
            write_status(login.status);
        }
        return;
    }
    if (equal_name(command, "SHUTDOWN")) {
        call(OP_SHUTDOWN, 0, 0, 0, 0, 0, 0);
        return;
    }
    if (equal_name(command, "WATCHDOG")) {
        watchdog_command(cursor);
        return;
    }
    if (equal_name(command, "LOGOUT")) {
        write_text("Logging out...\n");
        struct response logout = call(OP_LOGIN_LOGOUT, 0, 0, 0, 0, 0, 0);
        if (logout.status != STATUS_NORMAL) {
            write_text("Logout request failed\n");
            write_status(logout.status);
        }
        return;
    }
    if (equal_name(command, "WHOAMI")) {
        char username[33];
        struct response whoami = call(
            OP_LOGIN_WHOAMI, 0, 0, (u64)username, sizeof(username), 0, 0
        );
        if (whoami.status != STATUS_NORMAL || whoami.values[0] >= sizeof(username)) {
            write_text("Whoami request failed\n");
            write_status(whoami.status);
            return;
        }
        username[whoami.values[0]] = 0;
        write_text(username);
        write_text("\n");
        return;
    }
    if (equal_name(command, "PWD")) {
        write_text("/\n");
        return;
    }
    if (equal_name(command, "CREDENTIAL")) {
        char action[256];
        char username[256];
        char id[256];
        if (next_word(&cursor, action) == 0) {
            write_text("Use: CREDENTIAL LIST <username>, ADD <username>, or REMOVE <username> <id>\n");
            return;
        }
        if (next_word(&cursor, username) == 0) {
            write_text(equal_name(action, "LIST")
                ? "Use: CREDENTIAL LIST <username>\n"
                : equal_name(action, "ADD")
                    ? "Use: CREDENTIAL ADD <username>\n"
                    : "Use: CREDENTIAL REMOVE <username> <id>\n");
            return;
        }
        if (equal_name(action, "LIST")) {
            list_credentials(username, buffer);
        } else if (equal_name(action, "ADD")) {
            add_credential(username, buffer);
        } else if (equal_name(action, "REMOVE")) {
            if (next_word(&cursor, id) == 0) {
                write_text("Use: CREDENTIAL REMOVE <username> <id>\n");
                return;
            }
            remove_credential(username, id, buffer);
        } else {
            write_text("Use: CREDENTIAL LIST <username>, ADD <username>, or REMOVE <username> <id>\n");
        }
        return;
    }
    if (equal_name(command, "ACCOUNT")) {
        char action[256];
        char username[256];
        char confirmation[256];
        u64 action_length = next_word(&cursor, action);
        if (action_length == 0) {
            write_text("Use: ACCOUNT LIST, SHOW, CREATE, DELETE, ENABLE, DISABLE, or RENAME\n");
            return;
        }
        if (equal_name(action, "LIST")) {
            list_accounts(buffer);
            return;
        }
        if (equal_name(action, "SHOW")) {
            if (next_word(&cursor, username) == 0) {
                write_text("Use: ACCOUNT SHOW <username>\n");
                return;
            }
            show_account(username, buffer);
            return;
        }
        if (equal_name(action, "CREATE")) {
            if (next_word(&cursor, username) == 0) {
                write_text("Use: ACCOUNT CREATE <username>\n");
                return;
            }
            create_account(username, buffer);
            return;
        }
        if (equal_name(action, "DELETE")) {
            if (next_word(&cursor, username) == 0) {
                write_text("Use: ACCOUNT DELETE <username> CONFIRM\n");
                return;
            }
            if (next_word(&cursor, confirmation) == 0) {
                confirmation[0] = 0;
            }
            delete_account(username, confirmation, buffer);
            return;
        }
        if (equal_name(action, "ENABLE") || equal_name(action, "DISABLE")) {
            if (next_word(&cursor, username) == 0) {
                write_text(equal_name(action, "ENABLE")
                    ? "Use: ACCOUNT ENABLE <username>\n"
                    : "Use: ACCOUNT DISABLE <username>\n");
                return;
            }
            set_account_enabled(username, equal_name(action, "ENABLE"), buffer);
            return;
        }
        if (equal_name(action, "RENAME")) {
            char new_username[256];
            if (next_word(&cursor, username) == 0 || next_word(&cursor, new_username) == 0) {
                write_text("Use: ACCOUNT RENAME <old> <new>\n");
                return;
            }
            rename_account(username, new_username, buffer);
            return;
        }
        {
            write_text("Use: ACCOUNT LIST, SHOW, CREATE, DELETE, ENABLE, DISABLE, or RENAME\n");
            return;
        }
    }
    if (equal_name(command, "HELP") || equal_name(command, "?")
        || equal_name(command, "COMMANDS")) {
        print_help();
        return;
    }
    if (next_word(&cursor, path) == 0) {
        write_text("missing path\n");
        return;
    }
    if (require_absolute_path(path) != 0) {
        return;
    }
    u64 path_length = length(path);
    struct response response;
    if (equal_name(command, "CREATE")) {
        response = call(OP_GHOSTFS_OPEN, OPEN_READ | OPEN_WRITE | OPEN_CREATE | OPEN_EXCLUSIVE,
                        0, (u64)path, path_length, 0, 0);
        if (response.status == STATUS_NORMAL) {
            call(OP_GHOSTFS_CLOSE, 0, response.values[0], 0, 0, 0, 0);
        }
    } else if (equal_name(command, "TYPE") || equal_name(command, "CAT")) {
        type_file(path, buffer);
        return;
    } else if (equal_name(command, "MKDIR")) {
        response = call(OP_GHOSTFS_MKDIR, FLAG_RECURSIVE, 0, (u64)path, path_length, 0, 0);
    } else if (equal_name(command, "RMDIR") || equal_name(command, "RD")) {
        response = call(OP_GHOSTFS_RMDIR, 0, 0, (u64)path, path_length, 0, 0);
    } else if (equal_name(command, "DELETE") || equal_name(command, "DEL")) {
        response = call(OP_GHOSTFS_DELETE, 0, 0, (u64)path, path_length, 0, 0);
    } else {
        write_text("unknown filesystem command\n");
        return;
    }
    if (response.status != STATUS_NORMAL) {
        write_status(response.status);
    }
}

static u64 credential_kind_from_answer(const char *answer)
{
    if (answer[0] == 0 || equal_name(answer, "PASSKEY") || equal_name(answer, "1")) {
        return 1;
    }
    if (equal_name(answer, "TPM") || equal_name(answer, "2")) {
        return 2;
    }
    if (equal_name(answer, "SSH") || equal_name(answer, "3")) {
        return 3;
    }
    return 0;
}

static const char *credential_kind_name(u64 kind_id)
{
    if (kind_id == 2) {
        return "TPM";
    }
    if (kind_id == 3) {
        return "SSH";
    }
    return "PASSKEY";
}

static int reset_first_admin_staging(void);

__attribute__((noinline))
static int commit_first_admin_account(
    const char *username,
    u64 username_length,
    u64 kind_id,
    const u8 *material,
    u64 material_length
)
{
    struct response response = call(
        OP_LOGIN_BOOTSTRAP_USERNAME,
        0,
        0,
        (u64)username,
        username_length,
        0,
        0
    );
    if (response.status != STATUS_NORMAL) {
        write_text("Username rejected. status=");
        write_hex(response.status);
        write_text("\n");
        write_text("Repeat setup with identical answers until the commit succeeds.\n");
        return 0;
    }
    write_text("Administrator username saved.\n");
    response = call(
        OP_LOGIN_BOOTSTRAP_CREDENTIAL,
        0,
        0,
        kind_id,
        (u64)material,
        material_length,
        0
    );
    if (response.status != STATUS_NORMAL) {
        if (response.status == 0x00030018U
            && reset_first_admin_staging()) {
            write_text("Old setup cleared. Retrying the same passkey.\n");
            response = call(
                OP_LOGIN_BOOTSTRAP_USERNAME,
                0,
                0,
                (u64)username,
                username_length,
                0,
                0
            );
            if (response.status == STATUS_NORMAL) {
                response = call(
                    OP_LOGIN_BOOTSTRAP_CREDENTIAL,
                    0,
                    0,
                    kind_id,
                    (u64)material,
                    material_length,
                    0
                );
            }
        }
    }
    if (response.status != STATUS_NORMAL) {
        write_text("Credential rejected. status=");
        write_hex(response.status);
        write_text("\n");
        write_text("Repeat setup with identical answers until the commit succeeds.\n");
        return 0;
    }
    write_text("Administrator credential saved.\n");
    response = call(OP_LOGIN_BOOTSTRAP_CONFIRM, 0, 0, 0, 0, 0, 0);
    if (response.status != STATUS_NORMAL) {
        write_text("Confirmation rejected. status=");
        write_hex(response.status);
        write_text("\n");
        write_text("Repeat setup with identical answers until the commit succeeds.\n");
        return 0;
    }
    write_text("Administrator account committed.\n");
    return 1;
}

static int reset_first_admin_staging(void)
{
    struct response response = call(
        OP_LOGIN_BOOTSTRAP_RECOVERY,
        0,
        0,
        2,
        0,
        0,
        0
    );
    if (response.status != STATUS_NORMAL) {
        return 0;
    }
    response = call(
        OP_LOGIN_BOOTSTRAP_RECOVERY,
        0,
        0,
        1,
        0,
        0,
        0
    );
    return response.status == STATUS_NORMAL
        && response.values[0] == 0
        && response.values[1] == 0;
}

__attribute__((noinline))
static void run_first_run_wizard(void)
{
    char username[256];
    char kind[256];
    char material[256];
    char answer[256];
    u8 material_bytes[ACCOUNT_CREDENTIAL_CAPACITY] = {0};
    u64 username_length;
    u64 material_length;
    u64 kind_id;

    write_text("Answer each question to create the administrator account.\n");
    write_text("GhostOS starts once the account is committed.\n");
    write_text("Open the local passkey URL shown by the VM host for guided setup.\n");
    write_text("Passkey public material arrives through the local bridge and stays hidden.\n");
    for (;;) {
        write_text("\n\x1b]GhostOSEnroll\x07");
        username_length = read_bridge_credential_line(username, sizeof(username));
        if (username_length == 0 || username_length > 32) {
            write_text("Username must be 1-32 valid characters.\n");
            continue;
        }
        if (!reset_first_admin_staging()) {
            write_text("Previous setup state could not be cleared.\n");
            continue;
        }
        write_text("Credential type [PASSKEY/TPM/SSH] (PASSKEY): ");
        read_bridge_credential_line(kind, sizeof(kind));
        kind_id = credential_kind_from_answer(kind);
        if (kind_id == 0) {
            write_text("Unknown credential type. Use PASSKEY, TPM, or SSH.\n");
            continue;
        }
        write_text("Waiting for passkey public key from local browser: ");
        u64 stored_material_length;
        if (bridge_transport_active && kind_id == 1) {
            read_bridge_credential_line(
                (char *)material_bytes,
                sizeof(material_bytes)
            );
            stored_material_length = canonical_passkey_material(material_bytes) ? 77 : 0;
            material_length = stored_material_length;
        } else if (bridge_transport_active) {
            material_length = read_bridge_credential_line(material, sizeof(material));
            stored_material_length = material_length;
            for (u64 index = 0; index < material_length && index < sizeof(material_bytes); index++) {
                material_bytes[index] = (u8)material[index];
            }
        } else {
            material_length = read_bridge_credential_line(material, sizeof(material));
            stored_material_length = material_length;
            if (kind_id == 1) {
                stored_material_length = decode_hex_material(
                    material,
                    material_length,
                    material_bytes,
                    sizeof(material_bytes)
                );
            } else {
                for (u64 index = 0; index < material_length && index < sizeof(material_bytes); index++) {
                    material_bytes[index] = (u8)material[index];
                }
            }
        }
        if (material_length == 0 || stored_material_length == 0) {
            write_text("Credential material is invalid.\n");
            continue;
        }
        write_text("\nUsername: ");
        write_bytes(username, username_length);
        write_text("\nCredential type: ");
        write_text(credential_kind_name(kind_id));
        write_text("\nMaterial: received");
        write_text("\nCreate this administrator account? [y/N]: ");
        read_bridge_credential_line(answer, sizeof(answer));
        if (equal_name(answer, "SHUTDOWN")) {
            call(OP_SHUTDOWN, 0, 0, 0, 0, 0, 0);
            continue;
        }
        if (!equal_name(answer, "Y") && !equal_name(answer, "YES")) {
            write_text("Setup restarted with new answers.\n");
            continue;
        }
        if (commit_first_admin_account(
            username,
            username_length,
            kind_id,
            material_bytes,
            stored_material_length
        )) {
            return;
        }
    }
}

static void execute_locked_line(char *line)
{
    char command[256];
    char *cursor = line;
    if (next_word(&cursor, command) != 0 && equal_name(command, "SHUTDOWN")) {
        call(OP_SHUTDOWN, 0, 0, 0, 0, 0, 0);
        return;
    }
    write_text("Terminal locked. Use the login prompt or SHUTDOWN.\n");
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

    while (!shell_dependencies_ready()) {
        sleep_for(SHELL_POLL_DELAY_US);
    }
    while (!shell_ready()) {
        sleep_for(SHELL_POLL_DELAY_US);
    }
    if (first_run_mode()) {
        write_text("No administrator account exists.\n");
        write_text("GhostOS first-run setup mode\n");
        run_first_run_wizard();
        write_text("\nGhostOS user shell\n");
    } else {
        write_text("GhostOS user shell\n");
    }
    int prompt_authorized = -1;
    int prompt_first_run = -1;
    for (;;) {
        if (!update_prompt(&prompt_authorized, &prompt_first_run)) {
            line_length = 0;
            sleep_for(SHELL_POLL_DELAY_US);
            continue;
        }
        if (!read_byte(&byte)) {
            sleep_for(SHELL_POLL_DELAY_US);
            idle_polls++;
            if (idle_polls == 256) {
                call(OP_SERVICE_HEARTBEAT, 0, 0, SHELL_ROLE, ++heartbeat, 0, 0);
                idle_polls = 0;
            }
            continue;
        }
        idle_polls = 0;
        if (byte == '\r' || byte == '\n') {
            if (line_length == 0) {
                continue;
            }
            write_text("\n");
            line[line_length] = 0;
            if (prompt_authorized) {
                execute_line(line, buffer);
            } else {
                execute_locked_line(line);
            }
            line_length = 0;
            prompt_authorized = -1;
            prompt_first_run = -1;
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
