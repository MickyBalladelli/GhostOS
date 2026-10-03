#include "ghostos/dsm.h"
#include <assert.h>
#include <string.h>
static bool same_text(const uint8_t *bytes, size_t length, const char *text) {
    size_t i;
    for (i = 0; text[i]; ++i) if (i == length || bytes[i] != (uint8_t)text[i]) return false;
    return i == length;
}
static void packet_round_trip_and_page_reassembly(void) {
    const uint8_t fetch[] = {'f', 'e', 't', 'c', 'h'};
    uint8_t wire[64], page[GHOSTOS_DSM_PAGE], fragment[GHOSTOS_DSM_FRAGMENT];
    ghostos_dsm_assembler assembler;
    const uint8_t *payload = 0;
    uint8_t kind = 0;
    uint32_t source = 0, destination = 0, sequence = 0;
    uint64_t page_address = 0;
    size_t written = 0, payload_length = 0, fragment_length = 0, index;
    bool complete = false;
    int order[] = {2, 0, 1};
    assert(!ghostos_dsm_encode(1, 1, 2, 9, GHOSTOS_DSM_PAGE, 3, 0, 1, fetch, 5, wire, sizeof wire, &written));
    assert(wire[0] == 0x88 && wire[1] == 0xb5);
    assert(!ghostos_dsm_decode(wire, written, &kind, &source, &destination, &sequence, &page_address, &payload, &payload_length));
    assert(kind == 1 && source == 1 && destination == 2 && sequence == 9 && page_address == GHOSTOS_DSM_PAGE);
    assert(same_text(payload, payload_length, "fetch"));
    assert(ghostos_dsm_encode(1, 1, 2, 9, 1, 3, 0, 1, fetch, 5, wire, sizeof wire, &written) == 1);
    memset(page, 0, sizeof page);
    page[0] = 1;
    page[GHOSTOS_DSM_PAGE - 1] = 2;
    memset(&assembler, 0, sizeof assembler);
    assembler.page_address = GHOSTOS_DSM_PAGE;
    assembler.sequence = 10;
    for (index = 0; index < 3; ++index) {
        assert(!ghostos_dsm_page_fragment(2, 1, 10, GHOSTOS_DSM_PAGE, 4, (size_t)order[index], page, GHOSTOS_DSM_PAGE, fragment, sizeof fragment, &fragment_length));
        assert(!ghostos_dsm_push(&assembler, 2, GHOSTOS_DSM_PAGE, 10, (uint16_t)order[index], GHOSTOS_DSM_FRAGMENTS, fragment, fragment_length, &complete));
        assert(complete == (order[index] == 1));
    }
    assert(assembler.bytes[0] == 1 && assembler.bytes[GHOSTOS_DSM_PAGE - 1] == 2);
}
int main(void) {
    packet_round_trip_and_page_reassembly();
    return 0;
}
