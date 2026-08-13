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
    OP_LOGIN_STATUS = 51,
    OP_LOGIN_START = 52,
    OP_LOGIN_BOOTSTRAP_USERNAME = 58,
    OP_LOGIN_BOOTSTRAP_CREDENTIAL = 59,
    OP_LOGIN_BOOTSTRAP_CONFIRM = 60,
    OP_LOGIN_BOOTSTRAP_RECOVERY = 61,
    OP_LOGIN_LOGOUT = 56,
    OP_LOGIN_WHOAMI = 57,
    OP_SLEEP_UNTIL = 47,
    SHELL_ROLE = 9,
    OPEN_READ = 1,
    OPEN_WRITE = 2,
    OPEN_CREATE = 4,
    OPEN_EXCLUSIVE = 1 << 9,
    FLAG_RECURSIVE = 1 << 8,
};

static const char ACCOUNT_AUTHORIZATION_PATH[] = "/system/security/authorization";
enum {
    ACCOUNT_RECORD_VERSION = 1,
    ACCOUNT_USERNAME_CAPACITY = 32,
    ACCOUNT_CREDENTIAL_CAPACITY = 96,
    ACCOUNT_RECORD_HEADER_BYTES = 36,
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

static int login_authorized(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0, 0, 0);
    return response.status == 0 && response.values[3] != 0;
}

static int first_run_mode(void)
{
    struct response response = call(OP_LOGIN_STATUS, 0, 0, 0, 0, 0, 0);
    return response.status == 0 && response.values[1] == 0;
}

static void write_prompt(int authorized, int first_run)
{
    if (authorized) {
        write_text("$ ");
    } else if (first_run) {
        write_text("\x1b[1;33mSYNOS\x1b[90m::\x1b[35mFIRST-RUN\x1b[0m> ");
    } else {
        write_text("\x1b[1;33mSYNOS\x1b[90m::\x1b[31mLOCKED\x1b[0m> ");
    }
}

