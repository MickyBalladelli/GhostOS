#include "ghostos/test_property.h"
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

uint64_t ghostos_test_stable_hash(const uint8_t *bytes, size_t length) {
    uint64_t hash = UINT64_C(0xcbf29ce484222325);
    for (size_t i = 0; i < length; ++i) {
        hash ^= bytes[i];
        /* Preserve the Rust fixture multiplier exactly, including its extra zero. */
        hash *= UINT64_C(0x1000000001b3);
    }
    return hash;
}
ghostos_test_entropy ghostos_test_entropy_new(uint64_t seed) {
    return (ghostos_test_entropy){seed == 0 ? 1 : seed};
}
uint64_t ghostos_test_entropy_next_u64(ghostos_test_entropy *entropy) {
    uint64_t value = entropy->state;
    value ^= value >> 12;
    value ^= value << 25;
    value ^= value >> 27;
    entropy->state = value;
    return value * UINT64_C(0x2545f4914f6cdd1d);
}
void ghostos_test_entropy_fill_bytes(ghostos_test_entropy *entropy, uint8_t *out, size_t length) {
    size_t offset = 0;
    while (offset < length) {
        uint64_t value = ghostos_test_entropy_next_u64(entropy);
        size_t chunk = length - offset < 8 ? length - offset : 8;
        for (size_t i = 0; i < chunk; ++i) out[offset + i] = (uint8_t)(value >> (8 * i));
        offset += chunk;
    }
}
ghostos_property_config ghostos_property_config_new(uint64_t seed, size_t cases) {
    return (ghostos_property_config){seed, cases, false, 0};
}
ghostos_property_config ghostos_property_config_replay(uint64_t seed, size_t case_index) {
    return (ghostos_property_config){seed, 1, true, case_index};
}
ghostos_property_config ghostos_property_config_default(void) {
    return ghostos_property_config_new(GHOSTOS_TEST_DEFAULT_SEED, GHOSTOS_PROPERTY_DEFAULT_CASES);
}
static bool parse_unsigned(const char *text, unsigned base, uint64_t maximum, uint64_t *out) {
    if (text == NULL) return false;
    if (*text == '+') ++text;
    if (*text == '\0') return false;
    uint64_t value = 0;
    for (; *text != '\0'; ++text) {
        unsigned digit;
        if (*text >= '0' && *text <= '9') digit = (unsigned)(*text - '0');
        else if (*text >= 'a' && *text <= 'f') digit = (unsigned)(*text - 'a') + 10;
        else if (*text >= 'A' && *text <= 'F') digit = (unsigned)(*text - 'A') + 10;
        else return false;
        if (digit >= base || value > (maximum - digit) / base) return false;
        value = value * base + digit;
    }
    *out = value;
    return true;
}
ghostos_property_config ghostos_property_config_with_env(ghostos_property_config defaults) {
    ghostos_property_config config = defaults;
    const char *seed = getenv("GHOSTOS_PROPERTY_SEED");
    unsigned base = 10;
    if (seed != NULL && seed[0] == '0' && (seed[1] == 'x' || seed[1] == 'X')) { seed += 2; base = 16; }
    uint64_t parsed;
    if (parse_unsigned(seed, base, UINT64_MAX, &parsed)) config.seed = parsed;
    if (parse_unsigned(getenv("GHOSTOS_PROPERTY_CASES"), 10, SIZE_MAX, &parsed) && parsed > 0) config.cases = (size_t)parsed;
    if (parse_unsigned(getenv("GHOSTOS_PROPERTY_CASE"), 10, SIZE_MAX, &parsed)) {
        config.has_replay_case = true;
        config.replay_case = (size_t)parsed;
    }
    return config;
}
ghostos_property_config ghostos_property_config_from_env(void) {
    return ghostos_property_config_with_env(ghostos_property_config_default());
}
uint64_t ghostos_property_case_seed(ghostos_property_config config, size_t case_index) {
    uint8_t input[16];
    for (size_t i = 0; i < 8; ++i) {
        input[i] = (uint8_t)(config.seed >> (i * 8));
        input[8 + i] = (uint8_t)((uint64_t)case_index >> (i * 8));
    }
    return ghostos_test_stable_hash(input, sizeof(input));
}
static char *copy_string(const char *value) {
    size_t length = strlen(value);
    if (length == SIZE_MAX) return NULL;
    char *copy = malloc(length + 1);
    if (copy != NULL) memcpy(copy, value, length + 1);
    return copy;
}
void ghostos_property_failure_dispose(ghostos_property_failure *failure) {
    if (failure == NULL) return;
    free(failure->property);
    free(failure->message);
    *failure = (ghostos_property_failure){0};
}
ghostos_property_result ghostos_property_run(const char *property, ghostos_property_config config,
    ghostos_property_check check, void *context, ghostos_property_failure *failure) {
    if (failure == NULL) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    *failure = (ghostos_property_failure){0};
    if (property == NULL || check == NULL) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    size_t count = config.has_replay_case ? 1 : config.cases;
    for (size_t i = 0; i < count; ++i) {
        size_t case_index = config.has_replay_case ? config.replay_case : i;
        uint64_t seed = ghostos_property_case_seed(config, case_index);
        ghostos_test_entropy entropy = ghostos_test_entropy_new(seed);
        const char *message = check(case_index, seed, &entropy, context);
        if (message == NULL) continue;
        failure->property = copy_string(property);
        failure->message = copy_string(message);
        if (failure->property == NULL || failure->message == NULL) {
            ghostos_property_failure_dispose(failure);
            return GHOSTOS_PROPERTY_NO_MEMORY;
        }
        failure->seed = config.seed;
        failure->case_index = case_index;
        failure->case_seed = seed;
        return GHOSTOS_PROPERTY_FAILED;
    }
    return GHOSTOS_PROPERTY_OK;
}
typedef struct { ghostos_property_predicate check; void *context; } predicate_context;
static const char *check_predicate(size_t case_index, uint64_t seed, ghostos_test_entropy *entropy, void *context) {
    predicate_context *predicate = context;
    return predicate->check(case_index, seed, entropy, predicate->context) ? NULL : "property assertion failed";
}
ghostos_property_result ghostos_property_run_assert(const char *property, ghostos_property_config config,
    ghostos_property_predicate check, void *context, ghostos_property_failure *failure) {
    if (check == NULL) {
        if (failure != NULL) *failure = (ghostos_property_failure){0};
        return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    }
    predicate_context predicate = {check, context};
    return ghostos_property_run(property, config, check_predicate, &predicate, failure);
}
int ghostos_property_failure_format(const ghostos_property_failure *failure, char *out, size_t capacity) {
    if (failure == NULL || failure->property == NULL || failure->message == NULL || (capacity != 0 && out == NULL)) return -1;
    return snprintf(out, capacity,
        "property %s failed at case %zu (seed 0x%016" PRIx64 ", case seed 0x%016" PRIx64 "): %s\n"
        "replay with GHOSTOS_PROPERTY_SEED=0x%016" PRIx64 " GHOSTOS_PROPERTY_CASE=%zu",
        failure->property, failure->case_index, failure->seed, failure->case_seed,
        failure->message, failure->seed, failure->case_index);
}
size_t ghostos_property_bounded_usize(ghostos_test_entropy *entropy, size_t exclusive_upper_bound) {
    return exclusive_upper_bound == 0 ? 0 : (size_t)(ghostos_test_entropy_next_u64(entropy) % (uint64_t)exclusive_upper_bound);
}
ghostos_property_result ghostos_property_bytes(ghostos_test_entropy *entropy, size_t maximum_length,
                                              uint8_t **out, size_t *length) {
    if (entropy == NULL || out == NULL || length == NULL) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    *out = NULL;
    *length = 0;
    size_t bound = maximum_length == SIZE_MAX ? SIZE_MAX : maximum_length + 1;
    size_t count = ghostos_property_bounded_usize(entropy, bound);
    if (count == 0) return GHOSTOS_PROPERTY_OK;
    uint8_t *values = malloc(count);
    if (values == NULL) return GHOSTOS_PROPERTY_NO_MEMORY;
    ghostos_test_entropy_fill_bytes(entropy, values, count);
    *out = values;
    *length = count;
    return GHOSTOS_PROPERTY_OK;
}
ghostos_property_result ghostos_property_ascii(ghostos_test_entropy *entropy, size_t maximum_length,
                                              char **out, size_t *length) {
    if (out == NULL || length == NULL) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    *out = NULL;
    *length = 0;
    uint8_t *values;
    size_t count;
    ghostos_property_result result = ghostos_property_bytes(entropy, maximum_length, &values, &count);
    if (result != GHOSTOS_PROPERTY_OK) return result;
    char *text = malloc(count + 1);
    if (text == NULL) { free(values); return GHOSTOS_PROPERTY_NO_MEMORY; }
    for (size_t i = 0; i < count; ++i) text[i] = (char)('a' + values[i] % 26);
    text[count] = '\0';
    free(values);
    *out = text;
    *length = count;
    return GHOSTOS_PROPERTY_OK;
}

