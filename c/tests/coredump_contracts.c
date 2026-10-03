#include "ghostos/coredump.h"
#include <assert.h>
#include <string.h>
static void capture_matches_the_existing_core_layout(void) {
    const uint8_t directory[] = "/cores/3";
    const uint8_t payload[] = {9, 8, 7};
    const uint64_t registers[] = {1, 2};
    uint8_t metadata[GHOSTOS_CORE_METADATA_BYTES];
    uint8_t page[32];
    uint8_t name[13];
    uint8_t path[32];
    size_t written = 0;
    uint64_t process = 0;
    uint64_t timestamp = 0;
    uint64_t decoded[2] = {0};
    uint32_t pages = 0;
    uint8_t reason = 0;
    size_t count = 0;
    assert(!ghostos_core_directory(directory, sizeof directory - 1));
    assert(ghostos_core_directory((const uint8_t *)"/cores", 6) == 1);
    assert(ghostos_core_path((const uint8_t *)"/cores/../x", 11) == 1);
    assert(!ghostos_core_page_name(0, name));
    assert(!memcmp(name, "PAGE-00000000", 13));
    assert(!ghostos_core_child(directory, sizeof directory - 1, name, 13, path, sizeof path, &written));
    assert(written == 22 && !memcmp(path, "/cores/3/PAGE-00000000", 22));
    assert(!ghostos_core_metadata(3, 1, 99, 1, registers, 2, metadata));
    assert(!memcmp(metadata, "SYNCORE1", 8));
    assert(!ghostos_core_decode_metadata(metadata, sizeof metadata, &process, &reason, &timestamp, &pages, decoded, &count));
    assert(process == 3 && reason == 1 && timestamp == 99 && pages == 1 && count == 2);
    assert(decoded[0] == 1 && decoded[1] == 2);
    assert(!ghostos_core_page(0x4000, payload, 3, page, sizeof page, &written));
    assert(written == 19 && page[16] == 9 && page[17] == 8 && page[18] == 7);
}
int main(void) {
    capture_matches_the_existing_core_layout();
    return 0;
}
