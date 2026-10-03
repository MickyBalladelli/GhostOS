#include "ghostos/platform_io.h"

_Static_assert(sizeof(ghostos_io_slot) == 8, "slot size");
_Static_assert(offsetof(ghostos_io_buffer, offset) == 8, "buffer offset");
_Static_assert(offsetof(ghostos_io_buffer, length) == 16, "buffer length");
_Static_assert(offsetof(ghostos_io_buffer, access) == 20, "buffer access");

static bool find_from(const ghostos_io_slot *slots, size_t capacity,
    size_t start, uint32_t state, size_t *index)
{
    if (capacity == 0) return false;
    for (size_t offset = 0; offset < capacity; ++offset) {
        size_t candidate = (start + offset) % capacity;
        if (slots[candidate].state == state) {
            *index = candidate;
            return true;
        }
    }
    return false;
}

static uint64_t slot_token(size_t index, uint32_t generation)
{
    return ((uint64_t)generation << 32) | (uint64_t)index;
}

static uint32_t validate_token(const ghostos_io_slot *slots, size_t capacity,
    uint64_t token, size_t *index)
{
    *index = (uint32_t)token;
    if (*index >= capacity || slots[*index].state == GHOSTOS_IO_VACANT ||
        slots[*index].generation != (uint32_t)(token >> 32))
        return GHOSTOS_IO_INVALID_TOKEN;
    return GHOSTOS_IO_OK;
}

uint32_t ghostos_io_submit(ghostos_io_slot *slots, size_t capacity,
    ghostos_io_cursors *cursors, size_t *index, uint64_t *token)
{
    if (!find_from(slots, capacity, cursors->submit, GHOSTOS_IO_VACANT, index))
        return GHOSTOS_IO_QUEUE_FULL;
    ghostos_io_slot *slot = &slots[*index];
    ++slot->generation;
    if (slot->generation == 0) slot->generation = 1;
    slot->state = GHOSTOS_IO_QUEUED;
    cursors->submit = (*index + 1) % capacity;
    *token = slot_token(*index, slot->generation);
    return GHOSTOS_IO_OK;
}

bool ghostos_io_dispatch(ghostos_io_slot *slots, size_t capacity,
    ghostos_io_cursors *cursors, size_t *index, uint64_t *token)
{
    if (!find_from(slots, capacity, cursors->dispatch, GHOSTOS_IO_QUEUED, index))
        return false;
    slots[*index].state = GHOSTOS_IO_DISPATCHED;
    cursors->dispatch = (*index + 1) % capacity;
    *token = slot_token(*index, slots[*index].generation);
    return true;
}

uint32_t ghostos_io_complete(ghostos_io_slot *slots, size_t capacity, uint64_t token)
{
    size_t index;
    uint32_t code = validate_token(slots, capacity, token, &index);
    if (code != GHOSTOS_IO_OK) return code;
    if (slots[index].state != GHOSTOS_IO_DISPATCHED)
        return GHOSTOS_IO_REQUEST_NOT_DISPATCHED;
    slots[index].state = GHOSTOS_IO_COMPLETED;
    return GHOSTOS_IO_OK;
}

bool ghostos_io_poll(ghostos_io_slot *slots, size_t capacity,
    ghostos_io_cursors *cursors, size_t *index, uint64_t *token)
{
    if (!find_from(slots, capacity, cursors->completion, GHOSTOS_IO_COMPLETED, index))
        return false;
    *token = slot_token(*index, slots[*index].generation);
    slots[*index].state = GHOSTOS_IO_VACANT;
    cursors->completion = (*index + 1) % capacity;
    return true;
}

uint32_t ghostos_io_cancel(ghostos_io_slot *slots, size_t capacity, uint64_t token)
{
    size_t index;
    uint32_t code = validate_token(slots, capacity, token, &index);
    if (code != GHOSTOS_IO_OK) return code;
    if (slots[index].state != GHOSTOS_IO_QUEUED) return GHOSTOS_IO_CANNOT_CANCEL;
    slots[index].state = GHOSTOS_IO_VACANT;
    return GHOSTOS_IO_OK;
}

size_t ghostos_io_pending(const ghostos_io_slot *slots, size_t capacity)
{
    size_t count = 0;
    for (size_t index = 0; index < capacity; ++index)
        if (slots[index].state != GHOSTOS_IO_VACANT) ++count;
    return count;
}

static bool readable(uint8_t access)
{
    return access == GHOSTOS_IO_READ_ONLY || access == GHOSTOS_IO_READ_WRITE;
}

static bool writable(uint8_t access)
{
    return access == GHOSTOS_IO_WRITE_ONLY || access == GHOSTOS_IO_READ_WRITE;
}

uint32_t ghostos_io_buffer_validate(const ghostos_io_buffer *buffer)
{
    if (buffer->region == 0 || buffer->length == 0 ||
        buffer->length > UINT64_MAX - buffer->offset) return GHOSTOS_IO_EMPTY_BUFFER;
    return GHOSTOS_IO_OK;
}

uint32_t ghostos_io_request_validate(uint32_t device, uint8_t operation,
    const ghostos_io_buffer *buffer)
{
    if (device == 0) return GHOSTOS_IO_INVALID_DEVICE;
    if (operation == GHOSTOS_IO_FLUSH) return GHOSTOS_IO_OK;
    if (buffer == NULL) return operation == GHOSTOS_IO_CONTROL ?
        GHOSTOS_IO_OK : GHOSTOS_IO_EMPTY_BUFFER;
    uint32_t code = ghostos_io_buffer_validate(buffer);
    if (code != GHOSTOS_IO_OK) return code;
    if ((operation == GHOSTOS_IO_READ && !writable(buffer->access)) ||
        (operation == GHOSTOS_IO_WRITE && !readable(buffer->access)))
        return GHOSTOS_IO_INVALID_BUFFER_ACCESS;
    return GHOSTOS_IO_OK;
}

uint32_t ghostos_media_format_validate(bool video, uint32_t first, uint32_t second)
{
    (void)video;
    return first == 0 || second == 0 ? GHOSTOS_IO_INVALID_FORMAT : GHOSTOS_IO_OK;
}

uint32_t ghostos_media_packet_validate(uint32_t stream, uint8_t operation,
    bool video, uint8_t encoding, uint32_t first, uint32_t second,
    const ghostos_io_buffer planes[GHOSTOS_MAX_MEDIA_PLANES],
    const bool present[GHOSTOS_MAX_MEDIA_PLANES])
{
    uint32_t code = ghostos_media_format_validate(video, first, second);
    if (code != GHOSTOS_IO_OK) return code;
    if (stream == 0) return GHOSTOS_IO_INVALID_DEVICE;
    size_t expected = !video || encoding <= GHOSTOS_VIDEO_BGRA8888 ? 1 :
        encoding == GHOSTOS_VIDEO_NV12 ? 2 : 3;
    for (size_t plane = 0; plane < GHOSTOS_MAX_MEDIA_PLANES; ++plane)
        if (present[plane] != (plane < expected)) return GHOSTOS_IO_INVALID_PLANE_COUNT;
    for (size_t plane = 0; plane < expected; ++plane) {
        code = ghostos_io_buffer_validate(&planes[plane]);
        if (code != GHOSTOS_IO_OK) return code;
        bool valid = operation == GHOSTOS_MEDIA_PRESENT || operation == GHOSTOS_MEDIA_ENCODE ?
            readable(planes[plane].access) : writable(planes[plane].access);
        if (!valid) return GHOSTOS_IO_INVALID_BUFFER_ACCESS;
    }
    return GHOSTOS_IO_OK;
}
