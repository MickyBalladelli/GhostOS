#ifndef GHOSTOS_TEST_PROPERTY_H
#define GHOSTOS_TEST_PROPERTY_H

/* Hosted test support only. Do not link this library into the kernel or services. */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define GHOSTOS_TEST_DEFAULT_SEED UINT64_C(0x53594e4f535f5445)
#define GHOSTOS_PROPERTY_DEFAULT_CASES 256u

typedef struct { uint64_t state; } ghostos_test_entropy;
uint64_t ghostos_test_stable_hash(const uint8_t *bytes, size_t length);
ghostos_test_entropy ghostos_test_entropy_new(uint64_t seed);
uint64_t ghostos_test_entropy_next_u64(ghostos_test_entropy *entropy);
void ghostos_test_entropy_fill_bytes(ghostos_test_entropy *entropy, uint8_t *out, size_t length);

typedef struct {
    uint64_t seed;
    size_t cases;
    bool has_replay_case;
    size_t replay_case;
} ghostos_property_config;

ghostos_property_config ghostos_property_config_new(uint64_t seed, size_t cases);
ghostos_property_config ghostos_property_config_replay(uint64_t seed, size_t case_index);
ghostos_property_config ghostos_property_config_default(void);
ghostos_property_config ghostos_property_config_from_env(void);
ghostos_property_config ghostos_property_config_with_env(ghostos_property_config defaults);
uint64_t ghostos_property_case_seed(ghostos_property_config config, size_t case_index);

typedef struct {
    char *property;
    uint64_t seed;
    size_t case_index;
    uint64_t case_seed;
    char *message;
} ghostos_property_failure;

typedef enum {
    GHOSTOS_PROPERTY_OK,
    GHOSTOS_PROPERTY_FAILED,
    GHOSTOS_PROPERTY_NO_MEMORY,
    GHOSTOS_PROPERTY_INVALID_ARGUMENT
} ghostos_property_result;

/* Return NULL for success, or a message valid until the runner copies it.
   State and generated entropy are borrowed only for the duration of the callback. */
typedef const char *(*ghostos_property_check)(size_t case_index, uint64_t case_seed,
                                            ghostos_test_entropy *entropy, void *context);
typedef bool (*ghostos_property_predicate)(size_t case_index, uint64_t case_seed,
                                         ghostos_test_entropy *entropy, void *context);

/* failure is an output, initialized even on success. Dispose before reusing it.
   Failure strings are owned copies, so callbacks may reuse their message buffers. */
ghostos_property_result ghostos_property_run(const char *property, ghostos_property_config config,
    ghostos_property_check check, void *context, ghostos_property_failure *failure);
ghostos_property_result ghostos_property_run_assert(const char *property, ghostos_property_config config,
    ghostos_property_predicate check, void *context, ghostos_property_failure *failure);
void ghostos_property_failure_dispose(ghostos_property_failure *failure);
/* Returns required characters excluding NUL, or -1 on invalid input/format error. */
int ghostos_property_failure_format(const ghostos_property_failure *failure, char *out, size_t capacity);

size_t ghostos_property_bounded_usize(ghostos_test_entropy *entropy, size_t exclusive_upper_bound);
/* Successful outputs are heap-owned; free them with the C library free().
   ASCII output is NUL terminated; length excludes the terminator.
   Empty byte output is NULL with length zero. */
ghostos_property_result ghostos_property_bytes(ghostos_test_entropy *entropy, size_t maximum_length,
                                              uint8_t **out, size_t *length);
ghostos_property_result ghostos_property_ascii(ghostos_test_entropy *entropy, size_t maximum_length,
                                              char **out, size_t *length);

/* Generic FIFO copies fixed-size values, without taking ownership of pointers
   inside them. Failed push leaves the input with the caller (Rust's Err(value)). */
typedef struct {
    size_t capacity, item_size, length, head;
    uint8_t *values;
} ghostos_property_bounded_queue;
ghostos_property_result ghostos_property_queue_new(size_t capacity, size_t item_size,
                                                  ghostos_property_bounded_queue *out);
ghostos_property_result ghostos_property_queue_clone(const ghostos_property_bounded_queue *queue,
                                                    ghostos_property_bounded_queue *out);
void ghostos_property_queue_dispose(ghostos_property_bounded_queue *queue);
bool ghostos_property_queue_push(ghostos_property_bounded_queue *queue, const void *value);
bool ghostos_property_queue_pop(ghostos_property_bounded_queue *queue, void *out);
size_t ghostos_property_queue_len(const ghostos_property_bounded_queue *queue);
bool ghostos_property_queue_is_empty(const ghostos_property_bounded_queue *queue);
bool ghostos_property_queue_is_full(const ghostos_property_bounded_queue *queue);

typedef struct {
    uint64_t slot;
    uint32_t rights, generation;
    bool active;
} ghostos_property_capability_entry;
typedef struct {
    ghostos_property_capability_entry *entries;
    size_t length, capacity;
} ghostos_property_capability_model;
ghostos_property_capability_model ghostos_property_capability_model_new(void);
ghostos_property_result ghostos_property_capability_clone(const ghostos_property_capability_model *model,
                                                         ghostos_property_capability_model *out);
void ghostos_property_capability_dispose(ghostos_property_capability_model *model);
ghostos_property_result ghostos_property_capability_insert(ghostos_property_capability_model *model,
    uint64_t slot, uint32_t rights, uint32_t *generation);
bool ghostos_property_capability_attenuate(ghostos_property_capability_model *model, uint64_t slot, uint32_t rights);
bool ghostos_property_capability_revoke(ghostos_property_capability_model *model, uint64_t slot);
bool ghostos_property_capability_allows(const ghostos_property_capability_model *model, uint64_t slot, uint32_t rights);
uint32_t ghostos_property_capability_generation(const ghostos_property_capability_model *model, uint64_t slot);

typedef struct {
    uint64_t expiry;
    bool has_expiry, revoked;
} ghostos_property_lease_model;
ghostos_property_lease_model ghostos_property_lease_new(uint64_t expiry);
void ghostos_property_lease_renew(ghostos_property_lease_model *lease, uint64_t expiry);
void ghostos_property_lease_revoke(ghostos_property_lease_model *lease);
bool ghostos_property_lease_active_at(ghostos_property_lease_model lease, uint64_t now);

#endif
