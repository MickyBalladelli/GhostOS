#include "ghostos/vm_e1000.h"
#include "ghostos/vm_terminal.h"
#include "ghostos/vm_input.h"
#include "ghostos/vm_segment.h"
#include <assert.h>
#include <stdio.h>
#include <string.h>

static uint64_t nic_read(ghostos_vm_e1000 *nic, uint64_t address) {
    uint64_t value;
    assert(ghostos_vm_e1000_read(nic, address, 4, true, &value) == 0);
    return value;
}
static void nic_write(ghostos_vm_e1000 *nic, uint64_t address, uint32_t value) {
    assert(ghostos_vm_e1000_write(nic, address, 4, value, NULL) == 0);
}

/* Port of registers_mac_link_interrupt_and_reset. */
static void e1000_registers(void) {
    const uint8_t mac[] = {0x52,0x54,0,0x12,0x34,1};
    ghostos_vm_e1000 *nic = ghostos_vm_e1000_new(mac);
    assert(nic);
    assert((nic_read(nic, 8) & 2) == 2);
    assert(nic_read(nic, 0x5400) == 0x12005452);
    assert((nic_read(nic, 0x5404) & 0xffff) == 0x134);
    uint64_t value;
    assert(ghostos_vm_e1000_read(nic, 0, 2, true, &value) == GHOSTOS_VM_E1000_SIZE);
    nic_write(nic, 0xd0, 1);
    ghostos_vm_e1000_signal(nic, 1, NULL);
    assert(nic_read(nic, 0xc0) == 1);
    nic_write(nic, 0, 1u << 26);
    assert(nic_read(nic, 0x5400) == 0x12005452);
    assert((nic_read(nic, 0x5404) & 0xffff) == 0x134);
    assert(nic_read(nic, 0xc0) == 0);
    ghostos_vm_e1000_free(nic);
}

typedef struct {
    uint8_t memory[0x20000];
    uint8_t frame[60];
    bool pending;
} network_fixture;
static bool read_memory(void *raw, uint64_t address, uint8_t *out, size_t length) {
    network_fixture *fixture = raw;
    if (address > sizeof(fixture->memory) || length > sizeof(fixture->memory) - address) return false;
    memcpy(out, fixture->memory + address, length);
    return true;
}
static bool write_memory(void *raw, uint64_t address, const uint8_t *bytes, size_t length) {
    network_fixture *fixture = raw;
    if (address > sizeof(fixture->memory) || length > sizeof(fixture->memory) - address) return false;
    memcpy(fixture->memory + address, bytes, length);
    return true;
}
static int32_t transmit(void *raw, const uint8_t *packet, size_t length) {
    network_fixture *fixture = raw;
    assert(length == sizeof(fixture->frame));
    memcpy(fixture->frame, packet, length);
    fixture->pending = true;
    return -1;
}
static int32_t receive(void *raw, const uint8_t **packet, size_t *length) {
    network_fixture *fixture = raw;
    if (!fixture->pending) return 0;
    *packet = fixture->frame;
    *length = sizeof(fixture->frame);
    fixture->pending = false;
    return 1;
}

/* Port of tx_and_rx_descriptor_rings_move_a_frame. */
static void e1000_rings(void) {
    const uint8_t tx_mac[] = {0x52,0x54,0,0x12,0x34,1};
    const uint8_t rx_mac[] = {0x52,0x54,0,0x12,0x34,2};
    ghostos_vm_e1000 *tx = ghostos_vm_e1000_new(tx_mac);
    ghostos_vm_e1000 *rx = ghostos_vm_e1000_new(rx_mac);
    assert(tx && rx);
    network_fixture fixture = {0};
    ghostos_vm_e1000_io io = {read_memory, write_memory, receive, transmit, NULL, &fixture};
    uint8_t frame[60] = {0};
    memcpy(frame, rx_mac, 6);
    memcpy(frame + 6, tx_mac, 6);
    frame[12] = 8;
    memcpy(fixture.memory + 0x3000, frame, sizeof(frame));
    fixture.memory[0x1001] = 0x30;
    fixture.memory[0x1008] = sizeof(frame);
    nic_write(tx, 0x3800, 0x1000);
    nic_write(tx, 0x3808, 16);
    nic_write(tx, 0x400, 1);
    assert(ghostos_vm_e1000_poll(tx, &io, true) == 0);
    assert((fixture.memory[0x100c] & 3) == 3);
    fixture.memory[0x2011] = 0x40;
    fixture.memory[0x2019] = 2;
    nic_write(rx, 0x2800, 0x2000);
    nic_write(rx, 0x2808, 32);
    nic_write(rx, 0x2818, 0);
    nic_write(rx, 0x100, 1);
    assert(ghostos_vm_e1000_poll(rx, &io, true) == 0);
    assert(memcmp(fixture.memory + 0x4000, frame, sizeof(frame)) == 0);
    ghostos_vm_e1000_free(tx);
    ghostos_vm_e1000_free(rx);
}

/* Policy portions of fake_terminal_input_output_are_deterministic,
 * terminal_transcript_replays_policy_events_without_host_state, and
 * fake_terminal_eof_becomes_ctrl_d. Stream/thread and PTY fixtures remain Rust. */
