#include "ghostos/volume_version.h"
#include <assert.h>
#include <string.h>
static void exact_log_version_returns_old(void) {
    const uint8_t name[] = {'/', 'd', 'a', 't', 'a', '/', 'a', 'r', 'c', 'h', 'i', 'v', 'e', '/', 'l', 'o', 'g'};
    const uint8_t old_bytes[] = {'o', 'l', 'd'};
    const uint8_t new_bytes[] = {'n', 'e', 'w'};
    ghostos_volume_exact files[3];
    uint8_t output[3];
    size_t read = 0;
    memset(files, 0, sizeof files);
    files[0].name = name;
    files[0].name_length = sizeof name;
    files[0].data = old_bytes;
    files[0].data_length = 3;
    files[0].file_type = 1;
    files[0].version = 1;
    files[1] = files[0];
    files[1].data = new_bytes;
    files[1].version = 2;
    files[2] = files[0];
    files[2].file_type = 2;
    files[2].version = 1;
    files[2].name_length = 13;
    assert(ghostos_volume_read_version(files, 2, name, sizeof name, 0, output, sizeof output, &read) == 1);
    assert(!ghostos_volume_read_version(files, 2, name, sizeof name, 1, output, sizeof output, &read));
    assert(read == 3 && !memcmp(output, "old", 3));
    assert(!ghostos_volume_read_version(files, 2, name, sizeof name, 2, output, sizeof output, &read));
    assert(!memcmp(output, "new", 3));
    assert(ghostos_volume_read_version(files, 2, name, sizeof name, 1, output, 2, &read) == 3);
    assert(ghostos_volume_read_version(files, 2, name, sizeof name, 3, output, sizeof output, &read) == 2);
    files[0].deleted = true;
    assert(ghostos_volume_read_version(files, 2, name, sizeof name, 1, output, sizeof output, &read) == 2);
    assert(ghostos_volume_read_version(files, 3, name, 13, 1, output, sizeof output, &read) == 4);
}
int main(void) {
    exact_log_version_returns_old();
    return 0;
}
