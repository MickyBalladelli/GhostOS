#include "ghostos/capability.h"
#include "ghostos/ipc.h"

#include <assert.h>

int main(void) {
    ghostos_capability_space caps;
    ghostos_capability_handle endpoint, memory;
    ghostos_channel_id channel_id;
    ghostos_shared_region_id region;
    ghostos_ipc_channel channel;
    ghostos_ipc_message message = {0}, received;
    uint64_t retry_after = 0;
    ghostos_capability_object endpoint_object, memory_object;
    ghostos_capability_space_init(&caps, 4, NULL, NULL);
    assert(ghostos_channel_id_new(8, &channel_id));
    assert(ghostos_shared_region_id_new(3, &region));
    endpoint_object = ghostos_capability_object_make(GHOSTOS_OBJECT_IPC_CHANNEL,
        0, channel_id.raw, 0, 0);
    memory_object = ghostos_capability_object_make(GHOSTOS_OBJECT_MEMORY_REGION,
        0, region.raw, 0, 0);
    assert(ghostos_capability_mint_root(&caps, 1, endpoint_object,
        GHOSTOS_RIGHT_SEND | GHOSTOS_RIGHT_RECEIVE, &endpoint) == GHOSTOS_CAP_OK);
    assert(ghostos_capability_mint_root(&caps, 1, memory_object,
        GHOSTOS_RIGHT_READ | GHOSTOS_RIGHT_WRITE | GHOSTOS_RIGHT_MAP, &memory) == GHOSTOS_CAP_OK);
    assert(ghostos_ipc_channel_init(&channel, channel_id, 2, NULL, NULL));
    message.label = 42;
    message.has_buffer = true;
    message.buffer = (ghostos_shared_buffer){region, 4, 8, false};
    message.words[0] = 1;
    assert(ghostos_ipc_try_send(&channel, &caps, 2, endpoint, true, memory,
        0, message, &retry_after) == GHOSTOS_IPC_ACCESS_DENIED);
    assert(ghostos_ipc_try_send(&channel, &caps, 1, endpoint, true, memory,
        0, message, &retry_after) == GHOSTOS_IPC_OK);
    assert(ghostos_ipc_try_send(&channel, &caps, 1, endpoint, true, memory,
        0, message, &retry_after) == GHOSTOS_IPC_OK);
    assert(ghostos_ipc_try_send(&channel, &caps, 1, endpoint, true, memory,
        0, message, &retry_after) == GHOSTOS_IPC_FULL);
    assert(ghostos_ipc_try_receive(&channel, &caps, 1, endpoint, 0,
        &received, &retry_after) == GHOSTOS_IPC_OK);
    assert(received.label == 42 && received.has_buffer);
    return 0;
}
