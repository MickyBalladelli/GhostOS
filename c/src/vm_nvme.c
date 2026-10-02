#include "ghostos/vm_nvme.h"
#include <stdlib.h>
#include <string.h>

#define MAX_QUEUES 4096
#define MAX_DEPTH 1024
#define MAX_TRANSFER (16u * 1024u * 1024u)
enum { SUCCESS = 0, INVALID_OPCODE = 1, INVALID_FIELD = 2, INVALID_QID = 3,
    SIZE_EXCEEDED = 8, INVALID_NS = 11, LBA_RANGE = 128, CAP_EXCEEDED = 129, NS_NOT_READY = 133 };

typedef struct {
    uint64_t base;
    uint16_t depth, completion_queue, tail, head;
    bool phase, deleted;
} queue;

struct ghostos_vm_nvme {
    uint64_t asq, acq;
    uint32_t intms, intmc, cc, csts, aqa;
    queue admin_sq, admin_cq;
    queue *sq, *cq;
    uint16_t *pending;
    size_t sq_count, cq_count;
    uint16_t admin_pending;
    bool checked;
};

static uint16_t get16(const uint8_t *p) { return (uint16_t)(p[0] | (uint16_t)p[1] << 8); }
static uint32_t get32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}
static uint64_t get64(const uint8_t *p) { return get32(p) | (uint64_t)get32(p + 4) << 32; }
static void put16(uint8_t *p, uint16_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); }
static void put32(uint8_t *p, uint32_t v) { for (size_t i = 0; i < 4; ++i) p[i] = (uint8_t)(v >> (8 * i)); }
static void put64(uint8_t *p, uint64_t v) { for (size_t i = 0; i < 8; ++i) p[i] = (uint8_t)(v >> (8 * i)); }
static queue empty_queue(void) { return (queue){.phase = true, .deleted = true}; }
static queue active_queue(uint64_t base, uint16_t depth, uint16_t cq) {
    return (queue){.base = base, .depth = depth, .completion_queue = cq, .phase = true};
}
static void clear_queues(ghostos_vm_nvme *n) {
    free(n->sq); free(n->cq); free(n->pending);
    n->sq = NULL; n->cq = NULL; n->pending = NULL;
    n->sq_count = n->cq_count = 0;
}
ghostos_vm_nvme *ghostos_vm_nvme_new(bool checked) {
    ghostos_vm_nvme *n = calloc(1, sizeof(*n));
    if (n) { n->checked = checked; n->admin_sq = n->admin_cq = empty_queue(); }
    return n;
}
void ghostos_vm_nvme_free(ghostos_vm_nvme *n) { if (n) { clear_queues(n); free(n); } }
void ghostos_vm_nvme_reset(ghostos_vm_nvme *n) {
    bool checked = n->checked;
    clear_queues(n);
    memset(n, 0, sizeof(*n));
    n->checked = checked; n->admin_sq = n->admin_cq = empty_queue();
}
bool ghostos_vm_nvme_pending(const ghostos_vm_nvme *n) {
    if (n->admin_pending) return true;
    for (size_t i = 0; i < n->sq_count; ++i) if (n->pending[i]) return true;
    return false;
}
static uint64_t read_register(const ghostos_vm_nvme *n, uint64_t off) {
    switch (off) {
        case 0: return (MAX_DEPTH - 1) | (UINT64_C(1) << 37) | (UINT64_C(8) << 24);
        case 8: return 0x00010300;
        case 0xc: return n->intms;
        case 0x10: return n->intmc;
        case 0x14: return n->cc;
        case 0x1c: return n->csts;
        case 0x24: return n->aqa;
        case 0x28: return n->asq;
        case 0x30: return n->acq;
        default: return 0;
    }
}
uint32_t ghostos_vm_nvme_read(const ghostos_vm_nvme *n, uint64_t address, uint8_t size, uint64_t *value) {
    uint64_t off = address & 0x1fff;
    if (off < 0x1000) {
        if (size != 4 && size != 8) return GHOSTOS_VM_NVME_SIZE;
        *value = read_register(n, off);
        if (size == 4) *value &= UINT32_MAX;
    } else {
        if (size != 4) return GHOSTOS_VM_NVME_SIZE;
        *value = 0;
    }
    return 0;
}
uint32_t ghostos_vm_nvme_write(ghostos_vm_nvme *n, uint64_t address, uint8_t size, uint64_t value) {
    uint64_t off = address & 0x1fff;
    if (off < 0x1000) {
        if (size != 4 && size != 8) return GHOSTOS_VM_NVME_SIZE;
        switch (off) {
            case 0xc: n->intms |= (uint32_t)value; break;
            case 0x10: n->intmc |= (uint32_t)value; break;
            case 0x24: n->aqa = (uint32_t)value; break;
            case 0x28: n->asq = value; break;
            case 0x30: n->acq = value; break;
            case 0x14: {
                bool was_enabled = (n->cc & 1) != 0;
                n->cc = (uint32_t)value;
                if ((n->cc & 1) && !was_enabled) {
                    uint16_t sd = (uint16_t)((n->aqa & 0xfff) + 1);
                    uint16_t cd = (uint16_t)(((n->aqa >> 16) & 0xfff) + 1);
                    n->admin_sq = active_queue(n->asq, sd < 2 ? 2 : sd, 0);
                    n->admin_cq = active_queue(n->acq, cd < 2 ? 2 : cd, 0);
                    clear_queues(n);
                    n->csts |= 1;
                } else if (!(n->cc & 1) && was_enabled) n->csts &= ~UINT32_C(1);
                break;
            }
            default: break;
        }
    } else {
        if (size != 4) return GHOSTOS_VM_NVME_SIZE;
        if (off % 4) return 0;
        size_t index = (size_t)((off - 0x1000) / 4);
        uint16_t tail = (uint16_t)value;
        if (index == 0) {
            if (tail < n->admin_sq.depth) n->admin_sq.tail = tail;
            n->admin_pending = (uint16_t)(n->admin_pending + 1);
        } else if (index == 1) n->admin_cq.head = tail;
        else {
            size_t qid = index / 2;
            if (!(index % 2) && qid <= n->sq_count) {
                if (tail < n->sq[qid - 1].depth) {
                    n->sq[qid - 1].tail = tail;
                    n->pending[qid - 1] = (uint16_t)(n->pending[qid - 1] + 1);
                }
            } else if (index % 2 && qid <= n->cq_count) n->cq[qid - 1].head = tail;
        }
    }
    return 0;
}

