#include "ghostos/monitor.h"
#include "ghostos/dlm.h"

typedef struct { char bytes[256]; size_t length; } line_buffer;

static void append_char(line_buffer *line, char c) {
    if (line->length < sizeof(line->bytes)) line->bytes[line->length++] = c;
}

static void append_text(line_buffer *line, const char *text) {
    while (*text) append_char(line, *text++);
}

static void append_u64(line_buffer *line, uint64_t value) {
    char digits[20]; size_t count = 0;
    do { digits[count++] = (char)('0' + value % 10); value /= 10; } while (value);
    while (count) append_char(line, digits[--count]);
}

static void append_u64_field(line_buffer *line, uint64_t value, size_t width) {
    char digits[20]; size_t count = 0;
    do { digits[count++] = (char)('0' + value % 10); value /= 10; } while (value);
    for (size_t i = 0; i < count; ++i) append_char(line, digits[count - i - 1]);
    while (count++ < width) append_char(line, ' ');
}

static void append_hex4(line_buffer *line, uint32_t value) {
    static const char digits[] = "0123456789abcdef";
    for (int shift = 12; shift >= 0; shift -= 4) append_char(line, digits[(value >> shift) & 15]);
}

static void append_hex2(line_buffer *line, uint32_t value) {
    static const char digits[] = "0123456789abcdef";
    append_text(line, "0x");
    append_char(line, digits[(value >> 4) & 15]);
    append_char(line, digits[value & 15]);
}

static void append_padded(line_buffer *line, const char *text, size_t width) {
    size_t length = 0;
    while (text[length] && length < width) { append_char(line, text[length]); ++length; }
    while (length++ < width) append_char(line, ' ');
}

static void emit(line_buffer *line, ghostos_monitor_write_fn write, void *context) {
    if (write) write(context, line->bytes, line->length);
}

static void emit_text(const char *text, ghostos_monitor_write_fn write, void *context) {
    if (write) {
        size_t length = 0;
        while (text[length]) ++length;
        write(context, text, length);
    }
}

static const char *state_name(uint8_t state) {
    switch (state) {
        case GHOSTOS_MONITOR_VACANT: return "VACANT";
        case GHOSTOS_MONITOR_READY: return "READY";
        case GHOSTOS_MONITOR_RUNNING: return "RUNNING";
        case GHOSTOS_MONITOR_BLOCKED: return "BLOCKED";
        case GHOSTOS_MONITOR_SLEEPING: return "SLEEPING";
        default: return "????";
    }
}

static const char *policy_name(uint8_t policy) {
    switch (policy) {
        case GHOSTOS_MONITOR_COOPERATIVE: return "COOP";
        case GHOSTOS_MONITOR_REALTIME: return "RT";
        default: return "????";
    }
}

static void append_thread_id(line_buffer *line, uint32_t id) {
    if (id <= 63) append_hex4(line, id);
    else append_text(line, "????");
}

static void append_address_space(line_buffer *line, uint32_t id) {
    if (id <= 15) append_hex2(line, id);
    else append_text(line, "0x??");
}

static void append_resource_id(line_buffer *line, uint64_t id) {
    if (id <= 223) append_hex4(line, (uint32_t)id);
    else append_text(line, "????");
}

void ghostos_monitor_init(ghostos_monitor_state *monitor) {
    if (monitor) *monitor = (ghostos_monitor_state){.view = GHOSTOS_MONITOR_PROCESSES};
}

void ghostos_monitor_switch_view(ghostos_monitor_state *monitor) {
    if (!monitor) return;
    monitor->view = (ghostos_monitor_view)((monitor->view + 1u) % 4u);
}

ghostos_monitor_view ghostos_monitor_current_view(const ghostos_monitor_state *monitor) {
    return monitor && monitor->view <= GHOSTOS_MONITOR_MEMORY ? monitor->view : GHOSTOS_MONITOR_PROCESSES;
}

void ghostos_monitor_update(ghostos_monitor_state *monitor) {
    if (monitor) ++monitor->refresh_counter;
}

static const ghostos_monitor_thread *thread_for_slot(const ghostos_monitor_thread *threads,
    size_t count, size_t slot) {
    if (!threads) return NULL;
    if (count > GHOSTOS_MONITOR_MAX_THREADS) count = GHOSTOS_MONITOR_MAX_THREADS;
    for (size_t i = 0; i < count; ++i)
        if (threads[i].slot == slot && threads[i].thread_id == (UINT32_C(0x10000) | (uint32_t)slot))
            return &threads[i];
    return NULL;
}

size_t ghostos_monitor_get_processes(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_process_info result[GHOSTOS_MONITOR_PROCESS_ROWS]) {
    if (!result) return 0;
    for (size_t i = 0; i < GHOSTOS_MONITOR_PROCESS_ROWS; ++i) result[i] = (ghostos_monitor_process_info){0};
    size_t count = 0;
    for (size_t slot = 0; slot < GHOSTOS_MONITOR_MAX_THREADS && count < GHOSTOS_MONITOR_PROCESS_ROWS; ++slot) {
        const ghostos_monitor_thread *thread = thread_for_slot(threads, thread_count, slot);
        if (thread && thread->state != GHOSTOS_MONITOR_VACANT) {
            result[count++] = (ghostos_monitor_process_info){thread->thread_id, thread->state,
                thread->policy, thread->switches, thread->address_space};
        }
    }
    return count;
}

