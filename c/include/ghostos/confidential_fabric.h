#ifndef GHOSTOS_CONFIDENTIAL_FABRIC_H
#define GHOSTOS_CONFIDENTIAL_FABRIC_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 invalid, 2 authentication failed, 3 replay, 4 capacity,
 * 5 invalid configuration. Transport: CXL=1, Ethernet=2. */
#define GHOSTOS_FABRIC_PACKET_BYTES 1432u
#define GHOSTOS_FABRIC_FRAME_BYTES 1520u
typedef struct {
    uint8_t transport, kem[32], nonce[16], ciphertext[GHOSTOS_FABRIC_PACKET_BYTES], tag[32];
    uint16_t length;
} ghostos_fabric_frame;
int ghostos_fabric_keypair(const uint8_t seed[32], uint8_t public_key[32], uint8_t secret[32]);
int ghostos_fabric_packet(uint8_t kind, uint32_t source, uint32_t destination, uint32_t sequence,
    uint64_t page_address, uint32_t lease_epoch, uint16_t fragment, uint16_t fragment_count,
    const uint8_t *payload, size_t payload_length, uint8_t *output, size_t output_length, size_t *written);
int ghostos_fabric_packet_payload(const uint8_t *packet, size_t length, size_t *payload_offset, size_t *payload_length);
int ghostos_fabric_seal(const uint8_t *packet, size_t packet_length, uint8_t transport,
    const uint8_t recipient[32], const uint8_t ephemeral[32], const uint8_t nonce[16], ghostos_fabric_frame *frame);
int ghostos_fabric_open(const ghostos_fabric_frame *frame, const uint8_t secret[32],
    uint8_t *packet, size_t packet_capacity, size_t *packet_length);
int ghostos_fabric_frame_encode(const ghostos_fabric_frame *frame, uint8_t *output, size_t output_length);
int ghostos_fabric_frame_decode(const uint8_t *input, size_t length, ghostos_fabric_frame *frame);
int ghostos_fabric_nonce_accept(uint8_t *nonces, bool *occupied, size_t capacity, const uint8_t nonce[16]);
#endif
