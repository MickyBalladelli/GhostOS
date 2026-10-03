#ifndef GHOSTOS_BACKUP_H
#define GHOSTOS_BACKUP_H
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid budget, 2 output too small, 3 not complete, 4 invalid.
 * State: volume header=0, file header=1, file data=2, trailer=3, complete=4. */
#define GHOSTOS_BACKUP_HEADER_BYTES 228u
typedef struct {
    const uint8_t *name;
    size_t name_length;
    const uint8_t *data;
    size_t size;
    uint32_t version;
    uint64_t checksum, created_at;
} ghostos_backup_file;
typedef struct {
    uint64_t checkpoint_id, generation, file_offset, bytes_streamed;
    uint32_t file_index, files_streamed;
    size_t header_length, header_offset;
    uint8_t header[GHOSTOS_BACKUP_HEADER_BYTES];
    uint8_t state;
} ghostos_backup_job;
void ghostos_backup_start(ghostos_backup_job *job, uint64_t checkpoint_id, uint64_t generation);
int ghostos_backup_poll(ghostos_backup_job *job, const ghostos_backup_file *files, size_t file_count,
    uint8_t *output, size_t output_capacity, size_t *written, size_t byte_budget);
int ghostos_backup_finish(const ghostos_backup_job *job, uint32_t *files_streamed, uint64_t *bytes_streamed);
#endif
