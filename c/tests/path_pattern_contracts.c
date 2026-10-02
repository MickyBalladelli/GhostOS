#include "ghostos/path_pattern.h"
#include "ghostos/test_property.h"

#include <assert.h>
#include <stdlib.h>
#include <string.h>

static ghostos_pattern_error parse(const char *pattern) {
    bool magic;
    return ghostos_pattern_parse((const uint8_t *)pattern, strlen(pattern), &magic);
}

static bool matches(const char *pattern, const char *candidate) {
    assert(parse(pattern) == GHOSTOS_PATTERN_OK);
    return ghostos_pattern_matches((const uint8_t *)pattern, strlen(pattern),
        (const uint8_t *)candidate, strlen(candidate));
}

static bool wildcard_property(size_t index, uint64_t seed,
    ghostos_test_entropy *entropy, void *context) {
    (void)index; (void)seed; (void)context;
    char *name;
    size_t length;
    if (ghostos_property_ascii(entropy, 32, &name, &length) != GHOSTOS_PROPERTY_OK) return false;
    bool result = matches("*", name) && !matches("*", "a/b");
    free(name);
    return result;
}

static bool unescape_property(size_t index, uint64_t seed,
    ghostos_test_entropy *entropy, void *context) {
    (void)index; (void)seed; (void)context;
    char *value;
    size_t length, written;
    if (ghostos_property_ascii(entropy, 32, &value, &length) != GHOSTOS_PROPERTY_OK) return false;
    uint8_t output[GHOSTOS_MAX_PATTERN_BYTES];
    bool result = ghostos_pattern_unescape((const uint8_t *)value, length,
        output, sizeof(output), &written) == GHOSTOS_PATTERN_OK &&
        written == length && memcmp(output, value, length) == 0;
    free(value);
    return result;
}

int main(void) {
    assert(matches("/data/*.txt", "/data/a.txt"));
    assert(!matches("/data/*.txt", "/data/x/a.txt"));
    assert(matches("/data/file?.txt", "/data/file1.txt"));
    assert(matches("/data/[a-c].txt", "/data/b.txt"));
    assert(matches("/data/[!a].txt", "/data/b.txt"));
    assert(matches("/data/\\*.txt", "/data/*.txt"));
    assert(!matches("/data/\\*.txt", "/data/a.txt"));
    assert(parse("/data/[abc") == GHOSTOS_PATTERN_UNTERMINATED_CLASS);
    assert(parse("/data/foo\\") == GHOSTOS_PATTERN_TRAILING_ESCAPE);
    assert(parse("/data/[z-a]") == GHOSTOS_PATTERN_INVALID_RANGE);
    assert(parse("/data/[]") == GHOSTOS_PATTERN_EMPTY_CLASS);
    assert(parse("/data/foo/") == GHOSTOS_PATTERN_INVALID_PATH);
    assert(matches("*", ".hidden"));
    assert(matches("?", "é"));
    assert(!matches("?", "éé"));
    assert(!matches("*", "nested/name"));
    uint8_t boundary[GHOSTOS_MAX_PATTERN_BYTES + 1];
    memset(boundary, 'a', sizeof(boundary));
    bool magic;
    assert(ghostos_pattern_parse(boundary, sizeof(boundary) - 1, &magic) == GHOSTOS_PATTERN_OK);
    assert(ghostos_pattern_parse(boundary, sizeof(boundary), &magic) == GHOSTOS_PATTERN_INVALID_PATH);
    ghostos_property_failure failure;
    ghostos_property_config config = ghostos_property_config_new(UINT64_C(0x593), 128);
    assert(ghostos_property_run_assert("path-pattern.wildcard-separator", config,
        wildcard_property, NULL, &failure) == GHOSTOS_PROPERTY_OK);
    ghostos_property_failure_dispose(&failure);
    assert(ghostos_property_run_assert("path-pattern.unescape", config,
        unescape_property, NULL, &failure) == GHOSTOS_PROPERTY_OK);
    ghostos_property_failure_dispose(&failure);
    return 0;
}
