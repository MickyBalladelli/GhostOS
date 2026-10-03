#ifndef GHOSTOS_PROBES_H
#define GHOSTOS_PROBES_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 capacity, 2 invalid program.
 * Opcode: load field=1, immediate=2, add=3, subtract=4, and=5, or=6, xor=7,
 * equal=8, greater=9, jump if zero=10, emit=11, halt=12.
 * Field: timestamp=1, node=2, subject=3, value0=4, value1=5, value2=6, value3=7, kind=8. */
#define GHOSTOS_PROBE_INSTRUCTIONS 64u
#define GHOSTOS_PROBE_REGISTERS 8u
typedef struct {
    uint8_t opcode, destination, source;
    int64_t immediate;
} ghostos_probe_instruction;
typedef struct {
    ghostos_probe_instruction instructions[GHOSTOS_PROBE_INSTRUCTIONS];
    uint8_t len;
} ghostos_probe_program;
typedef struct {
    uint64_t timestamp_us, node, subject, value0, value1, value2, value3;
    uint8_t kind;
} ghostos_probe_sample;
void ghostos_probe_program_init(ghostos_probe_program *program);
int ghostos_probe_push(ghostos_probe_program *program, ghostos_probe_instruction instruction);
int ghostos_probe_verify(const ghostos_probe_program *program);
int ghostos_probe_run(const ghostos_probe_program *program, ghostos_probe_sample sample,
    uint64_t *emitted, size_t capacity, size_t *count);
int ghostos_probe_record(uint64_t *values, bool *occupied, size_t capacity, size_t *next,
    uint64_t *dropped, uint64_t value);
#endif
