#include "ghostos/vm_snapshot_auth.h"
#include <string.h>

static const uint32_t round_constants[64] = {
    0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,
    0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,
    0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,
    0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,
    0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,
    0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,
    0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,
    0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2
};

static uint32_t rotate_right(uint32_t value, unsigned amount) {
    return (value >> amount) | (value << (32u - amount));
}

static uint32_t load_be32(const uint8_t *bytes) {
    return ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
        ((uint32_t)bytes[2] << 8) | bytes[3];
}

static void compress(uint32_t state[8], const uint8_t block[64]) {
    uint32_t words[64];
    for (size_t index = 0; index < 16; ++index) words[index] = load_be32(block + index * 4);
    for (size_t index = 16; index < 64; ++index) {
        uint32_t x = words[index - 15], y = words[index - 2];
        uint32_t s0 = rotate_right(x, 7) ^ rotate_right(x, 18) ^ (x >> 3);
        uint32_t s1 = rotate_right(y, 17) ^ rotate_right(y, 19) ^ (y >> 10);
        words[index] = words[index - 16] + s0 + words[index - 7] + s1;
    }
    uint32_t a=state[0], b=state[1], c=state[2], d=state[3];
    uint32_t e=state[4], f=state[5], g=state[6], h=state[7];
    for (size_t index = 0; index < 64; ++index) {
        uint32_t s1 = rotate_right(e, 6) ^ rotate_right(e, 11) ^ rotate_right(e, 25);
        uint32_t choice = (e & f) ^ (~e & g);
        uint32_t t1 = h + s1 + choice + round_constants[index] + words[index];
        uint32_t s0 = rotate_right(a, 2) ^ rotate_right(a, 13) ^ rotate_right(a, 22);
        uint32_t majority = (a & b) ^ (a & c) ^ (b & c);
        uint32_t t2 = s0 + majority;
        h=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2;
    }
    state[0]+=a; state[1]+=b; state[2]+=c; state[3]+=d;
    state[4]+=e; state[5]+=f; state[6]+=g; state[7]+=h;
}

typedef struct {
    uint32_t state[8];
    uint8_t buffer[64];
    size_t buffered;
    uint64_t length;
} sha256_state;

static sha256_state sha256_new(void) {
    return (sha256_state){.state = {0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,
        0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19}};
}

static void update(sha256_state *hash, const uint8_t *bytes, size_t length) {
    hash->length += (uint64_t)length;
    if (hash->buffered) {
        size_t needed = 64 - hash->buffered;
        size_t copied = length < needed ? length : needed;
        if (copied) memcpy(hash->buffer + hash->buffered, bytes, copied);
        hash->buffered += copied;
        if (length < needed) return;
        compress(hash->state, hash->buffer);
        hash->buffered = 0;
        bytes += needed;
        length -= needed;
    }
    while (length >= 64) {
        compress(hash->state, bytes);
        bytes += 64;
        length -= 64;
    }
    if (length) memcpy(hash->buffer, bytes, length);
    hash->buffered = length;
}

static void finish(sha256_state *hash, uint8_t digest[32]) {
    uint64_t bits = hash->length * 8;
    hash->buffer[hash->buffered++] = 0x80;
    if (hash->buffered > 56) {
        memset(hash->buffer + hash->buffered, 0, 64 - hash->buffered);
        compress(hash->state, hash->buffer);
        hash->buffered = 0;
    }
    memset(hash->buffer + hash->buffered, 0, 56 - hash->buffered);
    for (unsigned i = 0; i < 8; ++i) hash->buffer[63 - i] = (uint8_t)(bits >> (i * 8));
    compress(hash->state, hash->buffer);
    for (unsigned i = 0; i < 8; ++i)
        for (unsigned j = 0; j < 4; ++j)
            digest[i * 4 + j] = (uint8_t)(hash->state[i] >> (24 - j * 8));
}

void ghostos_vm_snapshot_sha256(const uint8_t *bytes, size_t length, uint8_t digest[32]) {
    sha256_state hash = sha256_new();
    update(&hash, bytes, length);
    finish(&hash, digest);
}

void ghostos_vm_snapshot_hmac_sha256(const uint8_t key[32],
    const ghostos_vm_snapshot_auth_part *parts, size_t count, uint8_t tag[32]) {
    uint8_t pad[64], inner_digest[32];
    for (unsigned i = 0; i < 64; ++i) pad[i] = (i < 32 ? key[i] : 0) ^ 0x36;
    sha256_state inner = sha256_new();
    update(&inner, pad, 64);
    for (size_t i = 0; i < count; ++i) update(&inner, parts[i].bytes, parts[i].length);
    finish(&inner, inner_digest);
    for (unsigned i = 0; i < 64; ++i) pad[i] = (i < 32 ? key[i] : 0) ^ 0x5c;
    sha256_state outer = sha256_new();
    update(&outer, pad, 64);
    update(&outer, inner_digest, 32);
    finish(&outer, tag);
}

bool ghostos_vm_snapshot_auth_equal(const uint8_t *left, const uint8_t *right, size_t length) {
    volatile uint8_t difference = 0;
    for (size_t i = 0; i < length; ++i) difference |= left[i] ^ right[i];
    return difference == 0;
}