static bool grow_queues(ghostos_vm_nvme *n, size_t count, bool submission) {
    size_t old = submission ? n->sq_count : n->cq_count;
    if (count <= old) return true;
    queue *entries = malloc(count * sizeof(*entries));
    uint16_t *pending = submission ? calloc(count, sizeof(*pending)) : NULL;
    if (!entries || (submission && !pending)) { free(entries); free(pending); return false; }
    if (old) {
        memcpy(entries, submission ? n->sq : n->cq, old * sizeof(*entries));
        if (submission) memcpy(pending, n->pending, old * sizeof(*pending));
    }
    for (size_t i = old; i < count; ++i) entries[i] = empty_queue();
    if (submission) {
        free(n->sq); free(n->pending);
        n->sq = entries; n->pending = pending; n->sq_count = count;
    } else { free(n->cq); n->cq = entries; n->cq_count = count; }
    return true;
}

uint32_t ghostos_vm_nvme_admin(ghostos_vm_nvme *n, const ghostos_vm_storage_io *io, const uint8_t cmd[64], uint32_t *status) {
    uint8_t opcode = cmd[0];
    uint32_t nsid = get32(cmd + 4), dw10 = get32(cmd + 40), dw12 = get32(cmd + 48);
    uint64_t prp = get64(cmd + 8);
    *status = SUCCESS;
    if (opcode == 1 || opcode == 5) {
        uint16_t encoded = (uint16_t)(dw10 >> 16);
        if (encoded == UINT16_MAX && n->checked) return GHOSTOS_VM_NVME_LEGACY_OVERFLOW;
        uint16_t depth = (uint16_t)(encoded + 1);
        size_t qid = dw10 & 0xffff, cqid = dw12 & 0xffff;
        if (!qid || qid > MAX_QUEUES || (opcode == 1 && (!cqid || cqid > MAX_QUEUES))) *status = INVALID_QID;
        else if (depth < 2) *status = INVALID_FIELD;
        else if (depth > MAX_DEPTH) *status = SIZE_EXCEEDED;
        else if (opcode == 1 && cqid > n->cq_count) *status = INVALID_QID;
        else {
            if (!grow_queues(n, qid, opcode == 1)) return GHOSTOS_VM_NVME_ALLOCATION;
            if (opcode == 1) n->sq[qid - 1] = active_queue(prp, depth, (uint16_t)cqid);
            else n->cq[qid - 1] = active_queue(prp, depth, 0);
        }
    } else if (opcode == 0 || opcode == 4) {
        size_t qid = dw10 & 0xffff, count = opcode == 0 ? n->sq_count : n->cq_count;
        if (!qid || qid > count) *status = INVALID_QID;
        else (opcode == 0 ? n->sq : n->cq)[qid - 1].deleted = true;
    } else if (opcode == 6) {
        if (nsid > 1) { *status = INVALID_NS; return 0; }
        uint8_t id[4096] = {0};
        switch ((uint8_t)dw10) {
            case 0: {
                uint64_t sectors;
                if (!io->sector_count(io->context, &sectors)) { *status = INVALID_NS; return 0; }
                put64(id, sectors); put64(id + 8, sectors); id[129] = 9;
                break;
            }
            case 1: {
                put16(id, 0x8086); put16(id + 2, 0x8086);
                memcpy(id + 4, "SYNOSVM00001", 12);
                const char model[] = "GhostOS NVMe Virtual Disk";
                memcpy(id + 24, model, sizeof(model) - 1);
                memcpy(id + 64, "0.1", 4);
                id[512] = id[513] = 6; id[514] = id[515] = 4; put32(id + 516, 1);
                break;
            }
            case 2: put32(id, 1); break;
            default: *status = INVALID_FIELD; return 0;
        }
        if (!io->write_memory(io->context, prp, id, sizeof(id))) *status = CAP_EXCEEDED;
    } else if (opcode == 9 || opcode == 10) {
        switch ((uint8_t)dw10) {
            case 1: case 7: {
                const uint8_t features[] = {1, 0, 1, 0};
                if (opcode == 10) (void)io->write_memory(io->context, prp, features, sizeof(features));
                break;
            }
            case 2: case 4: break;
            default: *status = INVALID_FIELD; break;
        }
    } else if (opcode == 2) {
        const uint8_t log[512] = {0};
        (void)io->write_memory(io->context, prp, log, sizeof(log));
    } else if (opcode != 8 && opcode != 12) *status = INVALID_OPCODE;
    return 0;
}

