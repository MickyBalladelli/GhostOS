#include "ghostos/fsd_namespace.h"
#include <assert.h>
static void host_mount_is_read_only_until_unmounted(void) {
    const uint8_t disk[] = {'/', 'm', 'n', 't', '/', 'd', 'i', 's', 'k'};
    const uint8_t file[] = {'/', 'm', 'n', 't', '/', 'd', 'i', 's', 'k', '/', 'f', 'i', 'l', 'e'};
    const uint8_t data[] = {'/', 'd', 'a', 't', 'a'};
    ghostos_fsd_namespace namespace;
    ghostos_fsd_mount mounts[6];
    uint64_t capability = 0;
    uint8_t volume = 9, mount_length = 0;
    bool read_only = false;
    ghostos_fsd_namespace_init(&namespace, mounts, 6);
    assert(ghostos_fsd_namespace_mount_host(&namespace, mounts, 6, GHOSTOS_FSD_HOST_AUTHORITY, disk, 9, 0, 0x1000, 0x2000, &capability) == 2);
    assert(!ghostos_fsd_namespace_activate(&namespace, mounts, 6, 9));
    assert(ghostos_fsd_namespace_mount_host(&namespace, mounts, 6, 1, disk, 9, 0, 0x1000, 0x2000, &capability) == 8);
    assert(ghostos_fsd_namespace_mount_host(&namespace, mounts, 6, GHOSTOS_FSD_HOST_AUTHORITY, disk, 9, 0, 0x1000, 0, &capability) == 5);
    assert(ghostos_fsd_namespace_mount_host(&namespace, mounts, 6, GHOSTOS_FSD_HOST_AUTHORITY, data, 5, 0, 0x1000, 0x2000, &capability) == 11);
    assert(!ghostos_fsd_namespace_mount_host(&namespace, mounts, 6, GHOSTOS_FSD_HOST_AUTHORITY, disk, 9, 0, 0x1000, 0x2000, &capability));
    assert(mounts[5].host && mounts[5].read_only && mounts[5].filesystem == 0 && mounts[5].partition_length == 0x2000);
    assert(!ghostos_fsd_namespace_resolve(&namespace, mounts, 6, file, 14, &volume, &read_only, &mount_length));
    assert(read_only && mount_length == 9);
    assert(!ghostos_fsd_namespace_unmount(mounts, 6, capability));
    assert(!ghostos_fsd_namespace_resolve(&namespace, mounts, 6, file, 14, &volume, &read_only, &mount_length));
    assert(volume == 0 && !read_only && mount_length == 1);
    assert(ghostos_fsd_namespace_unmount(mounts, 6, capability) == 7);
}
int main(void) {
    host_mount_is_read_only_until_unmounted();
    return 0;
}
