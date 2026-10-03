#include "ghostos/fsd_list.h"
#include <assert.h>
#include <string.h>
static void listing_returns_first_then_second(void) {
    const uint8_t first[] = {'f', 'i', 'r', 's', 't'};
    const uint8_t second[] = {'s', 'e', 'c', 'o', 'n', 'd'};
    const uint8_t *names[] = {first, second};
    const uint8_t lengths[] = {5, 6};
    uint8_t output[28];
    size_t written = 0, next = 9;
    memset(output, 0, sizeof output);
    assert(ghostos_fsd_list(2, names, lengths, 2, 0, output, sizeof output, &written, &next) == 1);
    assert(!written);
    assert(!ghostos_fsd_list(1, names, lengths, 2, 0, output, sizeof output, &written, &next));
    assert(written > 0 && next == 1);
    assert(output[0] == 5 && output[1] == 0 && !memcmp(output + 22, "first", 5));
    assert(!ghostos_fsd_list(1, names, lengths, 2, 1, output, sizeof output, &written, &next));
    assert(next == 0 && output[0] == 6 && !memcmp(output + 22, "second", 6));
}
int main(void) {
    listing_returns_first_then_second();
    return 0;
}
