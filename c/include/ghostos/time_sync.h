#ifndef GHOSTOS_TIME_SYNC_H
#define GHOSTOS_TIME_SYNC_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdatomic.h>
#define GHOSTOS_PTP_PACKET_BYTES 64
#define GHOSTOS_PTP_VERSION 2
/* Errors: 0 success, 1 invalid node, 2 timestamp, 3 packet, 4 short buffer,
 * 5 unexpected message, 6 sequence mismatch, 7 no pending exchange,
 * 8 exchange in progress, 9 unsynchronized, 10 counter overflow. */
typedef struct { uint64_t seconds; uint32_t nanoseconds; } ghostos_ptp_timestamp;
typedef struct {
    uint8_t kind;
    uint16_t sequence;
    uint32_t source, target;
    ghostos_ptp_timestamp first, second;
    int64_t correction;
} ghostos_ptp_message;
typedef struct { uint16_t sequence_id; uint32_t master; int64_t offset_ns, path_delay_ns; } ghostos_sync_measurement;
typedef struct { int64_t offset_ns, frequency_ppb; bool stepped; } ghostos_clock_adjustment;
typedef struct {
    int64_t offset_ns, frequency_ppb, path_delay_ns;
    uint64_t last_update_ns;
    uint32_t samples;
    bool synchronized;
} ghostos_clock_state;
typedef struct { ghostos_clock_state state; uint64_t last_global_ns; } ghostos_cluster_clock;
typedef struct { uint64_t epoch, counter; uint32_t node; } ghostos_epoch_stamp;
typedef struct { uint32_t node; uint64_t epoch, counter; } ghostos_epoch_counter;
typedef struct {
    uint16_t sequence;
    uint32_t master;
    uint64_t t1, t2, t3;
    int64_t correction;
    bool has_t1, has_t2, has_t3;
} ghostos_pending_exchange;
typedef struct {
    uint32_t node;
    uint8_t role; /* 0 master, 1 slave */
    uint16_t next_sequence;
    ghostos_pending_exchange pending;
    bool has_pending;
    ghostos_cluster_clock clock;
    ghostos_epoch_counter epoch;
} ghostos_ptp_daemon;

uint64_t ghostos_manual_clock_now(const _Atomic uint64_t *clock);
void ghostos_manual_clock_set(_Atomic uint64_t *clock, uint64_t now);
void ghostos_manual_clock_advance(_Atomic uint64_t *clock, uint64_t delta);
uint32_t ghostos_time_timestamp_init(uint64_t seconds, uint32_t nanoseconds, ghostos_ptp_timestamp *out);
void ghostos_time_from_nanos(uint64_t value, ghostos_ptp_timestamp *out);
uint32_t ghostos_time_to_nanos(const ghostos_ptp_timestamp *value, uint64_t *out);
uint32_t ghostos_ptp_encode(const ghostos_ptp_message *message, uint8_t *output, size_t capacity);
uint32_t ghostos_ptp_decode(const uint8_t *bytes, size_t length, ghostos_ptp_message *out);
void ghostos_clock_discipline(ghostos_cluster_clock *clock, const ghostos_sync_measurement *measurement,
    uint64_t now, ghostos_clock_adjustment *out);
uint64_t ghostos_clock_corrected_time(ghostos_cluster_clock *clock, uint64_t local);
uint32_t ghostos_epoch_init(ghostos_epoch_counter *counter, uint32_t node, uint64_t initial);
uint32_t ghostos_epoch_issue(ghostos_epoch_counter *counter, uint64_t hardware, ghostos_epoch_stamp *out);
uint32_t ghostos_epoch_observe(ghostos_epoch_counter *counter, const ghostos_epoch_stamp *remote);
uint32_t ghostos_ptp_daemon_init(ghostos_ptp_daemon *daemon, uint32_t node, uint8_t role);
uint32_t ghostos_ptp_sync(ghostos_ptp_daemon *daemon, uint32_t target, uint64_t tx,
    bool two_step, ghostos_ptp_message *out);
uint32_t ghostos_ptp_follow_up(const ghostos_ptp_daemon *daemon,
    const ghostos_ptp_message *sync, uint64_t tx, ghostos_ptp_message *out);
uint32_t ghostos_ptp_receive_sync(ghostos_ptp_daemon *daemon, const ghostos_ptp_message *message, uint64_t rx);
uint32_t ghostos_ptp_receive_follow_up(ghostos_ptp_daemon *daemon, const ghostos_ptp_message *message);
uint32_t ghostos_ptp_delay_request(ghostos_ptp_daemon *daemon, uint64_t tx, ghostos_ptp_message *out);
uint32_t ghostos_ptp_receive_delay_request(const ghostos_ptp_daemon *daemon,
    const ghostos_ptp_message *message, uint64_t rx, ghostos_ptp_message *out);
uint32_t ghostos_ptp_receive_delay_response(ghostos_ptp_daemon *daemon,
    const ghostos_ptp_message *message, uint64_t rx, ghostos_sync_measurement *out);
uint32_t ghostos_ptp_synchronized_time(ghostos_ptp_daemon *daemon, uint64_t local, uint64_t *out);
#endif
