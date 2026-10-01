#ifndef GHOSTOS_PERSONA_H
#define GHOSTOS_PERSONA_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_MAX_PERSONA_RIGHTS 16u

typedef enum {
    GHOSTOS_PERSONA_OK = 0,
    GHOSTOS_PERSONA_FULL = 1,
    GHOSTOS_PERSONA_NOT_FOUND = 2
} ghostos_persona_error;

typedef struct {
    uint64_t identity;
    uint64_t active[GHOSTOS_MAX_PERSONA_RIGHTS];
    uint64_t disabled[GHOSTOS_MAX_PERSONA_RIGHTS];
} ghostos_execution_persona;

void ghostos_persona_init(ghostos_execution_persona *persona, uint64_t identity);
bool ghostos_persona_add(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error);
bool ghostos_persona_disable(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error);
bool ghostos_persona_enable(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error);
bool ghostos_persona_drop(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error);
bool ghostos_persona_has(const ghostos_execution_persona *persona, uint64_t right);
size_t ghostos_persona_active_count(const ghostos_execution_persona *persona);
uint64_t ghostos_persona_active_at(const ghostos_execution_persona *persona, size_t index);

#endif
