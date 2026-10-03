#include "ghostos/power_policy.h"
_Static_assert(sizeof(ghostos_power_candidate) == 24, "power candidate ABI");
_Static_assert(offsetof(ghostos_power_candidate, valid) == 23, "power validity ABI");
static uint32_t add(uint32_t a, uint32_t b) { return UINT32_MAX - a < b ? UINT32_MAX : a + b; }
static uint32_t sub(uint32_t a, uint32_t b) { return a < b ? 0 : a - b; }
static uint32_t mul(uint32_t a, uint32_t b) { return b && a > UINT32_MAX / b ? UINT32_MAX : a * b; }
uint32_t ghostos_power_frequency(uint32_t minimum, uint32_t maximum, uint8_t load, uint8_t throttle) {
    uint32_t range = sub(maximum, minimum);
    uint32_t requested = add(minimum, mul(range, load) / 100);
    uint32_t allowed = mul(range, sub(100, throttle)) / 100;
    uint32_t increase = sub(requested, minimum);
    return add(minimum, increase < allowed ? increase : allowed);
}
uint8_t ghostos_power_idle(uint64_t now, uint64_t wake, uint64_t budget) {
    uint64_t available = wake < now ? 0 : wake - now;
    uint64_t limit = available < budget ? available : budget;
    return limit >= 100 ? 3 : limit >= 10 ? 2 : limit >= 1 ? 1 : 0;
}
uint8_t ghostos_power_device(uint64_t now, uint64_t active, uint64_t idle_after, uint64_t suspend_after) {
    uint64_t elapsed = now < active ? 0 : now - active;
    return elapsed >= suspend_after ? 2 : elapsed >= idle_after ? 1 : 0;
}
int ghostos_power_place(const ghostos_power_candidate *clusters, size_t count,
    const uint64_t affinity[2], uint8_t class_id, bool preferred, uint8_t preferred_id,
    bool checked, size_t *selected) {
    for (;;) {
        bool found = false;
        uint32_t best = 0;
        for (size_t i = 0; i < count; ++i) {
            const ghostos_power_candidate *cluster = &clusters[i];
            if (!cluster->valid || cluster->throttle == 100 ||
                !((cluster->cpus[0] & affinity[0]) || (cluster->cpus[1] & affinity[1])) ||
                (preferred && cluster->id != preferred_id)) continue;
            uint32_t load = (uint32_t)cluster->load * 100;
            uint32_t penalty = class_id == 1 ? (uint32_t)cluster->throttle * 10 : 0;
            if (checked && UINT32_MAX - load < cluster->idle_power_mw) return 2;
            uint32_t score = load + cluster->idle_power_mw;
            if (checked && UINT32_MAX - score < penalty) return 2;
            score += penalty;
            if (!found || score < best) { found = true; best = score; *selected = i; }
        }
        if (found) return 0;
        if (!preferred) return 1;
        preferred = false;
    }
}
