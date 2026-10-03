#include "ghostos/actors.h"
#include <assert.h>
static ghostos_actor_entry entry(uint32_t node, uint64_t local, uint8_t kind, uint32_t endpoint) {
    ghostos_actor_entry value = {0};
    value.local = local;
    value.node = node;
    value.endpoint = endpoint;
    value.generation = 1;
    value.kind = kind;
    return value;
}
static void messages_reject_corrupt_identity(void) {
    assert(ghostos_actor_decode_words(1, 1, 2, 2) == 0);
    assert(ghostos_actor_decode_words(1, 0, 2, 2) == 3);
    assert(ghostos_actor_decode_words(0x100000000ULL, 1, 2, 2) == 3);
    assert(ghostos_actor_decode_words(0x100000001ULL, 1, 2, 2) == 3);
}
static void mailbox_and_directory_follow_existing_order(void) {
    ghostos_actor_entry directory[4] = {0};
    ghostos_actor_route routes[2] = {0};
    size_t index = 99;
    assert(ghostos_actor_validate_mailbox(2, 4096, 4096, 1, 4096, 4096, true, 1, 100, 1, 10, true) == 0);
    assert(ghostos_actor_validate_mailbox(2, 4096, 4096, 1, 4096, 4096, true, 1, 100, 1, 100, true) == 6);
    assert(ghostos_actor_register(directory, 4, 1, 1, 1, 7, 1, 1, &index) == 0);
    assert(index == 0);
    directory[0] = entry(1, 1, 1, 7);
    assert(ghostos_actor_register(directory, 4, 2, 2, 2, 2, 1, 1, &index) == 0);
    assert(index == 1);
    directory[1] = entry(2, 2, 2, 2);
    assert(ghostos_actor_find(directory, 4, 2, 2, &index) == 0);
    directory[1].kind = 0;
    assert(ghostos_actor_find(directory, 4, 2, 2, &index) == 8);
    assert(ghostos_actor_add_route(routes, 2, 2, 4096, 4096, 1, 4096, 4096, true, 1, 100, 1, 10, true, &index) == 0);
    assert(index == 0);
    routes[0].occupied = true;
    routes[0].node = 2;
    assert(ghostos_actor_prepare(directory, 4, routes, 2, 2, 3, 0, 9, 1, 1, &index, &index, &(bool){false}) == 0);
    assert(ghostos_actor_prepare(directory, 4, routes, 2, 3, 4, 0, 9, 1, 1, &index, &index, &(bool){false}) == 7);
    assert(ghostos_actor_prepare(directory, 4, routes, 2, 1, 0, 0, 9, 1, 1, &index, &index, &(bool){false}) == 4);
}
int main(void) {
    messages_reject_corrupt_identity();
    mailbox_and_directory_follow_existing_order();
    return 0;
}