static void terminal_policy(void) {
    ghostos_vm_terminal *terminal = ghostos_vm_terminal_new();
    assert(terminal);
    const uint8_t raw[] = {'h','i',0x7f,0x1b,'[','A'};
    const uint8_t expected[] = {'h','i',8,0x1b,'[','A'};
    assert(!ghostos_vm_terminal_poll_begin(terminal, 0, false));
    assert(ghostos_vm_terminal_accept_input(terminal, raw, sizeof(raw)));
    ghostos_vm_terminal_input input;
    ghostos_vm_terminal_poll_input(terminal, &input);
    assert(input.length == sizeof(expected));
    assert(memcmp(input.bytes, expected, sizeof(expected)) == 0);
    ghostos_vm_terminal_output_written(terminal, 12);
    ghostos_vm_terminal_output_flushed(terminal);
    ghostos_vm_terminal_counters counters;
    ghostos_vm_terminal_counters_get(terminal, &counters);
    assert(counters.output_bytes == 12 && counters.output_flushes == 1);
    ghostos_vm_terminal_event event;
    assert(ghostos_vm_terminal_event_get(terminal, 0, &event));
    assert(event.kind == 1 && event.raw_length == sizeof(raw) && event.length == sizeof(expected));
    assert(memcmp(event.raw, raw, sizeof(raw)) == 0);
    assert(memcmp(event.bytes, expected, sizeof(expected)) == 0);
    assert(ghostos_vm_terminal_replay_event(&event, &input));
    assert(input.length == sizeof(expected) && memcmp(input.bytes, expected, sizeof(expected)) == 0);
    assert(!ghostos_vm_terminal_poll_begin(terminal, 0, false));
    assert(ghostos_vm_terminal_accept_eof(terminal));
    ghostos_vm_terminal_poll_input(terminal, &input);
    assert(input.length == 1 && input.bytes[0] == 4);
    ghostos_vm_terminal_free(terminal);
}

/* Translation portion of terminal_translation_and_ps2_fallback_are_stable. */
static void terminal_translation(void) {
    const uint8_t raw[] = {'h','i','\r',0x7f,'\t',3,4,0x1b,'[','A'};
    const uint8_t expected[] = {'h','i','\r',8,'\t',3,4,0x1b,'[','A'};
    uint8_t output[sizeof(raw)];
    size_t length;
    bool previous_cr = false;
    assert(ghostos_vm_terminal_translate(raw, sizeof(raw), &previous_cr, output, sizeof(output), &length));
    assert(length == sizeof(expected) && memcmp(output, expected, length) == 0);
}

/* Port of shared_segment_delivers_unicast_and_broadcast. */
static void shared_segment_delivery(void) {
    ghostos_vm_segment *s = ghostos_vm_segment_new(3, false);
    assert(s);
    const ghostos_vm_mac_address macs[] = {{{0x52,0x54,0,0x12,0x34,0x60}},
        {{0x52,0x54,0,0x12,0x34,0x61}}, {{0x52,0x54,0,0x12,0x34,0x62}}};
    size_t ports[3];
    for (size_t i = 0; i < 3; ++i) assert(ghostos_vm_segment_connect(s, &macs[i], &ports[i]) == -1);
    uint8_t packet[14] = {0}, output[1518]; size_t length;
    memcpy(packet, macs[1].bytes, 6); memcpy(packet + 6, macs[0].bytes, 6);
    assert(ghostos_vm_segment_transmit(s, ports[0], packet, sizeof(packet)) == -1);
    assert(ghostos_vm_segment_receive(s, ports[1], output, &length) == 1);
    assert(ghostos_vm_segment_receive(s, ports[2], output, &length) == 0);
    memset(packet, 0xff, 6);
    assert(ghostos_vm_segment_transmit(s, ports[0], packet, sizeof(packet)) == -1);
    assert(ghostos_vm_segment_receive(s, ports[1], output, &length) == 1);
    assert(ghostos_vm_segment_receive(s, ports[2], output, &length) == 1);
    ghostos_vm_segment_free(s);
}

/* Port of administrative_state_is_distinct_from_carrier_state. */
static void shared_segment_admin(void) {
    ghostos_vm_segment *s = ghostos_vm_segment_new(1, false);
    assert(s);
    const ghostos_vm_mac_address mac = {{0x52,0x54,0,0x12,0x34,0x63}};
    size_t port;
    assert(ghostos_vm_segment_connect(s, &mac, &port) == -1);
    assert(ghostos_vm_segment_link(s));
    ghostos_vm_segment_set_admin(s, port, false);
    assert(ghostos_vm_segment_link(s));
    assert(!ghostos_vm_segment_admin(s, port));
    uint8_t packet[14] = {0}; memset(packet, 0xff, 6); memcpy(packet + 6, mac.bytes, 6);
    assert(ghostos_vm_segment_transmit(s, port, packet, sizeof(packet)) == 4);
    ghostos_vm_segment_set_link(s, false);
    assert(!ghostos_vm_segment_link(s));
    ghostos_vm_segment_free(s);
}

int main(void) {
    e1000_registers();
    e1000_rings();
    terminal_policy();
    terminal_translation();
    shared_segment_delivery();
    shared_segment_admin();
    puts("C VM I/O contracts passed");
    return 0;
}
