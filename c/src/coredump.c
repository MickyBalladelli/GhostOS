#include "ghostos/coredump.h"
static const uint8_t GHOSTOS_CORE_MAGIC[8] = {'S', 'Y', 'N', 'C', 'O', 'R', 'E', '1'};
static void store_le64(uint8_t *output, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) output[i] = (uint8_t)(value >> (8 * i));
}
static uint64_t load_le64(const uint8_t *input) {
    uint64_t value = 0;
    size_t i;
    for (i = 0; i < 8; ++i) value |= (uint64_t)input[i] << (8 * i);
    return value;
}
static int component_rejected(const uint8_t *path, size_t length) {
    size_t start = 0;
    size_t i;
    for (i = 0; i <= length; ++i) {
        if (i == length || path[i] == '/') {
            size_t component = i - start;
            if ((component == 1 && path[start] == '.') ||
                (component == 2 && path[start] == '.' && path[start + 1] == '.')) return 1;
            start = i + 1;
        }
    }
    return 0;
}
int ghostos_core_path(const uint8_t *path, size_t length) {
    size_t i;
    if (!length || length > GHOSTOS_CORE_PATH_BYTES || path[0] != '/' || path[length - 1] == '/') return 1;
    for (i = 0; i < length; ++i) {
        if (!path[i] || path[i] == ';' || (path[i] == '/' && i + 1 < length && path[i + 1] == '/')) return 1;
    }
    return component_rejected(path, length) ? 1 : 0;
}
int ghostos_core_directory(const uint8_t *path, size_t length) {
    static const uint8_t root[] = {'/', 'c', 'o', 'r', 'e', 's'};
    size_t i;
    if (ghostos_core_path(path, length)) return 1;
    if (length == sizeof root) {
        for (i = 0; i < sizeof root; ++i) if (path[i] != root[i]) break;
        if (i == sizeof root) return 1;
    }
    if (length < sizeof root + 1) return 1;
    for (i = 0; i < sizeof root; ++i) if (path[i] != root[i]) return 1;
    return path[sizeof root] == '/' ? 0 : 1;
}
int ghostos_core_child(const uint8_t *directory, size_t directory_length, const uint8_t *suffix,
    size_t suffix_length, uint8_t *output, size_t output_length, size_t *written) {
    size_t required = directory_length + 1 + suffix_length;
    size_t i;
    if (required < directory_length || required > GHOSTOS_CORE_PATH_BYTES) return 2;
    if (output_length < required) return 2;
    for (i = 0; i < directory_length; ++i) output[i] = directory[i];
    output[directory_length] = '/';
    for (i = 0; i < suffix_length; ++i) output[directory_length + 1 + i] = suffix[i];
    *written = required;
    return 0;
}
int ghostos_core_page_name(size_t index, uint8_t suffix[13]) {
    uint64_t value = index;
    size_t position;
    if (index > 99999999u) return 3;
    suffix[0] = 'P'; suffix[1] = 'A'; suffix[2] = 'G'; suffix[3] = 'E'; suffix[4] = '-';
    for (position = 0; position < 8; ++position) {
        suffix[12 - position] = (uint8_t)('0' + value % 10);
        value /= 10;
    }
    return 0;
}
int ghostos_core_metadata(uint64_t process, uint8_t reason, uint64_t timestamp_us, uint32_t pages,
    const uint64_t *registers, size_t register_count, uint8_t output[GHOSTOS_CORE_METADATA_BYTES]) {
    size_t i;
    if (!process || !reason || reason > 5 || !register_count || register_count > 32) return 1;
    for (i = 0; i < GHOSTOS_CORE_METADATA_BYTES; ++i) output[i] = 0;
    for (i = 0; i < 8; ++i) output[i] = GHOSTOS_CORE_MAGIC[i];
    output[8] = 1;
    output[10] = reason;
    output[11] = (uint8_t)register_count;
    store_le64(output + 12, process);
    store_le64(output + 20, timestamp_us);
    output[28] = (uint8_t)pages;
    output[29] = (uint8_t)(pages >> 8);
    output[30] = (uint8_t)(pages >> 16);
    output[31] = (uint8_t)(pages >> 24);
    for (i = 0; i < register_count; ++i) store_le64(output + 32 + i * 8, registers[i]);
    return 0;
}
int ghostos_core_decode_metadata(const uint8_t *input, size_t length, uint64_t *process,
    uint8_t *reason, uint64_t *timestamp_us, uint32_t *pages, uint64_t *registers, size_t *register_count) {
    size_t count;
    size_t i;
    if (length < GHOSTOS_CORE_METADATA_BYTES) return 2;
    for (i = 0; i < 8; ++i) if (input[i] != GHOSTOS_CORE_MAGIC[i]) return 4;
    if (input[8] != 1 || input[9] != 0 || !input[10] || input[10] > 5) return 4;
    count = input[11];
    if (!count || count > 32) return 4;
    *process = load_le64(input + 12);
    if (!*process) return 4;
    *reason = input[10];
    *timestamp_us = load_le64(input + 20);
    *pages = (uint32_t)input[28] | ((uint32_t)input[29] << 8) | ((uint32_t)input[30] << 16) | ((uint32_t)input[31] << 24);
    for (i = 0; i < count; ++i) registers[i] = load_le64(input + 32 + i * 8);
    *register_count = count;
    return 0;
}
int ghostos_core_page(uint64_t virtual_address, const uint8_t *bytes, size_t length,
    uint8_t *output, size_t output_length, size_t *written) {
    size_t i;
    if (!length || length > GHOSTOS_CORE_PAGE_BYTES) return 5;
    if (output_length < 16 + length) return 2;
    store_le64(output, virtual_address);
    store_le64(output + 8, length);
    for (i = 0; i < length; ++i) output[16 + i] = bytes[i];
    *written = 16 + length;
    return 0;
}
