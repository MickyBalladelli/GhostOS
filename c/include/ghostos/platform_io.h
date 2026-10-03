#ifndef GHOSTOS_PLATFORM_IO_H
#define GHOSTOS_PLATFORM_IO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_MEDIA_PLANES 4

enum ghostos_io_error {
    GHOSTOS_IO_OK, GHOSTOS_IO_CANNOT_CANCEL, GHOSTOS_IO_EMPTY_BUFFER,
    GHOSTOS_IO_INVALID_BUFFER_ACCESS, GHOSTOS_IO_INVALID_DEVICE,
    GHOSTOS_IO_INVALID_FORMAT, GHOSTOS_IO_INVALID_PLANE_COUNT,
    GHOSTOS_IO_INVALID_TOKEN, GHOSTOS_IO_QUEUE_FULL,
    GHOSTOS_IO_REQUEST_NOT_DISPATCHED
};
enum ghostos_io_slot_state {
    GHOSTOS_IO_VACANT, GHOSTOS_IO_QUEUED, GHOSTOS_IO_DISPATCHED, GHOSTOS_IO_COMPLETED
};
enum ghostos_io_buffer_access {
    GHOSTOS_IO_READ_ONLY, GHOSTOS_IO_WRITE_ONLY, GHOSTOS_IO_READ_WRITE
};
enum ghostos_io_operation { GHOSTOS_IO_READ, GHOSTOS_IO_WRITE, GHOSTOS_IO_FLUSH, GHOSTOS_IO_CONTROL };
enum ghostos_media_operation { GHOSTOS_MEDIA_PRESENT, GHOSTOS_MEDIA_CAPTURE, GHOSTOS_MEDIA_ENCODE, GHOSTOS_MEDIA_DECODE };
enum ghostos_video_encoding { GHOSTOS_VIDEO_RGBA8888, GHOSTOS_VIDEO_BGRA8888, GHOSTOS_VIDEO_NV12, GHOSTOS_VIDEO_YUV420 };

typedef struct { uint32_t generation, state; } ghostos_io_slot;
typedef struct { size_t submit, dispatch, completion; } ghostos_io_cursors;
typedef struct {
    uint32_t region;
    uint64_t offset;
    uint32_t length;
    uint8_t access;
} ghostos_io_buffer;
/* Slots and cursors must initially be zeroed. Payload storage is caller-owned.
 * Successful selection writes the slot index and token; no pointers are retained.
 * Tokens use the historical high-32 generation / low-32 slot representation. */
uint32_t ghostos_io_submit(ghostos_io_slot *slots, size_t capacity,
    ghostos_io_cursors *cursors, size_t *index, uint64_t *token);
bool ghostos_io_dispatch(ghostos_io_slot *slots, size_t capacity,
    ghostos_io_cursors *cursors, size_t *index, uint64_t *token);
uint32_t ghostos_io_complete(ghostos_io_slot *slots, size_t capacity, uint64_t token);
bool ghostos_io_poll(ghostos_io_slot *slots, size_t capacity,
    ghostos_io_cursors *cursors, size_t *index, uint64_t *token);
uint32_t ghostos_io_cancel(ghostos_io_slot *slots, size_t capacity, uint64_t token);
size_t ghostos_io_pending(const ghostos_io_slot *slots, size_t capacity);
uint32_t ghostos_io_buffer_validate(const ghostos_io_buffer *buffer);
/* FLUSH ignores any supplied buffer, matching the existing contract. */
uint32_t ghostos_io_request_validate(uint32_t device, uint8_t operation,
    const ghostos_io_buffer *buffer);
/* Audio needs nonzero sample rate/channels; video needs nonzero width/height.
 * Encoding validation is provided by the caller's typed enum. */
uint32_t ghostos_media_format_validate(bool video, uint32_t first, uint32_t second);
/* Each plane has an independent presence flag. Plane count is checked before
 * any buffer validation; buffers are then validated in ascending plane order. */
uint32_t ghostos_media_packet_validate(uint32_t stream, uint8_t operation,
    bool video, uint8_t encoding, uint32_t first, uint32_t second,
    const ghostos_io_buffer planes[GHOSTOS_MAX_MEDIA_PLANES],
    const bool present[GHOSTOS_MAX_MEDIA_PLANES]);

#endif
