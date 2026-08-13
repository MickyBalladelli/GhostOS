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
    OP_LOGIN_CHALLENGE = 53,
    OP_LOGIN_TPM_CHALLENGE = 54,
    OP_LOGIN_TPM_COMPLETE = 55,
    LOGIN_ROLE = 14,
};

enum {
    PASSKEY_CHALLENGE_BYTES = 32,
    PASSKEY_MAX_ASSERTION_BYTES = 512,
    TPM_CHALLENGE_BYTES = 32,
    TPM_MAX_QUOTE_BYTES = 512,
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

static void wait_until(u64 deadline)
{
    for (;;) {
        struct response clock = call(OP_CLOCK_NOW, 0, 0, 0, 0);
        if (clock.status != 0 || clock.values[0] >= deadline) {
            return;
        }
        call(OP_SLEEP_UNTIL, deadline, 0, 0, 0);
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

static u64 login_lock_until(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0);
    return response.status == 0 ? response.values[2] : 0;
}

static void clear_bytes(void *bytes, u64 capacity)
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

static int hex_digit(u8 byte)
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

static int valid_username(const char *username, u64 username_length)
{
    u64 index;
    if (username_length == 0 || username_length > 32) {
        return 0;
    }
    for (index = 0; index < username_length; index++) {
        u8 byte = (u8)username[index];
        if (!(byte >= 'a' && byte <= 'z')
            && !(byte >= 'A' && byte <= 'Z')
            && !(byte >= '0' && byte <= '9')
            && byte != '.' && byte != '_' && byte != '-' && byte != '$') {
            return 0;
        }
    }
    return 1;
}

static int valid_hex(const char *text, u64 text_length)
{
    u64 index;
    if (text_length == 0
        || (text_length & 1) != 0
        || text_length > TPM_MAX_QUOTE_BYTES * 2) {
        return 0;
    }
    for (index = 0; index < text_length; index++) {
        if (hex_digit((u8)text[index]) < 0) {
            return 0;
        }
    }
    return 1;
}

static void write_login_rejected(void)
{
    write_text("Login failed: username or credential was not accepted.\n");
    write_text("Check both and try again.\n");
}

static u64 decode_hex(const char *text, u64 text_length, u8 *output, u64 capacity)
{
    u64 count = 0;
    u64 index;
    if (text_length == 0 || (text_length & 1) != 0) {
        return 0;
    }
    for (index = 0; index < text_length; index += 2) {
        int high = hex_digit((u8)text[index]);
        int low = hex_digit((u8)text[index + 1]);
        if (high < 0 || low < 0 || count >= capacity) {
            return 0;
        }
        output[count++] = (u8)((high << 4) | low);
    }
    return count;
}

static void write_hex_bytes(const u8 *bytes, u64 count)
{
    static const char digits[] = "0123456789abcdef";
    u64 index;
    for (index = 0; index < count; index++) {
        char encoded[2];
        encoded[0] = digits[bytes[index] >> 4];
        encoded[1] = digits[bytes[index] & 0xf];
        write_bytes(encoded, sizeof(encoded));
    }
}

__attribute__((section(".text._start"), noreturn))
void _start(void)
{
    char username[128];
    char method[16];
    char credential_hex[TPM_MAX_QUOTE_BYTES * 2 + 1];
    u8 credential[TPM_MAX_QUOTE_BYTES];
    u8 challenge[TPM_CHALLENGE_BYTES];

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
        u64 locked_until = login_lock_until();
        if (locked_until != 0) {
            write_text("Login temporarily locked after repeated failures.\n");
            wait_until(locked_until);
            continue;
        }
        write_text("login: ");
        u64 username_length = read_line(username, sizeof(username), 1);
        write_text("\ncredential [passkey/tpm]: ");
        u64 method_length = read_line(method, sizeof(method), 1);
        int use_tpm = method_length == 3
            && (method[0] == 't' || method[0] == 'T')
            && (method[1] == 'p' || method[1] == 'P')
            && (method[2] == 'm' || method[2] == 'M');
        int use_passkey = method_length == 0
            || (method[0] == 'p' || method[0] == 'P');
        if (!use_tpm && !use_passkey) {
            write_text("\nUnknown credential type. Use passkey or tpm.\n");
            clear_bytes(username, sizeof(username));
            clear_bytes(method, sizeof(method));
            continue;
        }
        if (!valid_username(username, username_length)) {
            write_text("\nUsername must be 1-32 letters, numbers, or . _ - $.\n");
            clear_bytes(username, sizeof(username));
            clear_bytes(method, sizeof(method));
            continue;
        }
        u16 challenge_operation = use_tpm
            ? OP_LOGIN_TPM_CHALLENGE
            : OP_LOGIN_CHALLENGE;
        struct response challenge_response = call(
            challenge_operation,
            (u64)challenge,
            sizeof(challenge),
            0,
            0
        );
        if (challenge_response.status != 0) {
            write_login_rejected();
            clear_bytes(username, sizeof(username));
            clear_bytes(method, sizeof(method));
            continue;
        }
        write_text(use_tpm
            ? "\nPresent your TPM-backed credential and paste its quote as hex.\nchallenge: "
            : "\nTouch your passkey and paste its assertion as hex.\nchallenge: ");
        write_hex_bytes(challenge, sizeof(challenge));
        write_text(use_tpm ? "\ntpm quote: " : "\npasskey: ");
        u64 credential_hex_length = read_private_line(
            credential_hex,
            sizeof(credential_hex)
        );
        u64 credential_length = decode_hex(
            credential_hex,
            credential_hex_length,
            credential,
            sizeof(credential)
        );
        write_text("\n");

        if (!valid_hex(credential_hex, credential_hex_length)
            || credential_length == 0) {
            write_text("Credential must be non-empty hexadecimal data.\n");
            clear_bytes(username, sizeof(username));
            clear_bytes(method, sizeof(method));
            clear_bytes(credential_hex, sizeof(credential_hex));
            clear_bytes(credential, sizeof(credential));
            continue;
        }

        u16 complete_operation = use_tpm
            ? OP_LOGIN_TPM_COMPLETE
            : OP_LOGIN_COMPLETE;
        struct response completed = call(
            complete_operation,
            (u64)username,
            username_length,
            (u64)credential,
            credential_length
        );
        clear_bytes(username, sizeof(username));
        clear_bytes(method, sizeof(method));
        clear_bytes(credential_hex, sizeof(credential_hex));
        clear_bytes(credential, sizeof(credential));
        if (completed.status != 0) {
            write_login_rejected();
            continue;
        }
        write_text("Login accepted.\n");
    }
}
