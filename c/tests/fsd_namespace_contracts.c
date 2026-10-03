#include "ghostos/fsd_namespace.h"
#include <assert.h>
static void data_resolves_under_its_mount_and_cannot_be_removed(void) {
    ghostos_fsd_namespace namespace;
    ghostos_fsd_mount mounts[5];
    uint8_t volume = 9, mount_length = 0;
    bool read_only = true;
    ghostos_fsd_namespace_init(&namespace, mounts, 4);
    assert(ghostos_fsd_namespace_activate(&namespace, mounts, 4, 9) == 3);
    assert(!namespace.active);
    ghostos_fsd_namespace_init(&namespace, mounts, 5);
    assert(ghostos_fsd_namespace_resolve(&namespace, mounts, 5, (const uint8_t *)"/data/file", 10, &volume, &read_only, &mount_length) == 2);
    assert(!ghostos_fsd_namespace_activate(&namespace, mounts, 5, 9));
    assert(namespace.active && namespace.authority == GHOSTOS_FSD_HOST_AUTHORITY);
    assert(ghostos_fsd_namespace_activate(&namespace, mounts, 5, 9) == 1);
    assert(!ghostos_fsd_namespace_resolve(&namespace, mounts, 5, (const uint8_t *)"/data/file", 10, &volume, &read_only, &mount_length));
    assert(volume == 3 && !read_only && mount_length == 5);
    assert(!ghostos_fsd_namespace_resolve(&namespace, mounts, 5, (const uint8_t *)"/packages/pkg", 13, &volume, &read_only, &mount_length));
    assert(volume == 1 && read_only);
    assert(!ghostos_fsd_namespace_resolve(&namespace, mounts, 5, (const uint8_t *)"/dat", 4, &volume, &read_only, &mount_length));
    assert(volume == 0 && mount_length == 1);
    assert(ghostos_fsd_namespace_resolve(&namespace, mounts, 5, (const uint8_t *)"data", 4, &volume, &read_only, &mount_length) == 4);
    assert(ghostos_fsd_namespace_unmount(mounts, 5, (1ull << 32) | 1) == 9);
    assert(ghostos_fsd_remove_directory(&namespace, mounts, 5, (const uint8_t *)"/data", 5, 14) == 8);
    assert(ghostos_fsd_remove_directory(&namespace, mounts, 5, (const uint8_t *)"/data/empty", 11, 4) == 8);
    assert(!ghostos_fsd_remove_directory(&namespace, mounts, 5, (const uint8_t *)"/data/empty", 11, 14));
}
int main(void) {
    data_resolves_under_its_mount_and_cannot_be_removed();
    return 0;
}
