#include "ghostos/fsd_links.h"
static bool same_name(const ghostos_fsd_record *left, const ghostos_fsd_record *right) {
    size_t i;
    if (left->name_length != right->name_length) return false;
    for (i = 0; i < left->name_length; ++i) if (left->name[i] != right->name[i]) return false;
    return true;
}
static int name_order(const ghostos_fsd_record *left, const ghostos_fsd_record *right) {
    size_t i, length = left->name_length < right->name_length ? left->name_length : right->name_length;
    for (i = 0; i < length; ++i) if (left->name[i] != right->name[i]) return left->name[i] < right->name[i] ? -1 : 1;
    if (left->name_length == right->name_length) return 0;
    return left->name_length < right->name_length ? -1 : 1;
}
static bool latest(const ghostos_fsd_record *records, size_t count, size_t index) {
    size_t i;
    if (records[index].deleted) return false;
    for (i = 0; i < count; ++i) {
        if (i == index || records[i].deleted || !same_name(&records[i], &records[index])) continue;
        if (records[i].version > records[index].version) return false;
    }
    return true;
}
int ghostos_fsd_list_links(uint16_t rights, const uint8_t *pattern, size_t pattern_length, const ghostos_fsd_record *records,
    size_t record_count, size_t *matches, size_t match_capacity, uint8_t *output, size_t output_capacity, size_t *written) {
    size_t count = 0, next = 0, i, link_count = 0, required = 0, cursor = 0;
    size_t links[64];
    bool has_next = false;
    int status;
    if ((rights & 1u) == 0) return 1;
    status = ghostos_fsd_expand(pattern, pattern_length, records, record_count, 0, matches, match_capacity, &count, &next, &has_next);
    if (status == 1) return 2;
    if (status == 2) return 3;
    if (status == 3) return count ? 6 : 5;
    if (status == 4) return 4;
    if (status) return status;
    if (has_next) return 6;
    for (i = 0; i < count; ++i) {
        uint64_t object = records[matches[i]].object_id;
        size_t record;
        for (record = 0; record < record_count; ++record) {
            size_t slot;
            bool seen = false;
            if (!latest(records, record_count, record) || records[record].object_id != object) continue;
            for (slot = 0; slot < link_count; ++slot) if (same_name(&records[links[slot]], &records[record])) seen = true;
            if (seen) continue;
            if (link_count == 64) return 5;
            slot = link_count;
            while (slot > 0 && name_order(&records[record], &records[links[slot - 1]]) < 0) {
                links[slot] = links[slot - 1];
                slot -= 1;
            }
            links[slot] = record;
            link_count += 1;
        }
    }
    if (!link_count) return 4;
    for (i = 0; i < link_count; ++i) {
        if (required > SIZE_MAX - records[links[i]].name_length - 1) return 5;
        required += records[links[i]].name_length + 1;
    }
    if (required > output_capacity) return 5;
    for (i = 0; i < link_count; ++i) {
        size_t byte;
        for (byte = 0; byte < records[links[i]].name_length; ++byte) output[cursor + byte] = records[links[i]].name[byte];
        cursor += records[links[i]].name_length;
        output[cursor] = '\n';
        cursor += 1;
    }
    *written = cursor;
    return 0;
}
