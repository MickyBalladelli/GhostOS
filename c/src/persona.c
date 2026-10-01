#include "ghostos/persona.h"

static void set_error(ghostos_persona_error *error, ghostos_persona_error value) {
    if (error) *error = value;
}

static size_t find(const uint64_t entries[GHOSTOS_MAX_PERSONA_RIGHTS], uint64_t right) {
    for (size_t index = 0; index < GHOSTOS_MAX_PERSONA_RIGHTS; ++index) {
        if (entries[index] == right) return index;
    }
    return GHOSTOS_MAX_PERSONA_RIGHTS;
}

static size_t empty(const uint64_t entries[GHOSTOS_MAX_PERSONA_RIGHTS]) {
    return find(entries, 0);
}

void ghostos_persona_init(ghostos_execution_persona *persona, uint64_t identity) {
    if (!persona) return;
    persona->identity = identity;
    for (size_t index = 0; index < GHOSTOS_MAX_PERSONA_RIGHTS; ++index) {
        persona->active[index] = 0;
        persona->disabled[index] = 0;
    }
}

bool ghostos_persona_add(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error) {
    if (!persona || !right) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
    if (find(persona->active, right) < GHOSTOS_MAX_PERSONA_RIGHTS ||
        find(persona->disabled, right) < GHOSTOS_MAX_PERSONA_RIGHTS) {
        set_error(error, GHOSTOS_PERSONA_OK);
        return true;
    }
    size_t slot = empty(persona->active);
    if (slot == GHOSTOS_MAX_PERSONA_RIGHTS) { set_error(error, GHOSTOS_PERSONA_FULL); return false; }
    persona->active[slot] = right;
    set_error(error, GHOSTOS_PERSONA_OK);
    return true;
}

bool ghostos_persona_disable(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error) {
    if (!persona || !right) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
    size_t active = find(persona->active, right);
    if (active == GHOSTOS_MAX_PERSONA_RIGHTS) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
    size_t disabled = empty(persona->disabled);
    if (disabled == GHOSTOS_MAX_PERSONA_RIGHTS) { set_error(error, GHOSTOS_PERSONA_FULL); return false; }
    persona->active[active] = 0;
    persona->disabled[disabled] = right;
    set_error(error, GHOSTOS_PERSONA_OK);
    return true;
}

bool ghostos_persona_enable(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error) {
    if (!persona || !right) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
    size_t disabled = find(persona->disabled, right);
    if (disabled == GHOSTOS_MAX_PERSONA_RIGHTS) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
    size_t active = empty(persona->active);
    if (active == GHOSTOS_MAX_PERSONA_RIGHTS) { set_error(error, GHOSTOS_PERSONA_FULL); return false; }
    persona->disabled[disabled] = 0;
    persona->active[active] = right;
    set_error(error, GHOSTOS_PERSONA_OK);
    return true;
}

bool ghostos_persona_drop(ghostos_execution_persona *persona, uint64_t right, ghostos_persona_error *error) {
    if (!persona || !right) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
    size_t slot = find(persona->active, right);
    if (slot < GHOSTOS_MAX_PERSONA_RIGHTS) persona->active[slot] = 0;
    else {
        slot = find(persona->disabled, right);
        if (slot == GHOSTOS_MAX_PERSONA_RIGHTS) { set_error(error, GHOSTOS_PERSONA_NOT_FOUND); return false; }
        persona->disabled[slot] = 0;
    }
    set_error(error, GHOSTOS_PERSONA_OK);
    return true;
}

bool ghostos_persona_has(const ghostos_execution_persona *persona, uint64_t right) {
    return persona && right && find(persona->active, right) < GHOSTOS_MAX_PERSONA_RIGHTS;
}

size_t ghostos_persona_active_count(const ghostos_execution_persona *persona) {
    if (!persona) return 0;
    size_t count = 0;
    for (size_t index = 0; index < GHOSTOS_MAX_PERSONA_RIGHTS; ++index) {
        if (persona->active[index]) ++count;
    }
    return count;
}

uint64_t ghostos_persona_active_at(const ghostos_execution_persona *persona, size_t index) {
    if (!persona) return 0;
    for (size_t slot = 0; slot < GHOSTOS_MAX_PERSONA_RIGHTS; ++slot) {
        if (persona->active[slot] && index-- == 0) return persona->active[slot];
    }
    return 0;
}
