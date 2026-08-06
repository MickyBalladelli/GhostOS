use syn_shell::{
    network::{
        interface_update_request, register_network_commands, SET_HOSTNAME_ROUTE,
        SET_INTERFACE_ROUTE, SET_ROUTE_ROUTE, SHOW_INTERFACES_ROUTE, SHOW_NETWORK_ROUTE,
        SHOW_ROUTES_ROUTE,
    },
    parser::CommandRegistry,
};

fn registry() -> CommandRegistry<8> {
    let mut registry = CommandRegistry::new();
    register_network_commands(&mut registry).expect("register network commands");
    registry
}

#[test]
fn network_commands_use_single_noun_names() {
    let registry = registry();
    for (input, route) in [
        ("SHOW NETWORK", SHOW_NETWORK_ROUTE),
        ("SHOW INTERFACES", SHOW_INTERFACES_ROUTE),
        ("SHOW ROUTES", SHOW_ROUTES_ROUTE),
        ("SET HOSTNAME synos", SET_HOSTNAME_ROUTE),
        (
            "SET INTERFACE eth0 /ADDRESS=10.0.0.2 /GATEWAY=10.0.0.1 /MTU=1500 /ENABLE",
            SET_INTERFACE_ROUTE,
        ),
        (
            "SET ROUTE 0.0.0.0/0 /GATEWAY=10.0.0.1 /INTERFACE=eth0 /METRIC=100",
            SET_ROUTE_ROUTE,
        ),
    ] {
        assert_eq!(registry.parse(input).unwrap().stage(0).unwrap().route.raw(), route);
    }
}

#[test]
fn interface_updates_reject_conflicting_or_empty_changes() {
    let registry = registry();
    for input in [
        "SET INTERFACE eth0 /ENABLE /DISABLE",
        "SET INTERFACE eth0",
        "SET INTERFACE eth0 /MTU=500",
    ] {
        let call = registry.parse(input).unwrap().stage(0).unwrap();
        assert_eq!(call.route.raw(), SET_INTERFACE_ROUTE);
        assert!(interface_update_request(&call).is_err());
    }
}
