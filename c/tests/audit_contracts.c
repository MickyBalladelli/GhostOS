#include "ghostos/audit.h"
#include <assert.h>
static void advisories_findings_and_obsolete_packages_keep_order(void) {
    ghostos_audit_advisory advisories[2] = {0};
    ghostos_audit_obsolete_entry obsolete[1] = {0};
    ghostos_audit_finding findings[1] = {0};
    uint8_t package[32] = {1};
    uint8_t other[32] = {2};
    const uint8_t id[] = "RUSTSEC-1";
    size_t index = 9;
    assert(ghostos_audit_id((const uint8_t *)"", 0) == 1);
    assert(ghostos_audit_id((const uint8_t *)"caf\x80", 4) == 1);
    assert(ghostos_audit_id(id, sizeof id - 1) == 0);
    assert(ghostos_audit_advisory_slot(advisories, 2, 0, id, sizeof id - 1, package, &index) == 0);
    assert(index == 0);
    advisories[0].occupied = true;
    advisories[0].source = 0;
    advisories[0].id_length = (uint8_t)(sizeof id - 1);
    for (size_t i = 0; i < sizeof id - 1; ++i) advisories[0].id[i] = id[i];
    for (size_t i = 0; i < 32; ++i) advisories[0].package_hash[i] = package[i];
    assert(ghostos_audit_advisory_slot(advisories, 2, 0, id, sizeof id - 1, package, &index) == 1);
    assert(ghostos_audit_obsolete(false, 2, 1, 0, 0, 1, 0, 0) == 1);
    assert(ghostos_audit_obsolete(false, 2, 1, 0, 0, 1, 2, 0) == 0);
    assert(ghostos_audit_obsolete(true, 0, 0, 0, 0, 1, 0, 0) == 1);
    assert(ghostos_audit_obsolete_slot(obsolete, 1, 4, package, 2, &index) == 0 && index == 0);
    obsolete[0].occupied = true;
    obsolete[0].node = 4;
    obsolete[0].reason = 2;
    for (size_t i = 0; i < 32; ++i) obsolete[0].package[i] = package[i];
    assert(ghostos_audit_obsolete_slot(obsolete, 1, 4, package, 2, &index) == 1);
    assert(ghostos_audit_finding_slot(findings, 1, package, 0, id, sizeof id - 1, other, 3, false, &index) == 0);
    findings[0].occupied = true;
    findings[0].source = 0;
    findings[0].severity = 3;
    findings[0].id_length = (uint8_t)(sizeof id - 1);
    for (size_t i = 0; i < 32; ++i) {
        findings[0].package[i] = package[i];
        findings[0].advisory_package[i] = other[i];
    }
    for (size_t i = 0; i < sizeof id - 1; ++i) findings[0].id[i] = id[i];
    assert(ghostos_audit_finding_slot(findings, 1, package, 0, id, sizeof id - 1, other, 3, false, &index) == 1);
    assert(ghostos_audit_finding_slot(findings, 1, other, 0, id, sizeof id - 1, other, 3, false, &index) == 2);
    assert(ghostos_audit_budget(0) == 1);
    assert(ghostos_audit_budget(3) == 0);
}
int main(void) {
    advisories_findings_and_obsolete_packages_keep_order();
    return 0;
}
