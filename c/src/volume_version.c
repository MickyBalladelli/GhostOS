#include "ghostos/volume_version.h"
static bool same_name(const ghostos_volume_exact *file, const uint8_t *name, size_t name_length) {
    size_t i;
    if (file->name_length != name_length) return false;
    for (i = 0; i < name_length; ++i) if (file->name[i] != name[i]) return false;
    return true;
}
int ghostos_volume_read_version(const ghostos_volume_exact *files, size_t count, const uint8_t *name, size_t name_length, uint32_t version, uint8_t *output, size_t output_capacity, size_t *read) {
    size_t i, copied;
    if (!version) return 1;
    for (i = 0; i < count; ++i) {
        if (files[i].deleted || files[i].version != version || !same_name(&files[i], name, name_length)) continue;
        if (files[i].file_type == 2) return 4;
        if (files[i].data_length > output_capacity) return 3;
        for (copied = 0; copied < files[i].data_length; ++copied) output[copied] = files[i].data[copied];
        *read = files[i].data_length;
        return 0;
    }
    return 2;
}
