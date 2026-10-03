#include "ghostos/backup.h"
#include <assert.h>
#include <string.h>
static int contains(const uint8_t *bytes, size_t length, const char *text) {
    size_t text_length = strlen(text);
    size_t i, j;
    if (text_length > length) return 0;
    for (i = 0; i + text_length <= length; ++i) {
        for (j = 0; j < text_length && bytes[i + j] == (uint8_t)text[j]; ++j) {}
        if (j == text_length) return 1;
    }
    return 0;
}
static void pinned_bytes_are_streamed_and_zero_budget_changes_nothing(void) {
    const uint8_t name[] = "/before";
    const uint8_t data[] = "old";
    ghostos_backup_file file = {name, sizeof name - 1, data, 3, 1, 0, 0};
    ghostos_backup_job job;
    uint8_t output[128];
    size_t written = 0;
    size_t total = 0;
    uint32_t files = 0;
    uint64_t bytes = 0;
    ghostos_backup_start(&job, 7, 1);
    assert(ghostos_backup_poll(&job, &file, 1, output, sizeof output, &written, 0) == 1);
    assert(job.bytes_streamed == 0 && job.state == 0);
    while (job.state != 4) {
        assert(!ghostos_backup_poll(&job, &file, 1, output + total, sizeof output - total, &written, 7));
        total += written;
    }
    assert(!ghostos_backup_finish(&job, &files, &bytes));
    assert(files == 1 && total >= 8 && !memcmp(output, "SYNBACK1", 8));
    assert(contains(output, total, "old") && !contains(output, total, "new"));
    assert(contains(output, total, "SYNBEND1"));
}
int main(void) {
    pinned_bytes_are_streamed_and_zero_budget_changes_nothing();
    return 0;
}
