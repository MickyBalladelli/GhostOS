#include "ghostos/config_network.h"
#include <assert.h>
#include <string.h>
static size_t text_length(const char *text) {
    size_t length = 0;
    while (text[length]) ++length;
    return length;
}
static void network_keeps_the_interface_and_rejects_a_missing_route_target(void) {
    const char *source =
        "[network]\n"
        "hostname = \"ghostos\"\n"
        "\n"
        "[[network.interface]]\n"
        "name = \"eth0\"\n"
        "address = \"10.0.0.2\"\n"
        "mtu = 1500\n"
        "\n"
        "[[network.route]]\n"
        "destination = \"0.0.0.0/0\"\n"
        "gateway = \"10.0.0.1\"\n"
        "interface = \"eth0\"\n";
    const char *missing =
        "[[network.interface]]\nname = \"eth0\"\naddress = \"10.0.0.2\"\n"
        "[[network.route]]\ndestination = \"0.0.0.0/0\"\ngateway = \"10.0.0.1\"\ninterface = \"eth1\"\n";
    uint8_t hostname[16];
    ghostos_config_interface interfaces[1];
    ghostos_config_route routes[1];
    size_t hostname_length = 0, interface_count = 0, route_count = 0;
    assert(!ghostos_config_parse_network((const uint8_t *)source, text_length(source), hostname, sizeof hostname,
        &hostname_length, interfaces, 1, &interface_count, routes, 1, &route_count));
    assert(hostname_length == 7 && !memcmp(hostname, "ghostos", 7));
    assert(interface_count == 1 && interfaces[0].name_length == 4 && !memcmp(interfaces[0].name, "eth0", 4));
    assert(interfaces[0].mtu == 1500 && interfaces[0].enabled && interfaces[0].mode == 0);
    assert(route_count == 1 && routes[0].metric == 100 && routes[0].interface_length == 4);
    assert(ghostos_config_parse_network((const uint8_t *)missing, text_length(missing), hostname, sizeof hostname,
        &hostname_length, interfaces, 1, &interface_count, routes, 1, &route_count) == 2);
}
int main(void) {
    network_keeps_the_interface_and_rejects_a_missing_route_target();
    return 0;
}