typedef struct { const ghostos_monitor_thread *thread; uint64_t activity; } cpu_sample;

size_t ghostos_monitor_get_top_cpu(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_cpu_history_entry history[GHOSTOS_MONITOR_MAX_THREADS],
    ghostos_monitor_cpu_usage result[GHOSTOS_MONITOR_TOP_CPU_ROWS]) {
    if (!history || !result) return 0;
    cpu_sample samples[GHOSTOS_MONITOR_MAX_THREADS] = {{0}};
    size_t active = 0;
    for (size_t slot = 0; slot < GHOSTOS_MONITOR_MAX_THREADS; ++slot) {
        const ghostos_monitor_thread *thread = thread_for_slot(threads, thread_count, slot);
        if (!thread || thread->state == GHOSTOS_MONITOR_VACANT) continue;
        ghostos_monitor_cpu_history_entry *entry = &history[slot];
        uint64_t delta = thread->switches >= entry->prev_switches ? thread->switches - entry->prev_switches : 0;
        entry->prev_switches = thread->switches;
        entry->samples[entry->sample_idx % 10] = (uint8_t)(delta > 100 ? 100 : delta);
        entry->sample_idx = (entry->sample_idx + 1) % 10;
        uint64_t sum = 0;
        for (size_t i = 0; i < 10; ++i) sum += entry->samples[i];
        samples[active++] = (cpu_sample){thread, sum / 10};
    }
    for (size_t i = 0; i < active; ++i)
        for (size_t j = i + 1; j < active; ++j)
            if (samples[j].activity > samples[i].activity) {
                cpu_sample tmp = samples[i]; samples[i] = samples[j]; samples[j] = tmp;
            }
    uint64_t total = 0;
    for (size_t i = 0; i < active; ++i) total += samples[i].activity;
    size_t count = active < GHOSTOS_MONITOR_TOP_CPU_ROWS ? active : GHOSTOS_MONITOR_TOP_CPU_ROWS;
    for (size_t i = 0; i < GHOSTOS_MONITOR_TOP_CPU_ROWS; ++i) result[i] = (ghostos_monitor_cpu_usage){0};
    for (size_t i = 0; i < count; ++i) {
        const ghostos_monitor_thread *thread = samples[i].thread;
        uint8_t util = total ? (uint8_t)((samples[i].activity * 100) / total) : 0;
        result[i] = (ghostos_monitor_cpu_usage){thread->thread_id, thread->switches,
            thread->owner, thread->address_space, thread->state, thread->policy, util};
    }
    return count;
}

void ghostos_monitor_get_lock_contentions(const ghostos_monitor_lock *locks, size_t lock_count,
    ghostos_monitor_lock_contention result[GHOSTOS_MONITOR_LOCK_ROWS]) {
    if (!result) return;
    for (size_t i = 0; i < GHOSTOS_MONITOR_LOCK_ROWS; ++i) result[i] = (ghostos_monitor_lock_contention){0};
    if (!locks) return;
    if (lock_count > GHOSTOS_MONITOR_LOCK_CAPACITY) lock_count = GHOSTOS_MONITOR_LOCK_CAPACITY;
    for (size_t i = 0; i < lock_count; ++i) {
        if (locks[i].occupied) result[i % GHOSTOS_MONITOR_LOCK_ROWS] =
            (ghostos_monitor_lock_contention){true, locks[i].resource_id,
                (uint8_t)locks[i].granted, (uint8_t)!locks[i].granted, locks[i].owner_node};
    }
}

void ghostos_monitor_get_kernel_lock_contentions(
    ghostos_monitor_lock_contention result[GHOSTOS_MONITOR_LOCK_ROWS]) {
    if (!result) return;
    for (size_t i = 0; i < GHOSTOS_MONITOR_LOCK_ROWS; ++i)
        result[i] = (ghostos_monitor_lock_contention){0};
    for (size_t index = 0; index < GHOSTOS_MONITOR_LOCK_CAPACITY; ++index) {
        uint64_t resource = 0;
        uint32_t owner_node = 0, address_space = 0, mode = 0;
        bool granted = false;
        if (ghostos_dlm_kernel_lock_summary(index, &resource, &owner_node, &address_space,
                &mode, &granted, NULL, NULL)) {
            (void)address_space;
            (void)mode;
            result[index % GHOSTOS_MONITOR_LOCK_ROWS] =
                (ghostos_monitor_lock_contention){true, resource, (uint8_t)granted,
                    (uint8_t)!granted, owner_node};
        }
    }
}

