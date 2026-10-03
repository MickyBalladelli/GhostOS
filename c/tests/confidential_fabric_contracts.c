#include "ghostos/confidential_fabric.h"
#include <assert.h>
#include <string.h>
static void sealed_page_opens_once_and_rejects_a_changed_tag(void) {
    uint8_t seed[32], ephemeral[32], nonce[16], public_key[32], secret[32];
    uint8_t packet[64], opened[64], encoded[GHOSTOS_FABRIC_FRAME_BYTES];
    uint8_t history[32] = {0};
    bool occupied[2] = {false, false};
    ghostos_fabric_frame frame;
    const uint8_t payload[] = "secret page";
    size_t written = 0;
    size_t offset = 0;
    size_t payload_length = 0;
    size_t i;
    for (i = 0; i < 32; ++i) { seed[i] = 5; ephemeral[i] = 8; }
    for (i = 0; i < 16; ++i) nonce[i] = 4;
    assert(!ghostos_fabric_keypair(seed, public_key, secret));
    assert(!ghostos_fabric_packet(2, 1, 2, 4, 4096, 1, 0, 1, payload, 11, packet, sizeof packet, &written));
    assert(!ghostos_fabric_seal(packet, written, 2, public_key, ephemeral, nonce, &frame));
    assert(!ghostos_fabric_open(&frame, secret, opened, sizeof opened, &written));
    assert(!ghostos_fabric_packet_payload(opened, written, &offset, &payload_length));
    assert(payload_length == 11 && !memcmp(opened + offset, payload, 11));
    assert(!ghostos_fabric_frame_encode(&frame, encoded, sizeof encoded));
    assert(!ghostos_fabric_frame_decode(encoded, sizeof encoded, &frame));
    encoded[sizeof encoded - 1] ^= 1;
    assert(!ghostos_fabric_frame_decode(encoded, sizeof encoded, &frame));
    assert(ghostos_fabric_open(&frame, secret, opened, sizeof opened, &written) == 2);
    assert(!ghostos_fabric_nonce_accept(history, occupied, 2, nonce));
    assert(ghostos_fabric_nonce_accept(history, occupied, 2, nonce) == 3);
}
int main(void) {
    sealed_page_opens_once_and_rejects_a_changed_tag();
    return 0;
}