uint32_t ghostos_vm_nvme_command(ghostos_vm_nvme *n, const ghostos_vm_storage_io *io, const uint8_t cmd[64], uint32_t *status) {
    (void)n;
    *status = SUCCESS;
    if (get32(cmd + 4) != 1) { *status = INVALID_NS; return 0; }
    uint8_t opcode = cmd[0];
    uint64_t prp = get64(cmd + 8), sectors;
    uint32_t dw11 = get32(cmd + 44);
    uint64_t lba = get32(cmd + 40) | (uint64_t)(dw11 & 0xffff) << 32;
    size_t count = (size_t)(dw11 >> 16) + 1;
    if (opcode == 0) {
        if (!io->flush_disk(io->context)) *status = NS_NOT_READY;
        return 0;
    }
    if (opcode != 1 && opcode != 2) { *status = INVALID_OPCODE; return 0; }
    if (!io->sector_count(io->context, &sectors)) { *status = NS_NOT_READY; return 0; }
    if (lba > UINT64_MAX - count || lba + count > sectors) { *status = LBA_RANGE; return 0; }
    size_t total = count * 512;
    if (total > MAX_TRANSFER) { *status = CAP_EXCEEDED; return 0; }
    uint8_t *buffer = calloc(1, total);
    if (!buffer) return GHOSTOS_VM_NVME_ALLOCATION;
    if (opcode == 2) {
        for (size_t i = 0; i < count; ++i) {
            uint8_t sector[512] = {0};
            if (!io->read_sector(io->context, lba + i, sector)) { *status = LBA_RANGE; break; }
            memcpy(buffer + i * 512, sector, 512);
        }
        if (*status == SUCCESS && !io->write_memory(io->context, prp, buffer, total)) *status = CAP_EXCEEDED;
    } else if (!io->read_memory(io->context, prp, buffer, total)) *status = CAP_EXCEEDED;
    else {
        for (size_t i = 0; i < count; ++i) {
            if (!io->write_sector(io->context, lba + i, buffer + i * 512)) { *status = LBA_RANGE; break; }
        }
    }
    free(buffer);
    return 0;
}