void ghostos_monitor_get_dsm_stats(ghostos_monitor_dsm_page_stats result[GHOSTOS_MONITOR_LOCK_ROWS]) {
    if (!result) return;
    for (size_t i = 0; i < GHOSTOS_MONITOR_LOCK_ROWS; ++i) result[i] = (ghostos_monitor_dsm_page_stats){0};
}

void ghostos_monitor_render_processes(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_write_fn write, void *context) {
    emit_text("\x1b[1;36m=== PROCESSES ===\x1b[0m\n", write, context);
    emit_text("THREAD       STATE      SWITCHES SPACE    POLICY    \n", write, context);
    ghostos_monitor_process_info rows[GHOSTOS_MONITOR_PROCESS_ROWS];
    size_t count = ghostos_monitor_get_processes(threads, thread_count, rows);
    for (size_t i = 0; i < count; ++i) {
        line_buffer line = {{0}, 0};
        append_thread_id(&line, rows[i].thread_id); append_padded(&line, "", 8); append_char(&line, ' ');
        append_padded(&line, state_name(rows[i].state), 10); append_char(&line, ' ');
        append_u64_field(&line, rows[i].switches, 8); append_char(&line, ' ');
        append_address_space(&line, rows[i].address_space); append_padded(&line, "", 4); append_char(&line, ' ');
        append_padded(&line, policy_name(rows[i].policy), 10); append_char(&line, '\n');
        emit(&line, write, context);
    }
}

void ghostos_monitor_render_top_cpu(const ghostos_monitor_thread *threads, size_t thread_count,
    ghostos_monitor_cpu_history_entry history[GHOSTOS_MONITOR_MAX_THREADS],
    ghostos_monitor_write_fn write, void *context) {
    emit_text("\x1b[1;32m=== TOP CPU ===\x1b[0m\n", write, context);
    emit_text("THREAD       SWITCHES   UTIL%     \n", write, context);
    ghostos_monitor_cpu_usage rows[GHOSTOS_MONITOR_TOP_CPU_ROWS];
    size_t count = ghostos_monitor_get_top_cpu(threads, thread_count, history, rows);
    for (size_t i = 0; i < count; ++i) {
        line_buffer line = {{0}, 0};
        append_thread_id(&line, rows[i].thread_id); append_padded(&line, "", 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].switches, 10); append_char(&line, ' ');
        append_u64_field(&line, rows[i].util_percent, 3); append_text(&line, "%\n");
        emit(&line, write, context);
    }
}

void ghostos_monitor_render_dsm(const ghostos_monitor_lock *locks, size_t lock_count,
    ghostos_monitor_write_fn write, void *context) {
    emit_text("\x1b[1;33m=== DISTRIBUTED SHARED MEMORY (DSM) LOCKS ===\x1b[0m\n", write, context);
    emit_text("RESOURCE     GRANTED  QUEUED   OWNER     \n", write, context);
    ghostos_monitor_lock_contention rows[GHOSTOS_MONITOR_LOCK_ROWS];
    ghostos_monitor_get_lock_contentions(locks, lock_count, rows);
    for (size_t i = 0; i < GHOSTOS_MONITOR_LOCK_ROWS; ++i) {
        line_buffer line = {{0}, 0};
        if (rows[i].has_resource) append_resource_id(&line, rows[i].resource_id);
        else append_text(&line, "----");
        append_padded(&line, "", 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].granted, 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].queued, 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].owner_node, 10);
        append_char(&line, '\n'); emit(&line, write, context);
    }
}

void ghostos_monitor_render_kernel_dsm(ghostos_monitor_write_fn write, void *context) {
    emit_text("\x1b[1;33m=== DISTRIBUTED SHARED MEMORY (DSM) LOCKS ===\x1b[0m\n", write, context);
    emit_text("RESOURCE     GRANTED  QUEUED   OWNER     \n", write, context);
    ghostos_monitor_lock_contention rows[GHOSTOS_MONITOR_LOCK_ROWS];
    ghostos_monitor_get_kernel_lock_contentions(rows);
    for (size_t i = 0; i < GHOSTOS_MONITOR_LOCK_ROWS; ++i) {
        line_buffer line = {{0}, 0};
        if (rows[i].has_resource) append_resource_id(&line, rows[i].resource_id);
        else append_text(&line, "----");
        append_padded(&line, "", 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].granted, 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].queued, 8); append_char(&line, ' ');
        append_u64_field(&line, rows[i].owner_node, 10); append_char(&line, '\n');
        emit(&line, write, context);
    }
}

void ghostos_monitor_render_memory(uint64_t available_bytes, uint64_t used_bytes,
    ghostos_monitor_write_fn write, void *context) {
    emit_text("\x1b[1;34m=== MEMORY ===\x1b[0m\nAvailable: ", write, context);
    line_buffer line = {{0}, 0}; append_u64(&line, available_bytes);
    append_text(&line, " bytes\nUsed:      "); append_u64(&line, used_bytes);
    append_text(&line, " bytes\n"); emit(&line, write, context);
}
