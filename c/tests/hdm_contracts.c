#include "ghostos/hdm.h"
#include <assert.h>
#include <string.h>
static uint32_t load32(const uint8_t *bytes) {
    return (uint32_t)bytes[0] | ((uint32_t)bytes[1] << 8) | ((uint32_t)bytes[2] << 16) | ((uint32_t)bytes[3] << 24);
}
static void programming_rejects_a_committed_decoder_before_replacing_it(void) {
    uint8_t registers[0x30];
    uint8_t count = 0;
    memset(registers, 0, sizeof registers);
    registers[0] = 6;
    assert(ghostos_hdm_decoder_count(registers, sizeof registers, 0, &count) == 2);
    registers[0] = 0;
    assert(!ghostos_hdm_decoder_count(registers, sizeof registers, 0, &count));
    assert(count == 1);
    assert(ghostos_hdm_program(registers, sizeof registers, 0, 0, 1, GHOSTOS_HDM_GRANULARITY, 0, 0, 0, true, 0) == 1);
    registers[0x20] = (uint8_t)(1u << 2);
    registers[0x21] = (uint8_t)(1u << 2);
    assert(ghostos_hdm_program(registers, sizeof registers, 0, 0, GHOSTOS_HDM_GRANULARITY, GHOSTOS_HDM_GRANULARITY, 0, 0, 0, true, 0) == 4);
    assert(!load32(registers + 0x10));
    memset(registers + 0x20, 0, 4);
    assert(ghostos_hdm_program(registers, sizeof registers, 0, 0, GHOSTOS_HDM_GRANULARITY, GHOSTOS_HDM_GRANULARITY, 0, 1, 2, true, 0) == 5);
    assert(load32(registers + 0x10) == (uint32_t)GHOSTOS_HDM_GRANULARITY);
    assert(load32(registers + 0x18) == (uint32_t)GHOSTOS_HDM_GRANULARITY);
    assert(load32(registers + 0x20) == ((1u << 9) | (1u << 12) | 1u | (2u << 4)));
    assert(ghostos_hdm_program(registers, sizeof registers, 0, 1, GHOSTOS_HDM_GRANULARITY, GHOSTOS_HDM_GRANULARITY, 0, 0, 0, false, 0) == 3);
}
int main(void) {
    programming_rejects_a_committed_decoder_before_replacing_it();
    return 0;
}
