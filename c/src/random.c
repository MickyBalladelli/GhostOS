#include "ghostos/random.h"

#define RANDOM_KEY_WORDS 8u
#define RANDOM_BLOCK_BYTES 64u

static uint64_t key[RANDOM_KEY_WORDS / 2u];
static uint64_t block_counter;
static uint32_t ready;

static uint64_t load_relaxed(const uint64_t *value) {
    return __atomic_load_n(value, __ATOMIC_RELAXED);
}

#if defined(__x86_64__)
static uint64_t avalanche(uint64_t value) {
    value ^= value >> 30;
    value *= UINT64_C(0xbf58476d1ce4e5b9);
    value ^= value >> 27;
    value *= UINT64_C(0x94d049bb133111eb);
    return value ^ (value >> 31);
}

static uint64_t rotate_left64(uint64_t value, unsigned amount) {
    if (!amount) return value;
    return (value << amount) | (value >> (64u - amount));
}

static void cpuid(uint32_t leaf, uint32_t subleaf, uint32_t *a, uint32_t *b,
    uint32_t *c, uint32_t *d) {
    __asm__ volatile("cpuid"
        : "=a"(*a), "=b"(*b), "=c"(*c), "=d"(*d)
        : "a"(leaf), "c"(subleaf));
}

static bool random_instruction(bool seed, uint64_t *value) {
    uint8_t ok;
    if (seed) {
        __asm__ volatile("rdseed %0; setc %1" : "=r"(*value), "=qm"(ok) : : "cc");
    } else {
        __asm__ volatile("rdrand %0; setc %1" : "=r"(*value), "=qm"(ok) : : "cc");
    }
    return ok != 0;
}

static bool retry_random(bool seed, uint64_t *value) {
    for (unsigned attempt = 0; attempt < 64; ++attempt) {
        if (random_instruction(seed, value)) return true;
        __asm__ volatile("pause");
    }
    return false;
}

static uint64_t timestamp(void) {
    uint32_t low, high;
    __asm__ volatile("rdtsc" : "=a"(low), "=d"(high));
    return (uint64_t)low | ((uint64_t)high << 32);
}

static bool entropy_seed(uint64_t output[4]) {
    uint32_t a, b, c, d;
    cpuid(1, 0, &a, &b, &c, &d);
    bool has_rdrand = (c & (UINT32_C(1) << 30)) != 0;
    cpuid(7, 0, &a, &b, &c, &d);
    bool has_rdseed = (b & (UINT32_C(1) << 18)) != 0;
    if (!has_rdrand && !has_rdseed) return false;
    for (unsigned index = 0; index < 4; ++index) {
        uint64_t source;
        bool obtained = has_rdseed && retry_random(true, &source);
        if (!obtained && (!has_rdrand || !retry_random(false, &source))) return false;
        output[index] = avalanche(source ^ rotate_left64(timestamp(), index * 11u));
    }
    return true;
}
#else
static bool entropy_seed(uint64_t output[4]) {
    (void)output;
    return false;
}
#endif

void ghostos_random_initialize(void) {
    uint64_t seed[4] = {0};
    if (!entropy_seed(seed)) return;
    for (unsigned index = 0; index < 4; ++index) {
        __atomic_store_n(&key[index], seed[index], __ATOMIC_RELAXED);
        seed[index] = 0;
    }
    __atomic_store_n(&block_counter, 0, __ATOMIC_RELAXED);
    __atomic_store_n(&ready, 1, __ATOMIC_RELEASE);
}

bool ghostos_random_ready(void) {
    return __atomic_load_n(&ready, __ATOMIC_ACQUIRE) != 0;
}

static uint32_t key_word(unsigned index) {
    uint64_t value = load_relaxed(&key[index / 2u]);
    return (uint32_t)(value >> ((index % 2u) * 32u));
}

static uint32_t rotate_left(uint32_t value, unsigned amount) {
    return (value << amount) | (value >> (32u - amount));
}

static void quarter_round(uint32_t state[16], unsigned a, unsigned b, unsigned c, unsigned d) {
    state[a] += state[b]; state[d] = rotate_left(state[d] ^ state[a], 16);
    state[c] += state[d]; state[b] = rotate_left(state[b] ^ state[c], 12);
    state[a] += state[b]; state[d] = rotate_left(state[d] ^ state[a], 8);
    state[c] += state[d]; state[b] = rotate_left(state[b] ^ state[c], 7);
}

static void store_le32(uint8_t *output, uint32_t value) {
    output[0] = (uint8_t)value;
    output[1] = (uint8_t)(value >> 8);
    output[2] = (uint8_t)(value >> 16);
    output[3] = (uint8_t)(value >> 24);
}

static void chacha20_block(uint64_t counter, uint8_t output[RANDOM_BLOCK_BYTES]) {
    uint32_t state[16] = {
        UINT32_C(0x61707865), UINT32_C(0x3320646e), UINT32_C(0x79622d32), UINT32_C(0x6b206574),
        0, 0, 0, 0, 0, 0, 0, 0, (uint32_t)counter, (uint32_t)(counter >> 32), 0, 0,
    };
    for (unsigned index = 0; index < RANDOM_KEY_WORDS; ++index) state[4 + index] = key_word(index);
    state[14] = state[4] ^ state[8];
    state[15] = state[7] ^ state[11];
    uint32_t original[16];
    for (unsigned index = 0; index < 16; ++index) original[index] = state[index];
    for (unsigned round = 0; round < 10; ++round) {
        quarter_round(state, 0, 4, 8, 12); quarter_round(state, 1, 5, 9, 13);
        quarter_round(state, 2, 6, 10, 14); quarter_round(state, 3, 7, 11, 15);
        quarter_round(state, 0, 5, 10, 15); quarter_round(state, 1, 6, 11, 12);
        quarter_round(state, 2, 7, 8, 13); quarter_round(state, 3, 4, 9, 14);
    }
    for (unsigned index = 0; index < 16; ++index) store_le32(output + index * 4u, state[index] + original[index]);
}

bool ghostos_random_fill(uint8_t *output, size_t length) {
    if (!output || !length || length > GHOSTOS_RANDOM_MAX_REQUEST_BYTES || !ghostos_random_ready()) return false;
    uint64_t blocks = (length + RANDOM_BLOCK_BYTES - 1u) / RANDOM_BLOCK_BYTES;
    uint64_t first = __atomic_load_n(&block_counter, __ATOMIC_ACQUIRE);
    for (;;) {
        if (UINT64_MAX - first < blocks) return false;
        if (__atomic_compare_exchange_n(&block_counter, &first, first + blocks, true,
                __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE)) break;
    }
    size_t offset = 0;
    for (uint64_t index = 0; index < blocks; ++index) {
        uint8_t block[RANDOM_BLOCK_BYTES];
        chacha20_block(first + index, block);
        size_t remaining = length - offset;
        size_t copy = remaining < RANDOM_BLOCK_BYTES ? remaining : RANDOM_BLOCK_BYTES;
        for (size_t byte = 0; byte < copy; ++byte) output[offset + byte] = block[byte];
        offset += copy;
    }
    return true;
}
