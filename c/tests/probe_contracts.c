#include "ghostos/probes.h"
#include <assert.h>
static ghostos_probe_instruction instruction(uint8_t opcode, uint8_t destination, uint8_t source, int64_t immediate) {
    ghostos_probe_instruction value = {opcode, destination, source, immediate};
    return value;
}
static void arithmetic_runs_and_backward_jumps_are_rejected(void) {
    ghostos_probe_program program;
    ghostos_probe_sample sample = {7, 1, 4, 40, 2, 0, 0, 1};
    uint64_t emitted[2];
    size_t count = 0;
    uint64_t values[2] = {0};
    bool occupied[2] = {false, false};
    size_t next = 0;
    uint64_t dropped = 0;
    ghostos_probe_program_init(&program);
    assert(!ghostos_probe_push(&program, instruction(1, 0, 0, 4)));
    assert(!ghostos_probe_push(&program, instruction(2, 1, 0, 2)));
    assert(!ghostos_probe_push(&program, instruction(3, 0, 1, 0)));
    assert(!ghostos_probe_push(&program, instruction(11, 0, 0, 0)));
    assert(!ghostos_probe_push(&program, instruction(12, 0, 0, 0)));
    assert(!ghostos_probe_verify(&program));
    assert(!ghostos_probe_run(&program, sample, emitted, 2, &count));
    assert(count == 1 && emitted[0] == 42);
    ghostos_probe_program_init(&program);
    assert(!ghostos_probe_push(&program, instruction(10, 0, 0, 0)));
    assert(!ghostos_probe_push(&program, instruction(12, 0, 0, 0)));
    assert(ghostos_probe_verify(&program) == 2);
    assert(!ghostos_probe_record(values, occupied, 2, &next, &dropped, 1));
    assert(!ghostos_probe_record(values, occupied, 2, &next, &dropped, 2));
    assert(!ghostos_probe_record(values, occupied, 2, &next, &dropped, 3));
    assert(dropped == 1 && values[0] == 3 && next == 1);
    assert(ghostos_probe_record(values, occupied, 0, &next, &dropped, 4) == 1);
}
int main(void) {
    arithmetic_runs_and_backward_jumps_are_rejected();
    return 0;
}
