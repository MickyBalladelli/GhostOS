#include "ghostos/volume_record.h"
#include <assert.h>
#include <string.h>
static void record_round_trip_normalizes_type_and_links(void) {
    const uint8_t name[] = {'/', 's', 't', 'a', 't', 'e'};
    ghostos_volume_record record, decoded;
    uint8_t bytes[GHOSTOS_VOLUME_RECORD];
    size_t written = 0, consumed = 0, i;
    memset(&record, 0, sizeof record);
    for (i = 0; i < sizeof name; ++i) record.name[i] = name[i];
    record.name_length = sizeof name;
    record.version = 2;
    record.object_id = 7;
    record.size = 14;
    record.data = 3;
    record.checksum = 99;
    record.created_at = 5;
    record.mode = 0644;
    assert(!ghostos_volume_encode_record(&record, bytes, sizeof bytes, &written));
    assert(written == GHOSTOS_VOLUME_RECORD);
    assert(!ghostos_volume_decode_record(bytes, written, &decoded, &consumed));
    assert(consumed == written && decoded.version == 2 && decoded.object_id == 7 && decoded.size == 14);
    assert(decoded.file_type == 1 && decoded.link_count == 1 && decoded.mode == 0644);
    assert(decoded.name_length == sizeof name && !memcmp(decoded.name, name, sizeof name));
    bytes[2 + GHOSTOS_VOLUME_NAME + 4 + 8 + 8 + 4 + 8 + 8] = 2;
    assert(ghostos_volume_decode_record(bytes, written, &decoded, &consumed) == 1);
}
int main(void) {
    record_round_trip_normalizes_type_and_links();
    return 0;
}
