use syn_shell::{
    network::{
        command_help, interface_update_request, register_network_commands, InterfaceAddressMode,
        NetworkExecutor, NetworkSource, NetworkView, SET_HOSTNAME_ROUTE, SET_INTERFACE_ROUTE,
        SET_ROUTE_ROUTE, SHOW_INTERFACES_ROUTE, SHOW_NETWORK_ROUTE, SHOW_ROUTES_ROUTE,
    },
    interpreter::CommandExecutor,
    parser::CommandRegistry,
};
use synos_status::Status;

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
            "SET INTERFACE eth0 /DHCP",
            SET_INTERFACE_ROUTE,
        ),
        (
            "SET INTERFACE eth0 /STATIC /ADDRESS=10.0.0.2",
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
        "SET INTERFACE eth0 /DHCP /STATIC",
        "SET INTERFACE eth0 /DHCP /ADDRESS=10.0.0.2",
    ] {
        let call = registry.parse(input).unwrap().stage(0).unwrap();
        assert_eq!(call.route.raw(), SET_INTERFACE_ROUTE);
        assert!(interface_update_request(&call).is_err());
    }
}

#[test]
fn interface_dhcp_and_static_modes_parse() {
    let registry = registry();
    let dhcp = registry.parse("SET INTERFACE eth0 /DHCP").unwrap().stage(0).unwrap();
    let update = interface_update_request(&dhcp).unwrap();
    assert_eq!(update.mode, Some(InterfaceAddressMode::Dhcp));
    assert!(update.address.is_none());

    let static_mode = registry
        .parse("SET INTERFACE eth0 /STATIC /ADDRESS=10.0.0.9")
        .unwrap()
        .stage(0)
        .unwrap();
    let update = interface_update_request(&static_mode).unwrap();
    assert_eq!(update.mode, Some(InterfaceAddressMode::Static));
    assert_eq!(update.address, Some("10.0.0.9"));
}

#[test]
fn network_help_and_mutation_capability_are_present() {
    let registry = registry();
    assert_eq!(command_help("NETWORK").unwrap().name, "SHOW-NETWORK");
    assert_eq!(command_help("INTERFACE").unwrap().name, "SET-INTERFACE");

    let call = registry.parse("SET HOSTNAME node-1").unwrap().stage(0).unwrap();
    let mut executor: NetworkExecutor<_, 4> = NetworkExecutor::new(Source { allowed: false });
    let token = executor.submit(call, None).unwrap();
    assert!(matches!(executor.poll(token), Some(Err(Status::ACCESS_DENIED))));
}

struct Source {
    allowed: bool,
}

impl NetworkSource for Source {
    fn authorize_mutation(&mut self) -> Result<(), Status> {
        if self.allowed {
            Ok(())
        } else {
            Err(Status::ACCESS_DENIED)
        }
    }

    fn set_hostname(&mut self, _hostname: &str) -> Result<NetworkView, Status> {
        Ok(NetworkView::EMPTY)
    }
}
