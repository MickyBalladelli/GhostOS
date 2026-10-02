#include "ghostos/vm_segment.h"
#include "ghostos/vm_net.h"
#include "ghostos/vm_packet.h"
#include <stdlib.h>
#include <string.h>

typedef struct frame {
    struct frame *next;
    size_t length;
    uint8_t bytes[];
} frame;
typedef struct {
    ghostos_vm_mac_address mac;
    frame *head, *tail;
    size_t queued;
    bool connected, admin, promiscuous;
} port;
struct ghostos_vm_segment {
    port *ports;
    size_t count, capacity, limit, transmitted, drop_next;
    bool up, loopback;
};
static void increment(size_t *n) { if (*n != SIZE_MAX) ++*n; }
static void pop(port *p) {
    frame *f = p->head;
    p->head = f->next;
    if (!p->head) p->tail = NULL;
    --p->queued; free(f);
}
static void clear(port *p) { while (p->head) pop(p); }
static frame *copy_frame(const uint8_t *bytes, size_t length) {
    size_t padded = length < 60 ? 60 : length;
    frame *f = calloc(1, sizeof(*f) + padded);
    if (f) { f->length = padded; memcpy(f->bytes, bytes, length); }
    return f;
}
static void append(port *p, frame *f) {
    if (p->tail) p->tail->next = f; else p->head = f;
    p->tail = f; ++p->queued;
}
ghostos_vm_segment *ghostos_vm_segment_new(size_t capacity, bool loopback) {
    if (loopback) capacity = 2;
    if (capacity > SIZE_MAX / sizeof(port)) return NULL;
    ghostos_vm_segment *s = calloc(1, sizeof(*s));
    if (!s) return NULL;
    s->ports = capacity ? calloc(capacity, sizeof(port)) : NULL;
    if (capacity && !s->ports) { free(s); return NULL; }
    s->capacity = capacity; s->loopback = loopback; s->up = true; s->limit = 256;
    if (loopback) {
        s->count = 2;
        for (size_t i = 0; i < 2; ++i) s->ports[i].connected = s->ports[i].admin = true;
    }
    return s;
}
void ghostos_vm_segment_free(ghostos_vm_segment *s) {
    if (s) { ghostos_vm_segment_clear(s); free(s->ports); free(s); }
}
void ghostos_vm_segment_clear(ghostos_vm_segment *s) {
    for (size_t i = 0; i < s->count; ++i) clear(&s->ports[i]);
}
void ghostos_vm_segment_set_link(ghostos_vm_segment *s, bool up) {
    s->up = up;
    if (!up && !s->loopback) ghostos_vm_segment_clear(s);
}
bool ghostos_vm_segment_link(const ghostos_vm_segment *s) { return s->up; }
void ghostos_vm_segment_set_limit(ghostos_vm_segment *s, size_t limit) {
    s->limit = limit;
    for (size_t i = 0; i < s->count; ++i) while (s->ports[i].queued > limit) pop(&s->ports[i]);
}
void ghostos_vm_segment_drop_next(ghostos_vm_segment *s, size_t count) { s->drop_next = count; }
bool ghostos_vm_segment_disconnect(ghostos_vm_segment *s, const ghostos_vm_mac_address *mac) {
    for (size_t i = 0; i < s->count; ++i) {
        if (!memcmp(s->ports[i].mac.bytes, mac->bytes, 6)) {
            s->ports[i].connected = false; clear(&s->ports[i]); return true;
        }
    }
    return false;
}
int32_t ghostos_vm_segment_connect(ghostos_vm_segment *s, const ghostos_vm_mac_address *mac, size_t *index) {
    for (size_t i = 0; i < s->count; ++i) if (!memcmp(s->ports[i].mac.bytes, mac->bytes, 6)) return 5;
    if (s->count == s->capacity) return 5;
    *index = s->count++;
    s->ports[*index] = (port){.mac = *mac, .connected = true, .admin = true};
    return -1;
}
void ghostos_vm_segment_loopback_port(ghostos_vm_segment *s, size_t index, const ghostos_vm_mac_address *mac) {
    if (s->loopback && index < 2) s->ports[index].mac = *mac;
}
bool ghostos_vm_segment_admin(const ghostos_vm_segment *s, size_t index) {
    return index < s->count && s->ports[index].admin;
}
void ghostos_vm_segment_set_admin(ghostos_vm_segment *s, size_t index, bool up) {
    if (index < s->count) {
        s->ports[index].admin = up;
        if (!up && !s->loopback) clear(&s->ports[index]);
    }
}
void ghostos_vm_segment_set_promiscuous(ghostos_vm_segment *s, size_t index, bool enabled) {
    if (index < s->count) s->ports[index].promiscuous = enabled;
}
size_t ghostos_vm_segment_queued(const ghostos_vm_segment *s, size_t index) {
    return index < s->count ? s->ports[index].queued : 0;
}
size_t ghostos_vm_segment_transmitted(const ghostos_vm_segment *s) { return s->transmitted; }
static bool recipient(const ghostos_vm_segment *s, size_t from, size_t to, const uint8_t *bytes, size_t length) {
    const port *p = &s->ports[to];
    return to != from && ghostos_vm_net_segment_accepts(bytes, length, &p->mac, p->connected, p->admin);
}
int32_t ghostos_vm_segment_transmit(ghostos_vm_segment *s, size_t from, const uint8_t *bytes, size_t length) {
    /* Loopback's admin error precedes carrier/length checks. Shared segment
     * preserves its different carrier/length/source/admin order. */
    if (s->loopback && from < s->count && !s->ports[from].admin) return 4;
    if (!s->up) return 3;
    if (length < 14) return 1;
    if (length > 1518) return 0;
    if (from >= s->count || !s->ports[from].connected) return 5;
    if (!s->ports[from].admin) return 4;
    if (s->loopback) {
        port *to = &s->ports[1 - from];
        if (to->queued >= 256) return 2;
        frame *f = copy_frame(bytes, length);
        if (!f) return -2;
        append(to, f); increment(&s->transmitted); return -1;
    }
    for (size_t i = 0; i < s->count; ++i) {
        if (recipient(s, from, i, bytes, length) && s->ports[i].queued >= s->limit) return 2;
    }
    if (s->drop_next) { --s->drop_next; increment(&s->transmitted); return -1; }
    for (size_t i = 0; i < s->count; ++i) {
        if (!recipient(s, from, i, bytes, length)) continue;
        frame *f = copy_frame(bytes, length);
        if (!f) return -2;
        append(&s->ports[i], f);
    }
    increment(&s->transmitted); return -1;
}
int32_t ghostos_vm_segment_receive(ghostos_vm_segment *s, size_t index, uint8_t output[1518], size_t *length) {
    *length = 0;
    if (s->loopback && index < s->count && !s->ports[index].admin) return -5;
    if (!s->up) return -4;
    if (index >= s->count || !s->ports[index].connected) return -6;
    port *p = &s->ports[index];
    if (!p->admin) return -5;
    while (p->head) {
        frame *f = p->head;
        bool accepted = f->length >= 14 && ghostos_vm_mac_matches(f->bytes, 6, &p->mac, p->promiscuous);
        if (accepted) { *length = f->length; memcpy(output, f->bytes, f->length); }
        pop(p);
        if (accepted) return 1;
    }
    return 0;
}
