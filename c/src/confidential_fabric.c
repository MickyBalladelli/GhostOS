#include "ghostos/confidential_fabric.h"
#include "ghostos/vm_snapshot_auth.h"
static void store_be16(uint8_t *output, uint16_t value) {
    output[0] = (uint8_t)(value >> 8);
    output[1] = (uint8_t)value;
}
static void store_be32(uint8_t *output, uint32_t value) {
    size_t i;
    for (i = 0; i < 4; ++i) output[i] = (uint8_t)(value >> (24 - 8 * i));
}
static void store_be64(uint8_t *output, uint64_t value) {
    size_t i;
    for (i = 0; i < 8; ++i) output[i] = (uint8_t)(value >> (56 - 8 * i));
}
static uint16_t load_be16(const uint8_t *input) { return (uint16_t)((input[0] << 8) | input[1]); }
static uint32_t load_be32(const uint8_t *input) {
    return ((uint32_t)input[0] << 24) | ((uint32_t)input[1] << 16) | ((uint32_t)input[2] << 8) | input[3];
}
static bool zero32(const uint8_t bytes[32]) {
    size_t i;
    for (i = 0; i < 32; ++i) if (bytes[i]) return false;
    return true;
}
static bool equal32(const uint8_t *left, const uint8_t *right) {
    uint8_t difference = 0;
    size_t i;
    for (i = 0; i < 32; ++i) difference |= (uint8_t)(left[i] ^ right[i]);
    return !difference;
}
static void shared_secret(const uint8_t public_key[32], const uint8_t ciphertext[32], uint8_t out[32]) {
    uint8_t material[72] = {'s', 'y', 'n', 'o', 's', '-', 's', 's'};
    size_t i;
    for (i = 0; i < 32; ++i) {
        material[8 + i] = public_key[i];
        material[40 + i] = ciphertext[i];
    }
    ghostos_vm_snapshot_sha256(material, sizeof material, out);
}
static void xor_stream(uint8_t *output, const uint8_t *input, size_t length, const uint8_t shared[32],
    const uint8_t nonce[16], const uint8_t label[9]) {
    size_t offset = 0;
    uint64_t block = 0;
    while (offset < length) {
        uint8_t material[65] = {0};
        uint8_t stream[32];
        size_t i;
        for (i = 0; i < 9; ++i) material[i] = label[i];
        for (i = 0; i < 32; ++i) material[9 + i] = shared[i];
        for (i = 0; i < 16; ++i) material[41 + i] = nonce[i];
        store_be64(material + 57, block);
        ghostos_vm_snapshot_sha256(material, sizeof material, stream);
        for (i = 0; i < 32 && offset + i < length; ++i) output[offset + i] = (uint8_t)(input[offset + i] ^ stream[i]);
        offset += 32;
        ++block;
    }
}
static void authentication_tag(uint8_t transport, const uint8_t kem[32], const uint8_t nonce[16],
    const uint8_t *ciphertext, size_t length, const uint8_t shared[32], const uint8_t label[9], uint8_t tag[32]) {
    uint8_t material[98 + GHOSTOS_FABRIC_PACKET_BYTES] = {0};
    size_t i;
    for (i = 0; i < 9; ++i) material[i] = label[i];
    for (i = 0; i < 32; ++i) material[9 + i] = shared[i];
    for (i = 0; i < 16; ++i) material[41 + i] = nonce[i];
    material[57] = transport;
    for (i = 0; i < 32; ++i) material[58 + i] = kem[i];
    store_be64(material + 90, length);
    for (i = 0; i < length; ++i) material[98 + i] = ciphertext[i];
    ghostos_vm_snapshot_sha256(material, 98 + length, tag);
}
static int open_with_labels(const ghostos_fabric_frame *frame, const uint8_t shared[32], const uint8_t ctr[9],
    const uint8_t tag_label[9], uint8_t *packet, size_t packet_capacity, size_t *packet_length) {
    uint8_t expected[32];
    if (!frame->length || frame->length > GHOSTOS_FABRIC_PACKET_BYTES) return 1;
    authentication_tag(frame->transport, frame->kem, frame->nonce, frame->ciphertext, frame->length, shared, tag_label, expected);
    if (!equal32(expected, frame->tag)) return 2;
    if (packet_capacity < frame->length) return 1;
    xor_stream(packet, frame->ciphertext, frame->length, shared, frame->nonce, ctr);
    *packet_length = frame->length;
    return 0;
}
int ghostos_fabric_keypair(const uint8_t seed[32], uint8_t public_key[32], uint8_t secret[32]) {
    uint8_t material[41] = {'s', 'y', 'n', 'o', 's', '-', 'k', 'e', 'm'};
    size_t i;
    for (i = 0; i < 32; ++i) material[9 + i] = seed[i];
    ghostos_vm_snapshot_sha256(material, sizeof material, secret);
    ghostos_vm_snapshot_sha256(secret, 32, public_key);
    return zero32(public_key) ? 1 : 0;
}
int ghostos_fabric_packet(uint8_t kind, uint32_t source, uint32_t destination, uint32_t sequence,
    uint64_t page_address, uint32_t lease_epoch, uint16_t fragment, uint16_t fragment_count,
    const uint8_t *payload, size_t payload_length, uint8_t *output, size_t output_length, size_t *written) {
    size_t length = 32 + payload_length;
    size_t i;
    if (!kind || kind > 6 || !source || !destination || !fragment_count || fragment >= fragment_count ||
        page_address % 4096 || payload_length > 1400 || output_length < length) return 1;
    for (i = 0; i < length; ++i) output[i] = 0;
    store_be16(output, 0x88b5);
    output[2] = 1;
    output[3] = kind;
    store_be32(output + 4, source);
    store_be32(output + 8, destination);
    store_be32(output + 12, sequence);
    store_be64(output + 16, page_address);
    store_be32(output + 24, lease_epoch);
    store_be16(output + 28, fragment);
    store_be16(output + 30, fragment_count);
    for (i = 0; i < payload_length; ++i) output[32 + i] = payload[i];
    *written = length;
    return 0;
}
int ghostos_fabric_packet_payload(const uint8_t *packet, size_t length, size_t *payload_offset, size_t *payload_length) {
    if (length < 32 || load_be16(packet) != 0x88b5 || packet[2] != 1 || !packet[3] || packet[3] > 6 ||
        !load_be32(packet + 4) || !load_be32(packet + 8)) return 2;
    *payload_offset = 32;
    *payload_length = length - 32;
    return 0;
}
int ghostos_fabric_seal(const uint8_t *packet, size_t packet_length, uint8_t transport,
    const uint8_t recipient[32], const uint8_t ephemeral[32], const uint8_t nonce[16], ghostos_fabric_frame *frame) {
    static const uint8_t label[] = {'s', 'y', 'n', 'o', 's', '-', 't', 'a', 'g'};
    static const uint8_t ctr[] = {'s', 'y', 'n', 'o', 's', '-', 'c', 't', 'r'};
    uint8_t shared[32];
    size_t i;
    if (!packet_length || packet_length > GHOSTOS_FABRIC_PACKET_BYTES || (transport != 1 && transport != 2) ||
        zero32(recipient) || zero32(ephemeral)) return 1;
    for (i = 0; i < GHOSTOS_FABRIC_PACKET_BYTES; ++i) frame->ciphertext[i] = 0;
    frame->transport = transport;
    frame->length = (uint16_t)packet_length;
    for (i = 0; i < 32; ++i) frame->kem[i] = ephemeral[i];
    for (i = 0; i < 16; ++i) frame->nonce[i] = nonce[i];
    shared_secret(recipient, frame->kem, shared);
    xor_stream(frame->ciphertext, packet, packet_length, shared, nonce, ctr);
    authentication_tag(transport, frame->kem, nonce, frame->ciphertext, packet_length, shared, label, frame->tag);
    return 0;
}
int ghostos_fabric_open(const ghostos_fabric_frame *frame, const uint8_t secret[32],
    uint8_t *packet, size_t packet_capacity, size_t *packet_length) {
    static const uint8_t legacy_ctr[] = {'s', 'y', 'n', 'o', 's', '-', 'c', 't', 'r'};
    static const uint8_t legacy_tag[] = {'s', 'y', 'n', 'o', 's', '-', 't', 'a', 'g'};
    static const uint8_t ghost_ctr[] = {'g', 'h', 'o', 's', 't', '-', 'c', 't', 'r'};
    static const uint8_t ghost_tag[] = {'g', 'h', 'o', 's', 't', '-', 't', 'a', 'g'};
    uint8_t public_key[32];
    uint8_t shared[32];
    int status;
    ghostos_vm_snapshot_sha256(secret, 32, public_key);
    if (zero32(public_key)) return 1;
    shared_secret(public_key, frame->kem, shared);
    status = open_with_labels(frame, shared, legacy_ctr, legacy_tag, packet, packet_capacity, packet_length);
    if (!status) return 0;
    return open_with_labels(frame, shared, ghost_ctr, ghost_tag, packet, packet_capacity, packet_length);
}
int ghostos_fabric_frame_encode(const ghostos_fabric_frame *frame, uint8_t *output, size_t output_length) {
    size_t i;
    if (output_length < GHOSTOS_FABRIC_FRAME_BYTES) return 1;
    for (i = 0; i < GHOSTOS_FABRIC_FRAME_BYTES; ++i) output[i] = 0;
    output[0] = 'S'; output[1] = 'C'; output[2] = 'F'; output[3] = '1';
    output[4] = 1;
    output[5] = frame->transport;
    for (i = 0; i < 32; ++i) output[6 + i] = frame->kem[i];
    for (i = 0; i < 16; ++i) output[38 + i] = frame->nonce[i];
    store_be16(output + 54, frame->length);
    for (i = 0; i < GHOSTOS_FABRIC_PACKET_BYTES; ++i) output[56 + i] = frame->ciphertext[i];
    for (i = 0; i < 32; ++i) output[1488 + i] = frame->tag[i];
    return 0;
}
int ghostos_fabric_frame_decode(const uint8_t *input, size_t length, ghostos_fabric_frame *frame) {
    uint16_t payload = 0;
    size_t i;
    if (length != GHOSTOS_FABRIC_FRAME_BYTES || input[0] != 'S' || input[1] != 'C' || input[2] != 'F' ||
        input[3] != '1' || input[4] != 1 || (input[5] != 1 && input[5] != 2)) return 1;
    payload = load_be16(input + 54);
    if (!payload || payload > GHOSTOS_FABRIC_PACKET_BYTES || zero32(input + 6)) return 1;
    frame->transport = input[5];
    frame->length = payload;
    for (i = 0; i < 32; ++i) frame->kem[i] = input[6 + i];
    for (i = 0; i < 16; ++i) frame->nonce[i] = input[38 + i];
    for (i = 0; i < GHOSTOS_FABRIC_PACKET_BYTES; ++i) frame->ciphertext[i] = input[56 + i];
    for (i = 0; i < 32; ++i) frame->tag[i] = input[1488 + i];
    return 0;
}
int ghostos_fabric_nonce_accept(uint8_t *nonces, bool *occupied, size_t capacity, const uint8_t nonce[16]) {
    size_t free_slot = capacity;
    size_t i, j;
    if (!capacity) return 5;
    for (i = 0; i < capacity; ++i) {
        bool same = true;
        if (!occupied[i]) {
            if (free_slot == capacity) free_slot = i;
            continue;
        }
        for (j = 0; j < 16; ++j) if (nonces[i * 16 + j] != nonce[j]) same = false;
        if (same) return 3;
    }
    if (free_slot == capacity) return 4;
    for (j = 0; j < 16; ++j) nonces[free_slot * 16 + j] = nonce[j];
    occupied[free_slot] = true;
    return 0;
}
