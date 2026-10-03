#ifndef GHOSTOS_AUDIT_H
#define GHOSTOS_AUDIT_H
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
/* Identifier and budget: 0 valid, 1 invalid.
 * Advisory and obsolete slots: 0 stored index, 1 duplicate, 2 capacity.
 * Obsolete record: 0 valid, 1 invalid. Finding: 0 insert, 1 duplicate, 2 capacity.
 * Source: RustSec=0, OSV=1, CVE=2. Reason: deprecated=0, unmaintained=1, out of date=2. */
int ghostos_audit_id(const uint8_t *id, size_t length);
typedef struct {
    uint8_t id[64];
    uint8_t package_hash[32];
    uint8_t id_length;
    uint8_t source;
    bool occupied;
} ghostos_audit_advisory;
int ghostos_audit_advisory_slot(const ghostos_audit_advisory *records, size_t count,
    uint8_t source, const uint8_t *id, size_t id_length, const uint8_t package_hash[32],
    size_t *index);
int ghostos_audit_obsolete(bool name_empty, uint8_t reason, uint32_t installed_major,
    uint32_t installed_minor, uint32_t installed_patch, uint32_t latest_major,
    uint32_t latest_minor, uint32_t latest_patch);
typedef struct {
    uint32_t node;
    uint8_t package[32];
    uint8_t reason;
    bool occupied;
} ghostos_audit_obsolete_entry;
int ghostos_audit_obsolete_slot(const ghostos_audit_obsolete_entry *records, size_t count,
    uint32_t node, const uint8_t package[32], uint8_t reason, size_t *index);
typedef struct {
    uint8_t package[32];
    uint8_t advisory_package[32];
    uint8_t id[64];
    uint8_t id_length;
    uint8_t source;
    uint8_t severity;
    bool withdrawn;
    bool occupied;
} ghostos_audit_finding;
int ghostos_audit_finding_slot(const ghostos_audit_finding *records, size_t count,
    const uint8_t package[32], uint8_t source, const uint8_t *id, size_t id_length,
    const uint8_t advisory_package[32], uint8_t severity, bool withdrawn, size_t *index);
int ghostos_audit_budget(size_t package_budget);
#endif
