#include "ghostos/vm_execution.h"

static bool mnemonic_equal(const uint8_t *input, size_t length, const char *name) {
    size_t i = 0;
    while (name[i] != '\0') {
        if (i >= length || input[i] != (uint8_t)name[i]) return false;
        ++i;
    }
    return i == length;
}

bool ghostos_vm_execution_boundary(const uint8_t *input, size_t length) {
    static const char *const names[] = {
        "JMP", "JCC", "CALL", "CALLF", "RET", "RETF", "INT", "INT3",
        "IRET", "HLT", "SYSCALL", "SYSRET", "SYSENTER", "SYSEXIT",
        "LOOP", "LOOPE", "LOOPNE", "JRCXZ", "WRMSR", "RDMSR", "IN", "OUT",
        "INSB", "INSW", "INSD", "INSQ", "OUTSB", "OUTSW", "OUTSD", "OUTSQ", "STI", "CLI"
    };
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i)
        if (mnemonic_equal(input, length, names[i])) return true;
    return false;
}

bool ghostos_vm_execution_loop(uint64_t start, const ghostos_vm_execution_instruction *last, bool relative, uint64_t displacement) {
    static const char *const names[] = { "JMP", "JCC", "LOOP", "LOOPE", "LOOPNE", "JRCXZ" };
    if (last == NULL || !relative) return false;
    for (size_t i = 0; i < sizeof(names) / sizeof(names[0]); ++i)
        if (mnemonic_equal(last->mnemonic, last->mnemonic_length, names[i]))
            return last->next_ip + displacement <= start;
    return false;
}

bool ghostos_vm_execution_source_range(uint64_t start, size_t bytes, uint64_t ip, uint64_t next_ip, size_t *offset, size_t *length) {
    if (ip < start) return false;
    size_t begin = (size_t)(ip - start);
    size_t count = (size_t)(next_ip < ip ? 0 : next_ip - ip);
    size_t end = count > SIZE_MAX - begin ? SIZE_MAX : begin + count;
    if (end > bytes) return false;
    *offset = begin;
    /* Rust's saturating range end can shorten an overflowing range. */
    *length = end - begin;
    return true;
}

bool ghostos_vm_execution_observe_version(uint64_t *observed, uint64_t current) {
    bool changed = *observed != current;
    *observed = current;
    return changed;
}

bool ghostos_vm_execution_promote(bool enabled, bool loop, bool compiled, uint64_t *hot, uint64_t threshold) {
    if (*hot != UINT64_MAX) ++*hot;
    return enabled && loop && !compiled && *hot >= threshold;
}

bool ghostos_vm_execution_retune(uint64_t hits, uint64_t misses) {
    uint64_t samples = misses > UINT64_MAX - hits ? UINT64_MAX : hits + misses;
    return samples != 0 && samples % 64 == 0;
}

bool ghostos_vm_execution_run(const ghostos_vm_execution_host *host, void *context, size_t count, size_t maximum, size_t *executed, bool *clear_cache) {
    ghostos_vm_execution_state initial, state;
    host->state(context, &initial);
    *executed = 0;
    *clear_cache = false;
    size_t limit = count < maximum ? count : maximum;
    for (size_t i = 0; i < limit; ++i) {
        ghostos_vm_execution_instruction instruction;
        host->instruction(context, i, &instruction);
        host->state(context, &state);
        if (state.halted || state.rip != instruction.ip) break;
        if (state.code_version != initial.code_version && !host->source_valid(context, i)) break;
        if (state.translation_version != initial.translation_version) break;
        uint32_t mode = state.mode;
        if (!host->step(context, i)) return false;
        ++*executed;
        host->record(context, i);
        host->state(context, &state);
        if (state.mode != mode || ghostos_vm_execution_boundary(instruction.mnemonic, instruction.mnemonic_length)) break;
    }
    host->state(context, &state);
    *clear_cache = *executed == 0 && !state.halted;
    return true;
}

bool ghostos_vm_execution_translate(const ghostos_vm_translation_host *host, void *context, uint64_t start, size_t maximum) {
    size_t limit = maximum == 0 ? 1 : maximum;
    uint64_t ip = start;
    for (size_t i = 0; i < limit; ++i) {
        ghostos_vm_execution_instruction instruction;
        if (!host->decode(context, ip, &instruction)) {
            if (i == 0) return false;
            break;
        }
        ip = instruction.next_ip;
        if (ghostos_vm_execution_boundary(instruction.mnemonic, instruction.mnemonic_length)) break;
    }
    return host->finish(context, start, (size_t)(ip - start));
}
