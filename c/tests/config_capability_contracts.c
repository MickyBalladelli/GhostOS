#include "ghostos/config_capability.h"
#include <assert.h>
#include <string.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static void capability_keeps_read_and_execute_for_a_declared_service(void) {
    const char *source =
        "[[capability]]\n"
        "service = \"init\"\n"
        "resource = \"SYS$BOOT\"\n"
        "kind = \"file\"\n"
        "rights = [\"read\", \"execute\"]\n"
        "required = true\n";
    const uint8_t init[] = {'i', 'n', 'i', 't'};
    const uint8_t other[] = {'o', 't', 'h', 'e', 'r'};
    const uint8_t *names[] = {init};
    const uint8_t *missing[] = {other};
    const uint8_t lengths[] = {4};
    const uint8_t missing_lengths[] = {5};
    ghostos_config_capability capabilities[1];
    size_t count = 0;
    assert(!ghostos_config_parse_capabilities((const uint8_t *)source, text_length(source), names, lengths, 1,
        capabilities, 1, &count));
    assert(count == 1 && capabilities[0].kind == 1 && capabilities[0].rights == 5 && capabilities[0].required);
    assert(capabilities[0].service_length == 4 && !memcmp(capabilities[0].service, "init", 4));
    assert(capabilities[0].resource_length == 8 && !memcmp(capabilities[0].resource, "SYS$BOOT", 8));
    assert(ghostos_config_parse_capabilities((const uint8_t *)source, text_length(source), missing, missing_lengths, 1,
        capabilities, 1, &count) == 2);
}
int main(void) {
    capability_keeps_read_and_execute_for_a_declared_service();
    return 0;
}
