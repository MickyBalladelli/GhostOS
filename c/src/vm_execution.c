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

/* Separate chaining keeps payload addresses stable when the table grows.
 * FIFO keys are independent nodes: reset historically leaves stale keys and
 * repeated insertions may leave duplicates. Neither behavior is normalized. */
#include <stdlib.h>

typedef struct cache_entry {
    ghostos_vm_execution_key key;
    void *payload;
    struct cache_entry *next;
} cache_entry;
typedef struct cache_order {
    ghostos_vm_execution_key key;
    struct cache_order *next;
} cache_order;
struct ghostos_vm_execution_cache {
    cache_entry **buckets;
    size_t bucket_count, length;
    cache_order *first, *last;
    void (*destroy)(void *);
};

static bool same_key(const ghostos_vm_execution_key *a, const ghostos_vm_execution_key *b) {
    return a->rip == b->rip && a->cr3 == b->cr3 && a->mode == b->mode && a->privilege == b->privilege;
}
static uint64_t mix_key(uint64_t value) {
    value ^= value >> 30;
    value *= UINT64_C(0xbf58476d1ce4e5b9);
    value ^= value >> 27;
    value *= UINT64_C(0x94d049bb133111eb);
    return value ^ (value >> 31);
}
static size_t key_bucket(const ghostos_vm_execution_key *key, size_t count) {
    uint64_t hash = mix_key(key->rip) ^ mix_key(key->cr3) ^
        mix_key(((uint64_t)key->mode << 32) | key->privilege);
    return (size_t)hash & (count - 1);
}
static cache_entry **entry_slot(ghostos_vm_execution_cache *cache, const ghostos_vm_execution_key *key) {
    cache_entry **slot = &cache->buckets[key_bucket(key, cache->bucket_count)];
    while (*slot != NULL && !same_key(&(*slot)->key, key)) slot = &(*slot)->next;
    return slot;
}
static bool grow_cache(ghostos_vm_execution_cache *cache) {
    if (cache->bucket_count > SIZE_MAX / 2 / sizeof(cache_entry *)) return false;
    size_t count = cache->bucket_count * 2;
    cache_entry **buckets = calloc(count, sizeof(*buckets));
    if (buckets == NULL) return false;
    for (size_t i = 0; i < cache->bucket_count; ++i) {
        cache_entry *entry = cache->buckets[i];
        while (entry != NULL) {
            cache_entry *next = entry->next;
            size_t bucket = key_bucket(&entry->key, count);
            entry->next = buckets[bucket];
            buckets[bucket] = entry;
            entry = next;
        }
    }
    free(cache->buckets);
    cache->buckets = buckets;
    cache->bucket_count = count;
    return true;
}

ghostos_vm_execution_cache *ghostos_vm_execution_cache_new(void (*destroy)(void *)) {
    ghostos_vm_execution_cache *cache = calloc(1, sizeof(*cache));
    if (cache == NULL) return NULL;
    cache->bucket_count = 64;
    cache->buckets = calloc(cache->bucket_count, sizeof(*cache->buckets));
    if (cache->buckets == NULL) { free(cache); return NULL; }
    cache->destroy = destroy;
    return cache;
}

void ghostos_vm_execution_cache_clear(ghostos_vm_execution_cache *cache, bool clear_order) {
    for (size_t i = 0; i < cache->bucket_count; ++i) {
        cache_entry *entry = cache->buckets[i];
        cache->buckets[i] = NULL;
        while (entry != NULL) {
            cache_entry *next = entry->next;
            cache->destroy(entry->payload);
            free(entry);
            entry = next;
        }
    }
    cache->length = 0;
    if (clear_order) {
        while (cache->first != NULL) {
            cache_order *next = cache->first->next;
            free(cache->first);
            cache->first = next;
        }
        cache->last = NULL;
    }
}

void ghostos_vm_execution_cache_free(ghostos_vm_execution_cache *cache) {
    if (cache == NULL) return;
    ghostos_vm_execution_cache_clear(cache, true);
    free(cache->buckets);
    free(cache);
}

size_t ghostos_vm_execution_cache_length(const ghostos_vm_execution_cache *cache) { return cache->length; }

void *ghostos_vm_execution_cache_get(const ghostos_vm_execution_cache *cache, const ghostos_vm_execution_key *key) {
    const cache_entry *entry = cache->buckets[key_bucket(key, cache->bucket_count)];
    while (entry != NULL && !same_key(&entry->key, key)) entry = entry->next;
    return entry == NULL ? NULL : entry->payload;
}

void *ghostos_vm_execution_cache_remove(ghostos_vm_execution_cache *cache, const ghostos_vm_execution_key *key) {
    cache_entry **slot = entry_slot(cache, key);
    if (*slot == NULL) return NULL;
    cache_entry *entry = *slot;
    *slot = entry->next;
    void *payload = entry->payload;
    free(entry);
    --cache->length;
    return payload;
}

void ghostos_vm_execution_cache_forget_order(ghostos_vm_execution_cache *cache, const ghostos_vm_execution_key *key) {
    cache_order **slot = &cache->first;
    cache->last = NULL;
    while (*slot != NULL) {
        cache_order *order = *slot;
        if (same_key(&order->key, key)) {
            *slot = order->next;
            free(order);
        } else {
            cache->last = order;
            slot = &order->next;
        }
    }
}

void *ghostos_vm_execution_cache_evict(ghostos_vm_execution_cache *cache, const ghostos_vm_execution_key *key, size_t capacity) {
    if (capacity == 0) capacity = 1;
    if (cache->length < capacity || ghostos_vm_execution_cache_get(cache, key) != NULL) return NULL;
    while (cache->first != NULL) {
        cache_order *order = cache->first;
        cache->first = order->next;
        if (cache->first == NULL) cache->last = NULL;
        void *payload = ghostos_vm_execution_cache_remove(cache, &order->key);
        free(order);
        if (payload != NULL) return payload;
    }
    return NULL;
}

bool ghostos_vm_execution_cache_insert(ghostos_vm_execution_cache *cache, const ghostos_vm_execution_key *key, void *payload) {
    cache_order *order = malloc(sizeof(*order));
    if (order == NULL) return false;
    cache_entry **slot = entry_slot(cache, key);
    cache_entry *entry = *slot;
    if (entry == NULL) {
        entry = malloc(sizeof(*entry));
        if (entry == NULL) { free(order); return false; }
        if (cache->length >= cache->bucket_count && !grow_cache(cache)) {
            free(entry); free(order); return false;
        }
        slot = entry_slot(cache, key);
        entry->key = *key;
        entry->next = NULL;
        *slot = entry;
        ++cache->length;
    } else {
        cache->destroy(entry->payload);
    }
    entry->payload = payload;
    order->key = *key;
    order->next = NULL;
    if (cache->last == NULL) cache->first = order;
    else cache->last->next = order;
    cache->last = order;
    return true;
}