ghostos_property_result ghostos_property_queue_new(size_t capacity, size_t item_size,
                                                  ghostos_property_bounded_queue *out) {
    if (out == NULL) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    *out = (ghostos_property_bounded_queue){0};
    if (item_size == 0 || capacity > SIZE_MAX / item_size) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    uint8_t *values = capacity == 0 ? NULL : malloc(capacity * item_size);
    if (capacity != 0 && values == NULL) return GHOSTOS_PROPERTY_NO_MEMORY;
    *out = (ghostos_property_bounded_queue){capacity, item_size, 0, 0, values};
    return GHOSTOS_PROPERTY_OK;
}
static size_t queue_index(const ghostos_property_bounded_queue *queue, size_t offset) {
    size_t remaining = queue->capacity - queue->head;
    return offset < remaining ? queue->head + offset : offset - remaining;
}
ghostos_property_result ghostos_property_queue_clone(const ghostos_property_bounded_queue *queue,
                                                    ghostos_property_bounded_queue *out) {
    if (queue == NULL || out == NULL || queue == out) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    ghostos_property_result result = ghostos_property_queue_new(queue->capacity, queue->item_size, out);
    if (result != GHOSTOS_PROPERTY_OK) return result;
    for (size_t i = 0; i < queue->length; ++i)
        memcpy(out->values + i * queue->item_size, queue->values + queue_index(queue, i) * queue->item_size, queue->item_size);
    out->length = queue->length;
    return GHOSTOS_PROPERTY_OK;
}
void ghostos_property_queue_dispose(ghostos_property_bounded_queue *queue) {
    if (queue == NULL) return;
    free(queue->values);
    *queue = (ghostos_property_bounded_queue){0};
}
bool ghostos_property_queue_push(ghostos_property_bounded_queue *queue, const void *value) {
    if (value == NULL || queue->length == queue->capacity) return false;
    memcpy(queue->values + queue_index(queue, queue->length) * queue->item_size, value, queue->item_size);
    ++queue->length;
    return true;
}
bool ghostos_property_queue_pop(ghostos_property_bounded_queue *queue, void *out) {
    if (out == NULL || queue->length == 0) return false;
    memcpy(out, queue->values + queue->head * queue->item_size, queue->item_size);
    --queue->length;
    queue->head = queue->head == queue->capacity - 1 ? 0 : queue->head + 1;
    return true;
}
size_t ghostos_property_queue_len(const ghostos_property_bounded_queue *queue) { return queue->length; }
bool ghostos_property_queue_is_empty(const ghostos_property_bounded_queue *queue) { return queue->length == 0; }
bool ghostos_property_queue_is_full(const ghostos_property_bounded_queue *queue) { return queue->length == queue->capacity; }

