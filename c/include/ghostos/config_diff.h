#ifndef GHOSTOS_CONFIG_DIFF_H
#define GHOSTOS_CONFIG_DIFF_H
#include "ghostos/config_cluster.h"
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Result: 0 success, 1 capacity.
   Area: identity=0, discovery=1, membership=2, quorum=3, transport=4, security=5,
   resources=6, federation=7, node overrides=8, services=9, network=10. */
#define GHOSTOS_CONFIG_DIFF_SERVICE 48u
typedef struct {
    uint64_t revision;
    const ghostos_config_cluster *cluster;
    const uint8_t *description;
    uint8_t description_length;
    const ghostos_config_transport *transports;
    size_t transport_count;
    const ghostos_config_override *overrides;
    size_t override_count;
    const uint8_t *const *service_names;
    const uint8_t *service_lengths;
    size_t service_count;
} ghostos_config_view;
typedef struct {
    uint8_t bytes[GHOSTOS_CONFIG_DIFF_SERVICE];
    uint8_t length;
} ghostos_config_diff_service;
int ghostos_config_diff(const ghostos_config_view *previous, const ghostos_config_view *next, bool network_matches,
    uint64_t *from_revision, bool *has_from, uint8_t *areas, size_t area_capacity, size_t *area_count,
    uint32_t *nodes, size_t node_capacity, size_t *node_count, ghostos_config_diff_service *services,
    size_t service_capacity, size_t *service_count, bool *cluster_wide);
#endif
