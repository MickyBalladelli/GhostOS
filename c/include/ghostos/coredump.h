#ifndef GHOSTOS_COREDUMP_H
#define GHOSTOS_COREDUMP_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid, 2 too small, 3 capacity, 4 corrupt, 5 snapshot.
 * Crash reason: panic=1, protection=2, illegal=3, watchdog=4, unexpected=5. */
#define GHOSTOS_CORE_PATH_BYTES 192u
#define GHOSTOS_CORE_METADATA_BYTES 288u
#define GHOSTOS_CORE_PAGE_BYTES 4096u
int ghostos_core_path(const uint8_t *path, size_t length);
int ghostos_core_directory(const uint8_t *path, size_t length);
int ghostos_core_child(const uint8_t *directory, size_t directory_length, const uint8_t *suffix,
    size_t suffix_length, uint8_t *output, size_t output_length, size_t *written);
int ghostos_core_page_name(size_t index, uint8_t suffix[13]);
int ghostos_core_metadata(uint64_t process, uint8_t reason, uint64_t timestamp_us, uint32_t pages,
    const uint64_t *registers, size_t register_count, uint8_t output[GHOSTOS_CORE_METADATA_BYTES]);
int ghostos_core_decode_metadata(const uint8_t *input, size_t length, uint64_t *process,
    uint8_t *reason, uint64_t *timestamp_us, uint32_t *pages, uint64_t *registers, size_t *register_count);
int ghostos_core_page(uint64_t virtual_address, const uint8_t *bytes, size_t length,
    uint8_t *output, size_t output_length, size_t *written);
#endif
