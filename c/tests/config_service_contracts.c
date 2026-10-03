#include "ghostos/config_parser.h"
#include <assert.h>
#include <string.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static void service_record_uses_defaults_and_rejects_a_zero_image(void) {
    const char *source =
        "[system]\n"
        "schema = 1\n"
        "revision = 1\n"
        "\n"
        "[[service]]\n"
        "name = \"init\"\n"
        "image = 0x44\n"
        "kind = \"system\"\n"
        "restart = \"on-failure\"\n";
    const char *zero_image =
        "[system]\nschema = 1\nrevision = 1\n"
        "[[service]]\nname = \"init\"\nimage = 0\nkind = \"system\"\n";
    ghostos_config_service services[2];
    uint16_t schema = 0;
    uint64_t revision = 0;
    size_t count = 0;
    assert(!ghostos_config_parse_services((const uint8_t *)source, text_length(source), &schema, &revision, services, 2, &count));
    assert(schema == 1 && revision == 1 && count == 1);
    assert(services[0].name_length == 4 && !memcmp(services[0].name, "init", 4));
    assert(services[0].image == 0x44 && services[0].kind == 0 && services[0].restart == 1 && services[0].enabled);
    assert(ghostos_config_parse_services((const uint8_t *)zero_image, text_length(zero_image), &schema, &revision, services, 2, &count) == 2);
}
int main(void) {
    service_record_uses_defaults_and_rejects_a_zero_image();
    return 0;
}
