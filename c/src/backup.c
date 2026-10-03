#include "ghostos/backup.h"
static void store_le16(uint8_t *output, uint16_t value) {
    output[0] = (uint8_t)value;
    output[1] = (uint8_t)(value >> 8);
}
static void store_le32(uint8_t *output, uint32_t value) {
    size_t i;
    for (i = 0; i < 4; ++i) output[i] = (uint8_t)(value >> (8 * i));
}
static void store_le64(uint8_t *output, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) output[i] = (uint8_t)(value >> (8 * i));
}
static void add_streamed(uint64_t *value, size_t amount) {
    if (amount > UINT64_MAX - *value) *value = UINT64_MAX;
    else *value += amount;
}
static void clear(uint8_t *bytes, size_t length) {
    size_t i;
    for (i = 0; i < length; ++i) bytes[i] = 0;
}
static int append(uint8_t *output, size_t output_capacity, size_t *written, const uint8_t *bytes, size_t amount) {
    size_t i;
    if (*written > output_capacity || amount > output_capacity - *written) return 2;
    for (i = 0; i < amount; ++i) output[*written + i] = bytes[i];
    *written += amount;
    return 0;
}
static void volume_header(ghostos_backup_job *job) {
    clear(job->header, sizeof job->header);
    job->header[0] = 'S'; job->header[1] = 'Y'; job->header[2] = 'N'; job->header[3] = 'B';
    job->header[4] = 'A'; job->header[5] = 'C'; job->header[6] = 'K'; job->header[7] = '1';
    store_le16(job->header + 8, 1);
    store_le64(job->header + 12, job->checkpoint_id);
    store_le64(job->header + 20, job->generation);
    store_le32(job->header + 28, 4096);
    job->header_length = 32;
    job->header_offset = 0;
    job->state = 0;
}
static int file_header(ghostos_backup_job *job, const ghostos_backup_file *file) {
    size_t header_length = 36 + file->name_length;
    size_t i;
    if (file->name_length > 192 || header_length > sizeof job->header) return 4;
    clear(job->header, sizeof job->header);
    job->header[0] = 'F'; job->header[1] = 'I'; job->header[2] = 'L'; job->header[3] = 'E';
    store_le16(job->header + 4, (uint16_t)header_length);
    store_le16(job->header + 6, (uint16_t)file->name_length);
    store_le32(job->header + 8, file->version);
    store_le64(job->header + 12, file->size);
    store_le64(job->header + 20, file->checksum);
    store_le64(job->header + 28, file->created_at);
    for (i = 0; i < file->name_length; ++i) job->header[36 + i] = file->name[i];
    job->header_length = header_length;
    job->header_offset = 0;
    job->state = 1;
    return 0;
}
static void trailer(ghostos_backup_job *job) {
    clear(job->header, sizeof job->header);
    job->header[0] = 'S'; job->header[1] = 'Y'; job->header[2] = 'N'; job->header[3] = 'B';
    job->header[4] = 'E'; job->header[5] = 'N'; job->header[6] = 'D'; job->header[7] = '1';
    store_le32(job->header + 8, job->files_streamed);
    job->header_length = 16;
    job->header_offset = 0;
    job->state = 3;
}
static int prepare_next(ghostos_backup_job *job, const ghostos_backup_file *files, size_t file_count) {
    if (job->file_index >= file_count) {
        trailer(job);
        return 0;
    }
    job->file_offset = 0;
    return file_header(job, &files[job->file_index]);
}
void ghostos_backup_start(ghostos_backup_job *job, uint64_t checkpoint_id, uint64_t generation) {
    job->checkpoint_id = checkpoint_id;
    job->generation = generation;
    job->file_offset = 0;
    job->bytes_streamed = 0;
    job->file_index = 0;
    job->files_streamed = 0;
    volume_header(job);
}
int ghostos_backup_poll(ghostos_backup_job *job, const ghostos_backup_file *files, size_t file_count,
    uint8_t *output, size_t output_capacity, size_t *written, size_t byte_budget) {
    size_t remaining = byte_budget;
    *written = 0;
    if (!byte_budget) return 1;
    while (remaining && job->state != 4) {
        if (job->state == 0 || job->state == 1 || job->state == 3) {
            size_t amount = job->header_length - job->header_offset;
            int status;
            if (amount > remaining) amount = remaining;
            status = append(output, output_capacity, written, job->header + job->header_offset, amount);
            if (status) return status;
            job->header_offset += amount;
            add_streamed(&job->bytes_streamed, amount);
            remaining -= amount;
            if (job->header_offset == job->header_length) {
                if (job->state == 0) {
                    status = prepare_next(job, files, file_count);
                    if (status) return status;
                } else if (job->state == 1) job->state = 2;
                else job->state = 4;
            }
        } else if (job->state == 2) {
            const ghostos_backup_file *file = &files[job->file_index];
            size_t available = file->size - (size_t)job->file_offset;
            size_t amount = remaining < 4096 ? remaining : 4096;
            int status;
            if (job->file_offset > file->size) return 4;
            if (amount > available) amount = available;
            if (!amount) {
                if (job->files_streamed != UINT32_MAX) ++job->files_streamed;
                if (job->file_index != UINT32_MAX) ++job->file_index;
                status = prepare_next(job, files, file_count);
                if (status) return status;
                continue;
            }
            status = append(output, output_capacity, written, file->data + (size_t)job->file_offset, amount);
            if (status) return status;
            job->file_offset += amount;
            add_streamed(&job->bytes_streamed, amount);
            remaining -= amount;
        }
    }
    return 0;
}
int ghostos_backup_finish(const ghostos_backup_job *job, uint32_t *files_streamed, uint64_t *bytes_streamed) {
    if (job->state != 4) return 3;
    *files_streamed = job->files_streamed;
    *bytes_streamed = job->bytes_streamed;
    return 0;
}
