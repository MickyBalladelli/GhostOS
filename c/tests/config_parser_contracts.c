#include "ghostos/config_parser.h"
#include <assert.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static void system_section_rejects_schema_and_unknown_keys(void) {
    const char *valid = "[system]\nschema = 0x1\nrevision = 1\n";
    const char *schema = "[system]\nschema = 99\nrevision = 1\n";
    const char *unknown = "[system]\nschema = 1\nrevision = 1\nunknown = true\n";
    const char *duplicate = "schema = 1\nschema = 2\n";
    const char *missing = "[system]\nrevision = 1\n";
    const char *zero = "[system]\nschema = 1\nrevision = 0\n";
    uint16_t parsed_schema = 0;
    uint64_t revision = 0;
    assert(!ghostos_config_parse_system((const uint8_t *)valid, text_length(valid), &parsed_schema, &revision));
    assert(parsed_schema == 1 && revision == 1);
    assert(ghostos_config_parse_system((const uint8_t *)schema, text_length(schema), &parsed_schema, &revision) == 6);
    assert(ghostos_config_parse_system((const uint8_t *)unknown, text_length(unknown), &parsed_schema, &revision) == 3);
    assert(ghostos_config_parse_system((const uint8_t *)duplicate, text_length(duplicate), &parsed_schema, &revision) == 4);
    assert(ghostos_config_parse_system((const uint8_t *)missing, text_length(missing), &parsed_schema, &revision) == 7);
    assert(ghostos_config_parse_system((const uint8_t *)zero, text_length(zero), &parsed_schema, &revision) == 2);
}
int main(void) {
    system_section_rejects_schema_and_unknown_keys();
    return 0;
}