static bool read_sq(const ghostos_vm_storage_io *io, const queue *q, uint16_t index, uint8_t cmd[64]) {
    uint64_t offset = (uint64_t)index * 64;
    return !q->deleted && q->depth >= 2 && q->base % 4096 == 0
        && io->validate_dma(io->context, q->base, (size_t)q->depth * 64, 4096)
        && q->base <= UINT64_MAX - offset && io->read_memory(io->context, q->base + offset, cmd, 64);
}
static bool write_cq(const ghostos_vm_storage_io *io, const queue *q, uint32_t status, uint16_t head, uint16_t cid) {
    if (q->deleted || q->depth < 2 || q->base % 4096
        || !io->validate_dma(io->context, q->base, (size_t)q->depth * 16, 4096)) return false;
    uint8_t entry[16] = {0};
    put16(entry + 8, head); put16(entry + 10, (uint16_t)(((status & 0x7fff) << 1) | q->phase));
    put16(entry + 14, cid);
    uint64_t offset = (uint64_t)q->tail * 16;
    return q->base <= UINT64_MAX - offset && io->write_memory(io->context, q->base + offset, entry, 16);
}
static void advance_cq(queue *q) {
    uint16_t next = (uint16_t)(q->tail + 1);
    if (next == q->depth) { q->phase = !q->phase; q->tail = 0; } else q->tail = next;
}
static uint32_t process_admin(ghostos_vm_nvme *n, const ghostos_vm_storage_io *io) {
    uint16_t head = n->admin_sq.head;
    while (head != n->admin_sq.tail) {
        uint8_t cmd[64] = {0};
        if (!read_sq(io, &n->admin_sq, head, cmd)) break;
        uint32_t status, result = ghostos_vm_nvme_admin(n, io, cmd, &status);
        if (result) return result;
        if (!write_cq(io, &n->admin_cq, status, head, get16(cmd + 2))) break;
        advance_cq(&n->admin_cq);
        head = (uint16_t)((uint16_t)(head + 1) % n->admin_sq.depth);
        if (io->interrupt) io->interrupt(io->context);
    }
    n->admin_sq.head = head;
    return 0;
}
static uint32_t process_io(ghostos_vm_nvme *n, const ghostos_vm_storage_io *io, size_t index) {
    queue *sq = &n->sq[index];
    uint16_t head = sq->head;
    while (head != sq->tail) {
        uint8_t cmd[64] = {0};
        if (!read_sq(io, sq, head, cmd)) break;
        uint32_t status, result = ghostos_vm_nvme_command(n, io, cmd, &status);
        if (result) return result;
        size_t cq_index = sq->completion_queue ? (size_t)sq->completion_queue - 1 : UINT16_MAX;
        if (cq_index >= n->cq_count || sq->deleted || n->cq[cq_index].deleted) break;
        queue *cq = &n->cq[cq_index];
        if (!write_cq(io, cq, status, head, get16(cmd + 2))) break;
        advance_cq(cq);
        head = (uint16_t)((uint16_t)(head + 1) % sq->depth);
        if (io->interrupt) io->interrupt(io->context);
    }
    sq->head = head;
    return 0;
}
uint32_t ghostos_vm_nvme_poll(ghostos_vm_nvme *n, const ghostos_vm_storage_io *io) {
    if (n->admin_pending) {
        uint32_t result = process_admin(n, io);
        if (result) return result;
        n->admin_pending = 0;
    }
    for (size_t i = 0; i < n->sq_count; ++i) {
        if (n->pending[i]) {
            uint32_t result = process_io(n, io, i);
            if (result) return result;
            n->pending[i] = 0;
        }
    }
    return 0;
}
