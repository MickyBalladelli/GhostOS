#include "ghostos/time_sync.h"
#include <assert.h>
#include <string.h>

static void manual_clock_is_injectable_and_never_moves_backwards(void) {
    _Atomic uint64_t clock = 10;
    assert(ghostos_manual_clock_now(&clock) == 10);
    ghostos_manual_clock_advance(&clock, 5);
    assert(ghostos_manual_clock_now(&clock) == 15);
    ghostos_manual_clock_set(&clock, 3);
    assert(ghostos_manual_clock_now(&clock) == 15);
}

static void ptp_wire_round_trip_and_timestamp_bounds(void) {
    ghostos_ptp_timestamp timestamp;
    assert(ghostos_time_timestamp_init(3, 400, &timestamp) == 0);
    ghostos_ptp_message message = {.kind = 0, .sequence = 7, .source = 1, .target = 2,
        .first = timestamp, .correction = -4};
    uint8_t wire[GHOSTOS_PTP_PACKET_BYTES];
    assert(ghostos_ptp_encode(&message, wire, sizeof(wire)) == 0);
    ghostos_ptp_message decoded;
    assert(ghostos_ptp_decode(wire, sizeof(wire), &decoded) == 0);
    assert(decoded.kind == message.kind && decoded.sequence == message.sequence &&
        decoded.source == message.source && decoded.target == message.target &&
        decoded.first.seconds == timestamp.seconds && decoded.first.nanoseconds == timestamp.nanoseconds &&
        decoded.correction == message.correction);
    wire[0] = 'X';
    assert(ghostos_ptp_decode(wire, sizeof(wire), &decoded) == 3);
    ghostos_ptp_timestamp invalid;
    assert(ghostos_time_timestamp_init(0, 1000000000, &invalid) == 2);
    uint64_t nanos;
    assert(ghostos_time_to_nanos(&timestamp, &nanos) == 0 && nanos == UINT64_C(3000000400));
}

static void clock_never_moves_global_time_backwards(void) {
    ghostos_cluster_clock clock = {0};
    ghostos_sync_measurement measurement = {1, 1, 10000, 20};
    ghostos_clock_adjustment adjustment;
    ghostos_clock_discipline(&clock, &measurement, 100, &adjustment);
    assert(adjustment.stepped);
    uint64_t first = ghostos_clock_corrected_time(&clock, 100);
    uint64_t second = ghostos_clock_corrected_time(&clock, 90);
    assert(second > first && clock.state.synchronized);
}

static void daemon_rejects_wrong_role_and_handles_two_step_exchange(void) {
    ghostos_ptp_daemon master, slave;
    assert(ghostos_ptp_daemon_init(&master, 1, 0) == 0);
    assert(ghostos_ptp_daemon_init(&slave, 2, 1) == 0);
    ghostos_ptp_message sync, follow_up, request, response;
    assert(ghostos_ptp_sync(&master, 2, 0, true, &sync) == 0);
    assert(ghostos_ptp_follow_up(&master, &sync, 1000, &follow_up) == 0);
    assert(ghostos_ptp_receive_sync(&slave, &sync, 1100) == 0);
    assert(ghostos_ptp_receive_follow_up(&slave, &follow_up) == 0);
    assert(ghostos_ptp_delay_request(&slave, 1200, &request) == 0);
    assert(ghostos_ptp_receive_delay_request(&master, &request, 1300, &response) == 0);
    ghostos_sync_measurement measurement;
    assert(ghostos_ptp_receive_delay_response(&slave, &response, 1400, &measurement) == 0);
    assert(measurement.sequence_id == sync.sequence && slave.clock.state.synchronized);
    assert(ghostos_ptp_sync(&master, 1, 0, false, &sync) == 1);
    ghostos_ptp_daemon invalid;
    assert(ghostos_ptp_daemon_init(&invalid, 0, 0) == 1);
}

static void monotonic_epoch_counter_fences_invalid_stamps(void) {
    ghostos_epoch_counter counter;
    assert(ghostos_epoch_init(&counter, 2, 10) == 0);
    ghostos_epoch_stamp first, second;
    assert(ghostos_epoch_issue(&counter, 10, &first) == 0);
    assert(ghostos_epoch_issue(&counter, 1, &second) == 0);
    assert(second.counter > first.counter);
    ghostos_epoch_stamp remote = {4, 99, 1};
    assert(ghostos_epoch_observe(&counter, &remote) == 0 && counter.epoch == 4);
    remote = (ghostos_epoch_stamp){5, 0, 0};
    assert(ghostos_epoch_observe(&counter, &remote) == 1);
}

int main(void) {
    manual_clock_is_injectable_and_never_moves_backwards();
    ptp_wire_round_trip_and_timestamp_bounds();
    clock_never_moves_global_time_backwards();
    daemon_rejects_wrong_role_and_handles_two_step_exchange();
    monotonic_epoch_counter_fences_invalid_stamps();
    return 0;
}