static int update_prompt(int *prompt_authorized, int *prompt_first_run)
{
    int authorized = login_authorized();
    int first_run = first_run_mode();
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

static int valid_account_username(const u8 *username, u64 username_length)
{
    u64 index;
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
    return 1;
}

static const char *account_credential_name(u8 kind)
{
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

static int account_username_matches(
    const u8 *stored,
    u64 stored_length,
    const char *requested
)
{
    u64 requested_length = length(requested);
    u64 index;
    if (stored_length != requested_length) {
        return 0;
    }
    for (index = 0; index < stored_length; index++) {
        u8 left = stored[index];
        u8 right = (u8)requested[index];
        if (left >= 'a' && left <= 'z') {
            left = (u8)(left - ('a' - 'A'));
        }
        if (right >= 'a' && right <= 'z') {
            right = (u8)(right - ('a' - 'A'));
        }
        if (left != right) {
            return 0;
        }
    }
    return 1;
}

static int valid_account_record(const u8 *buffer, u64 record_bytes)
{
    u64 username_length = buffer[1];
    u64 credential_length = buffer[ACCOUNT_RECORD_HEADER_BYTES - 1];
    return record_bytes >= ACCOUNT_RECORD_HEADER_BYTES
        && record_bytes <= ACCOUNT_RECORD_HEADER_BYTES + ACCOUNT_CREDENTIAL_CAPACITY
        && buffer[0] == ACCOUNT_RECORD_VERSION
        && valid_account_username(buffer + 2, username_length)
        && account_credential_name(buffer[ACCOUNT_USERNAME_CAPACITY + 2]) != 0
        && credential_length != 0
        && credential_length <= ACCOUNT_CREDENTIAL_CAPACITY
        && record_bytes == ACCOUNT_RECORD_HEADER_BYTES + credential_length;
}

static int read_account_record(u8 *buffer, u64 *record_bytes)
{
    struct response opened = call(
        OP_SYNFS_OPEN,
        OPEN_READ,
        0,
        (u64)ACCOUNT_AUTHORIZATION_PATH,
        length(ACCOUNT_AUTHORIZATION_PATH),
        0,
        0
    );
    if (opened.status != 0) {
        return 0;
    }

    struct response read = call(
        OP_SYNFS_READ,
        0,
        opened.values[0],
        (u64)buffer,
        4096,
        1,
        0
    );
    call(OP_SYNFS_CLOSE, 0, opened.values[0], 0, 0, 0, 0);
    if (read.status != 0) {
        return -1;
    }
    *record_bytes = read.values[0];
    return *record_bytes != 0;
}

static void list_accounts(u8 *buffer)
{
    u64 record_bytes = 0;
    int read = read_account_record(buffer, &record_bytes);
    if (read < 0) {
        write_text("Account list unavailable.\n");
        return;
    }
    if (read == 0) {
        write_text("No local accounts.\n");
        return;
    }
    if (!valid_account_record(buffer, record_bytes)) {
        write_text("Account database is corrupt.\n");
        return;
    }

    u64 username_length = buffer[1];

    write_text("username=");
    write_bytes((const char *)(buffer + 2), username_length);
    write_text(" state=enabled credential=");
    write_text(account_credential_name(buffer[ACCOUNT_USERNAME_CAPACITY + 2]));
    write_text("\n");
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

    u64 record_bytes = 0;
    int read = read_account_record(buffer, &record_bytes);
    if (read < 0) {
        write_text("Account details unavailable.\n");
        return;
    }
    if (read == 0) {
        write_text("Account not found.\n");
        return;
    }
    if (!valid_account_record(buffer, record_bytes)) {
        write_text("Account database is corrupt.\n");
        return;
    }
    if (!account_username_matches(buffer + 2, buffer[1], username)) {
        write_text("Account not found.\n");
        return;
    }

    write_text("username=");
    write_bytes((const char *)(buffer + 2), buffer[1]);
    write_text("\nstate=enabled\nscope=local\ncredential=");
    write_text(account_credential_name(buffer[ACCOUNT_USERNAME_CAPACITY + 2]));
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
    if (equal_name(command, "LOGOUT")) {
        write_text("Logging out...\n");
        struct response logout = call(OP_LOGIN_LOGOUT, 0, 0, 0, 0, 0, 0);
        if (logout.status != 0) {
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
        if (whoami.status != 0 || whoami.values[0] >= sizeof(username)) {
            write_text("Whoami request failed\n");
            write_status(whoami.status);
            return;
        }
        username[whoami.values[0]] = 0;
        write_text(username);
        write_text("\n");
        return;
    }
    if (equal_name(command, "ACCOUNT")) {
        char action[256];
        char username[256];
        u64 action_length = next_word(&cursor, action);
        if (action_length == 0) {
            write_text("Use: ACCOUNT LIST or ACCOUNT SHOW <username>\n");
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
        {
            write_text("Use: ACCOUNT LIST or ACCOUNT SHOW <username>\n");
            return;
        }
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

static void execute_first_run_line(char *line)
{
    char command[256];
    char username[256];
    char *cursor = line;
    if (next_word(&cursor, command) == 0) {
        return;
    }
    if (equal_name(command, "HELP")) {
        write_text("Use USERNAME, CREDENTIAL, CONFIRM, or RECOVERY.\n");
        return;
    }
    if (equal_name(command, "USERNAME")) {
        u64 username_length = next_word(&cursor, username);
        if (username_length == 0 || username_length > 32) {
            write_text("Username must be 1-32 valid characters.\n");
            return;
        }
        struct response response = call(
            OP_LOGIN_BOOTSTRAP_USERNAME,
            0,
            0,
            (u64)username,
            username_length,
            0,
            0
        );
        if (response.status == 0) {
            write_text("Administrator username saved.\n");
        } else {
            write_text("Username rejected.\n");
        }
        return;
    }
    if (equal_name(command, "CREDENTIAL")) {
        char kind[256];
        char material[256];
        u64 kind_length = next_word(&cursor, kind);
        u64 material_length = next_word(&cursor, material);
        u64 kind_id = 0;
        if (equal_name(kind, "PASSKEY")) {
            kind_id = 1;
        } else if (equal_name(kind, "TPM")) {
            kind_id = 2;
        } else if (equal_name(kind, "SSH")) {
            kind_id = 3;
        }
        if (kind_length == 0 || material_length == 0 || material_length > 96 || kind_id == 0) {
            write_text("Use: CREDENTIAL PASSKEY|TPM|SSH <material>\n");
            return;
        }
        struct response response = call(
            OP_LOGIN_BOOTSTRAP_CREDENTIAL,
            0,
            0,
            kind_id,
            (u64)material,
            material_length,
            0
        );
        if (response.status == 0) {
            write_text("Administrator credential saved.\n");
        } else {
            write_text("Credential rejected.\n");
        }
        return;
    }
    if (equal_name(command, "CONFIRM")) {
        struct response response = call(
            OP_LOGIN_BOOTSTRAP_CONFIRM,
            0,
            0,
            0,
            0,
            0,
            0
        );
        if (response.status == 0) {
            write_text("Administrator account committed.\n");
        } else {
            write_text("Confirmation rejected.\n");
        }
        return;
    }
    if (equal_name(command, "RECOVERY")) {
        char action[256];
        u64 action_length = next_word(&cursor, action);
        u64 action_id = 0;
        if (action_length == 0 || equal_name(action, "STATUS")) {
            action_id = 1;
        } else if (equal_name(action, "RESET")) {
            action_id = 2;
        } else if (equal_name(action, "RETRY")) {
            action_id = 3;
        }
        if (action_id == 0) {
            write_text("Use: RECOVERY STATUS|RESET|RETRY\n");
            return;
        }
        struct response response = call(
            OP_LOGIN_BOOTSTRAP_RECOVERY,
            0,
            0,
            action_id,
            0,
            0,
            0
        );
        if (response.status != 0) {
            write_text("Recovery failed.\n");
        } else if (action_id == 1) {
            write_text("Recovery state: username=");
            write_hex((u32)response.values[0]);
            write_text(" credential=");
            write_hex((u32)response.values[1]);
            write_text(" sync=");
            write_hex((u32)response.values[2]);
            write_text("\n");
        } else if (action_id == 2) {
            write_text("Pending setup cleared.\n");
        } else {
            write_text("Recovery retry complete.\n");
        }
        return;
    }
    write_text("Use USERNAME, CREDENTIAL, CONFIRM, or RECOVERY.\n");
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
    if (first_run_mode()) {
        write_text("SynOS first-run setup mode\n");
    } else {
        write_text("SynOS user shell\n");
    }
    int prompt_authorized = -1;
    int prompt_first_run = -1;
    for (;;) {
        if (!update_prompt(&prompt_authorized, &prompt_first_run)) {
            line_length = 0;
            sleep_for(1000);
            continue;
        }
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
            if (first_run_mode()) {
                execute_first_run_line(line);
            } else if (login_authorized()) {
                execute_line(line, buffer);
            }
            line_length = 0;
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
