#include "ghostos/probes.h"
static int field_value(ghostos_probe_sample sample, int64_t field, uint64_t *value) {
    switch (field) {
    case 1: *value = sample.timestamp_us; return 0;
    case 2: *value = sample.node; return 0;
    case 3: *value = sample.subject; return 0;
    case 4: *value = sample.value0; return 0;
    case 5: *value = sample.value1; return 0;
    case 6: *value = sample.value2; return 0;
    case 7: *value = sample.value3; return 0;
    case 8: *value = sample.kind; return 0;
    default: return 2;
    }
}
void ghostos_probe_program_init(ghostos_probe_program *program) {
    size_t i;
    for (i = 0; i < GHOSTOS_PROBE_INSTRUCTIONS; ++i) {
        program->instructions[i].opcode = 12;
        program->instructions[i].destination = 0;
        program->instructions[i].source = 0;
        program->instructions[i].immediate = 0;
    }
    program->len = 0;
}
int ghostos_probe_push(ghostos_probe_program *program, ghostos_probe_instruction instruction) {
    if (program->len == GHOSTOS_PROBE_INSTRUCTIONS) return 1;
    program->instructions[program->len++] = instruction;
    return 0;
}
int ghostos_probe_verify(const ghostos_probe_program *program) {
    size_t index;
    if (!program->len || program->instructions[program->len - 1].opcode != 12) return 2;
    for (index = 0; index < program->len; ++index) {
        const ghostos_probe_instruction *instruction = &program->instructions[index];
        uint64_t loaded = 0;
        if (instruction->destination >= GHOSTOS_PROBE_REGISTERS) return 2;
        if (instruction->opcode >= 3 && instruction->opcode <= 9 &&
            instruction->source >= GHOSTOS_PROBE_REGISTERS) return 2;
        if (instruction->opcode == 1 && field_value((ghostos_probe_sample){0}, instruction->immediate, &loaded))
            return 2;
        if (instruction->opcode == 10) {
            if (instruction->immediate < 0 || (uint64_t)instruction->immediate > SIZE_MAX) return 2;
            if ((size_t)instruction->immediate <= index || (size_t)instruction->immediate >= program->len)
                return 2;
        }
    }
    return 0;
}
int ghostos_probe_run(const ghostos_probe_program *program, ghostos_probe_sample sample,
    uint64_t *emitted, size_t capacity, size_t *count) {
    uint64_t registers[GHOSTOS_PROBE_REGISTERS] = {0};
    size_t pc = 0;
    size_t steps = 0;
    *count = 0;
    while (pc < program->len && steps < GHOSTOS_PROBE_INSTRUCTIONS) {
        const ghostos_probe_instruction *instruction = &program->instructions[pc];
        size_t destination = instruction->destination;
        size_t source = instruction->source;
        uint64_t loaded = 0;
        ++steps;
        if (destination >= GHOSTOS_PROBE_REGISTERS) return 2;
        switch (instruction->opcode) {
        case 1:
            if (field_value(sample, instruction->immediate, &loaded)) return 2;
            registers[destination] = loaded;
            break;
        case 2:
            registers[destination] = (uint64_t)instruction->immediate;
            break;
        case 3: case 4: case 5: case 6: case 7: case 8: case 9:
            if (source >= GHOSTOS_PROBE_REGISTERS) return 2;
            if (instruction->opcode == 3) registers[destination] += registers[source];
            else if (instruction->opcode == 4) registers[destination] -= registers[source];
            else if (instruction->opcode == 5) registers[destination] &= registers[source];
            else if (instruction->opcode == 6) registers[destination] |= registers[source];
            else if (instruction->opcode == 7) registers[destination] ^= registers[source];
            else if (instruction->opcode == 8)
                registers[destination] = registers[destination] == registers[source];
            else registers[destination] = registers[destination] > registers[source];
            break;
        case 10:
            if (!registers[destination]) {
                pc = (size_t)instruction->immediate;
                continue;
            }
            break;
        case 11:
            if (*count == capacity) return 1;
            emitted[(*count)++] = registers[0];
            break;
        case 12:
            return 0;
        default:
            return 2;
        }
        ++pc;
    }
    return 2;
}
int ghostos_probe_record(uint64_t *values, bool *occupied, size_t capacity, size_t *next,
    uint64_t *dropped, uint64_t value) {
    if (!capacity) return 1;
    if (occupied[*next]) {
        if (*dropped < UINT64_MAX) ++*dropped;
    }
    values[*next] = value;
    occupied[*next] = true;
    *next = (*next + 1) % capacity;
    return 0;
}