ghostos_property_capability_model ghostos_property_capability_model_new(void) {
    return (ghostos_property_capability_model){0};
}
ghostos_property_result ghostos_property_capability_clone(const ghostos_property_capability_model *model,
                                                         ghostos_property_capability_model *out) {
    if (model == NULL || out == NULL || model == out) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    *out = ghostos_property_capability_model_new();
    if (model->length == 0) return GHOSTOS_PROPERTY_OK;
    out->entries = malloc(model->length * sizeof(*out->entries));
    if (out->entries == NULL) return GHOSTOS_PROPERTY_NO_MEMORY;
    memcpy(out->entries, model->entries, model->length * sizeof(*out->entries));
    out->capacity = out->length = model->length;
    return GHOSTOS_PROPERTY_OK;
}
void ghostos_property_capability_dispose(ghostos_property_capability_model *model) {
    if (model == NULL) return;
    free(model->entries);
    *model = ghostos_property_capability_model_new();
}
static size_t capability_position(const ghostos_property_capability_model *model, uint64_t slot) {
    size_t low = 0, high = model->length;
    while (low < high) {
        size_t mid = low + (high - low) / 2;
        if (model->entries[mid].slot < slot) low = mid + 1;
        else high = mid;
    }
    return low;
}
ghostos_property_result ghostos_property_capability_insert(ghostos_property_capability_model *model,
    uint64_t slot, uint32_t rights, uint32_t *generation) {
    if (model == NULL || generation == NULL) return GHOSTOS_PROPERTY_INVALID_ARGUMENT;
    size_t position = capability_position(model, slot);
    if (position == model->length || model->entries[position].slot != slot) {
        if (model->length == model->capacity) {
            size_t maximum = SIZE_MAX / sizeof(*model->entries);
            if (model->capacity >= maximum) return GHOSTOS_PROPERTY_NO_MEMORY;
            size_t capacity = model->capacity == 0 ? 1 : model->capacity > maximum / 2 ? maximum : model->capacity * 2;
            ghostos_property_capability_entry *entries = realloc(model->entries, capacity * sizeof(*entries));
            if (entries == NULL) return GHOSTOS_PROPERTY_NO_MEMORY;
            model->entries = entries;
            model->capacity = capacity;
        }
        memmove(model->entries + position + 1, model->entries + position, (model->length - position) * sizeof(*model->entries));
        model->entries[position] = (ghostos_property_capability_entry){slot, 0, 1, false};
        ++model->length;
    }
    model->entries[position].rights = rights;
    model->entries[position].active = true;
    *generation = model->entries[position].generation;
    return GHOSTOS_PROPERTY_OK;
}
bool ghostos_property_capability_attenuate(ghostos_property_capability_model *model, uint64_t slot, uint32_t rights) {
    size_t position = capability_position(model, slot);
    if (position == model->length || model->entries[position].slot != slot || !model->entries[position].active) return false;
    if ((rights & ~model->entries[position].rights) != 0) return false;
    model->entries[position].rights = rights;
    return true;
}
bool ghostos_property_capability_revoke(ghostos_property_capability_model *model, uint64_t slot) {
    size_t position = capability_position(model, slot);
    if (position == model->length || model->entries[position].slot != slot || !model->entries[position].active) return false;
    model->entries[position].active = false;
    ++model->entries[position].generation;
    if (model->entries[position].generation == 0) model->entries[position].generation = 1;
    return true;
}
bool ghostos_property_capability_allows(const ghostos_property_capability_model *model, uint64_t slot, uint32_t rights) {
    size_t position = capability_position(model, slot);
    return position < model->length && model->entries[position].slot == slot && model->entries[position].active &&
        (rights & ~model->entries[position].rights) == 0;
}
uint32_t ghostos_property_capability_generation(const ghostos_property_capability_model *model, uint64_t slot) {
    size_t position = capability_position(model, slot);
    return position < model->length && model->entries[position].slot == slot ? model->entries[position].generation : 0;
}
ghostos_property_lease_model ghostos_property_lease_new(uint64_t expiry) {
    return (ghostos_property_lease_model){expiry, true, false};
}
void ghostos_property_lease_renew(ghostos_property_lease_model *lease, uint64_t expiry) {
    if (!lease->revoked) { lease->expiry = expiry; lease->has_expiry = true; }
}
void ghostos_property_lease_revoke(ghostos_property_lease_model *lease) {
    lease->revoked = true;
    lease->has_expiry = false;
    lease->expiry = 0;
}
bool ghostos_property_lease_active_at(ghostos_property_lease_model lease, uint64_t now) {
    return !lease.revoked && lease.has_expiry && now < lease.expiry;
}
