use syn_shell::{
    network::{
        command_help, interface_update_request, register_network_commands, route_update_request,
        DhcpLeaseView, InterfaceAddressMode, InterfaceUpdate, NetworkExecutor, NetworkInterfaceView,
        NetworkRouteView, NetworkSource, NetworkText, NetworkView, RouteUpdate, SET_HOSTNAME_ROUTE,
        SET_INTERFACE_ROUTE, SET_ROUTE_ROUTE, SHOW_INTERFACES_ROUTE, SHOW_NETWORK_ROUTE,
        SHOW_ROUTES_ROUTE, MAX_NETWORK_OUTPUT_ROWS,
    },
    interpreter::CommandExecutor,
    parser::CommandRegistry,
    Text,
};
use synos_status::Status;
use synos_system_model::command::OutputValue;

fn registry() -> CommandRegistry<8> {
    let mut registry = CommandRegistry::new();
    register_network_commands(&mut registry).expect("register network commands");
    registry
}

fn text(value: &str) -> NetworkText {
    Text::new(value).expect("network text fits")
}

fn has_text(
    output: &synos_system_model::command::StructuredOutput,
    name: &str,
    expected: &str,
) -> bool {
    output.fields().any(|field| {
        field.name.as_str().eq_ignore_ascii_case(name)
            && matches!(
                field.value,
                OutputValue::Text(value) if value.as_str() == expected
            )
    })
}

fn has_unsigned(
    output: &synos_system_model::command::StructuredOutput,
    name: &str,
    expected: u64,
) -> bool {
    output.fields().any(|field| {
        field.name.as_str().eq_ignore_ascii_case(name)
            && matches!(field.value, OutputValue::Unsigned(value) if value == expected)
    })
}

fn has_bool(
    output: &synos_system_model::command::StructuredOutput,
    name: &str,
    expected: bool,
) -> bool {
    output.fields().any(|field| {
        field.name.as_str().eq_ignore_ascii_case(name)
            && matches!(field.value, OutputValue::Boolean(value) if value == expected)
    })
}

fn lacks_field(output: &synos_system_model::command::StructuredOutput, name: &str) -> bool {
    !output
        .fields()
        .any(|field| field.name.as_str().eq_ignore_ascii_case(name))
}

fn execute(
    executor: &mut NetworkExecutor<FakeNetwork, 8>,
    input: &str,
) -> Result<synos_system_model::command::StructuredOutput, Status> {
    let call = registry().parse(input).unwrap().stage(0).unwrap();
    let token = executor.submit(call, None).unwrap();
    executor.poll(token).unwrap()
}

#[test]
fn interfaces_alias_parses() {
    let registry = registry();
    for input in ["INTERFACES", "INTERFACE", "SHOW INTERFACE", "SHOW INTERFACES"] {
        assert_eq!(
            registry.parse(input).unwrap().stage(0).unwrap().route.raw(),
            SHOW_INTERFACES_ROUTE,
            "{input}"
        );
    }
    let suggestions = registry.suggestions("show int").unwrap();
    assert!(suggestions.commands().any(|name| name.as_str() == "SHOW-INTERFACES"));
}

#[test]
fn network_commands_use_single_noun_names() {

    let registry = registry();
    for (input, route) in [
        ("SHOW NETWORK", SHOW_NETWORK_ROUTE),
        ("SHOW INTERFACES", SHOW_INTERFACES_ROUTE),
        ("SHOW INTERFACE", SHOW_INTERFACES_ROUTE),
        ("SHOW INTERFACE eth0", SHOW_INTERFACES_ROUTE),
        ("SHOW ROUTES", SHOW_ROUTES_ROUTE),
        ("SET HOSTNAME synos", SET_HOSTNAME_ROUTE),
        (
            "SET INTERFACE eth0 /ADDRESS=10.0.0.2 /GATEWAY=10.0.0.1 /MTU=1500 /ENABLE",
            SET_INTERFACE_ROUTE,
        ),
        ("SET INTERFACE eth0 /DHCP", SET_INTERFACE_ROUTE),
        ("SET INTERFACE eth0 /STATIC /ADDRESS=10.0.0.2", SET_INTERFACE_ROUTE),
        ("SET INTERFACE eth0 /DISABLE", SET_INTERFACE_ROUTE),
        (
            "SET ROUTE 0.0.0.0/0 /GATEWAY=10.0.0.1 /INTERFACE=eth0 /METRIC=100",
            SET_ROUTE_ROUTE,
        ),
    ] {
        assert_eq!(
            registry.parse(input).unwrap().stage(0).unwrap().route.raw(),
            route
        );
    }
}

#[test]
fn interface_updates_reject_conflicting_or_empty_changes() {
    let registry = registry();
    for input in [
        "SET INTERFACE eth0 /ENABLE /DISABLE",
        "SET INTERFACE eth0",
        "SET INTERFACE eth0 /MTU=500",
        "SET INTERFACE eth0 /MTU=70000",
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
    let dhcp = registry
        .parse("SET INTERFACE eth0 /DHCP")
        .unwrap()
        .stage(0)
        .unwrap();
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

    let implied_static = registry
        .parse("SET INTERFACE eth0 /ADDRESS=10.0.0.8")
        .unwrap()
        .stage(0)
        .unwrap();
    let update = interface_update_request(&implied_static).unwrap();
    assert_eq!(update.mode, Some(InterfaceAddressMode::Static));
    assert_eq!(update.address, Some("10.0.0.8"));
}

#[test]
fn route_updates_require_destination_gateway_and_interface() {
    let registry = registry();
    let ok = registry
        .parse("SET ROUTE 10.0.0.0/24 /GATEWAY=10.0.0.1 /INTERFACE=eth0")
        .unwrap()
        .stage(0)
        .unwrap();
    let update = route_update_request(&ok).unwrap();
    assert_eq!(update.destination, "10.0.0.0/24");
    assert_eq!(update.gateway, "10.0.0.1");
    assert_eq!(update.interface, "eth0");
    assert_eq!(update.metric, None);

    for input in [
        "SET ROUTE 10.0.0.0/24 /GATEWAY=10.0.0.1",
        "SET ROUTE 10.0.0.0/24 /INTERFACE=eth0",
        "SET ROUTE /GATEWAY=10.0.0.1 /INTERFACE=eth0",
    ] {
        assert!(registry.parse(input).is_err() || {
            let call = registry.parse(input).unwrap().stage(0).unwrap();
            route_update_request(&call).is_err()
        });
    }
}

#[test]
fn network_help_covers_aliases_and_dhcp_qualifiers() {
    assert_eq!(command_help("NETWORK").unwrap().name, "SHOW-NETWORK");
    assert_eq!(command_help("INTERFACES").unwrap().name, "SHOW-INTERFACES");
    assert_eq!(command_help("INTERFACE").unwrap().name, "SET-INTERFACE");
    assert_eq!(command_help("ROUTES").unwrap().name, "SHOW-ROUTES");
    assert_eq!(command_help("HOSTNAME").unwrap().name, "SET-HOSTNAME");
    assert_eq!(command_help("INTERFACE").unwrap().name, "SET-INTERFACE");
    assert_eq!(command_help("ROUTE").unwrap().name, "SET-ROUTE");
    let interface = command_help("SET-INTERFACE").unwrap();
    assert!(interface.qualifiers.contains("/DHCP"));
    assert!(interface.qualifiers.contains("/STATIC"));
    assert!(command_help("SHOW-INTERFACES")
        .unwrap()
        .description
        .contains("DHCP"));
}

#[test]
fn mutations_require_network_administration_capability() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::denied());
    for input in [
        "SET HOSTNAME node-1",
        "SET INTERFACE eth0 /DHCP",
        "SET INTERFACE eth0 /ADDRESS=10.0.0.3",
        "SET ROUTE 0.0.0.0/0 /GATEWAY=10.0.0.1 /INTERFACE=eth0",
    ] {
        assert!(matches!(
            execute(&mut executor, input),
            Err(Status::ACCESS_DENIED)
        ));
    }
}

#[test]
fn show_and_set_commands_emit_structured_network_output() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());

    let network = execute(&mut executor, "SHOW NETWORK").unwrap();
    assert!(has_text(&network, "operation", "show-network"));
    assert!(has_unsigned(&network, "generation", 1));
    assert!(has_text(&network, "hostname", "synos"));
    assert!(has_unsigned(&network, "interface-count", 1));
    assert!(has_unsigned(&network, "route-count", 1));
    assert!(has_text(&network, "interface1-name", "eth0"));
    assert!(has_text(&network, "interface1-address", "10.0.0.2"));
    assert!(has_text(&network, "route1-destination", "0.0.0.0/0"));
    let rendered = syn_shell::render::render(&network, syn_shell::render::OutputFormat::List)
        .unwrap();
    assert!(rendered.as_str().contains("Network: synos"));
    assert!(rendered.as_str().contains("Address: 10.0.0.2"));

    let interfaces = execute(&mut executor, "SHOW INTERFACES").unwrap();
    assert!(has_text(&interfaces, "operation", "show-interfaces"));
    assert!(has_text(&interfaces, "interface1-name", "eth0"));
    assert!(has_text(&interfaces, "interface1-address", "10.0.0.2"));
    assert!(has_text(&interfaces, "interface1-gateway", "10.0.0.1"));
    assert!(has_unsigned(&interfaces, "interface1-mtu", 1500));
    assert!(has_bool(&interfaces, "interface1-enabled", true));
    assert!(has_bool(&interfaces, "interface1-link-up", true));
    assert!(has_text(&interfaces, "interface1-mode", "static"));

    let routes = execute(&mut executor, "SHOW ROUTES").unwrap();
    assert!(has_text(&routes, "operation", "show-routes"));
    assert!(has_text(&routes, "route1-destination", "0.0.0.0/0"));
    assert!(has_text(&routes, "route1-gateway", "10.0.0.1"));
    assert!(has_text(&routes, "route1-interface", "eth0"));
    assert!(has_unsigned(&routes, "route1-metric", 100));

    let hostname = execute(&mut executor, "SET HOSTNAME node-1").unwrap();
    assert!(has_text(&hostname, "hostname", "node-1"));
    assert!(has_unsigned(&hostname, "generation", 2));
}

#[test]
fn show_interface_selects_one_named_interface() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());
    let interface = execute(&mut executor, "SHOW INTERFACE eth0").unwrap();
    assert!(has_text(&interface, "operation", "show-interface"));
    assert!(has_text(&interface, "interface1-name", "eth0"));
    assert!(has_text(&interface, "interface1-address", "10.0.0.2"));
    assert_eq!(
        interface
            .fields()
            .filter(|field| field.name.as_str().starts_with("interface2-"))
            .count(),
        0
    );
}

#[test]
fn set_interface_dhcp_exposes_lease_details_on_show_interfaces() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());
    let set = execute(&mut executor, "SET INTERFACE eth0 /DHCP").unwrap();
    assert!(has_unsigned(&set, "generation", 2));

    let interfaces = execute(&mut executor, "SHOW INTERFACES").unwrap();
    assert!(has_text(&interfaces, "interface1-mode", "dhcp"));
    assert!(has_text(&interfaces, "interface1-dhcp-state", "bound"));
    assert!(has_text(&interfaces, "interface1-dhcp-server", "10.0.0.1"));
    assert!(has_unsigned(&interfaces, "interface1-dhcp-expires-ms", 60_000));
    assert!(has_text(&interfaces, "interface1-dns0", "10.0.0.53"));
    assert!(has_text(&interfaces, "interface1-address", "10.0.0.50"));
}

#[test]
fn set_interface_static_and_disable_update_view() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());
    execute(&mut executor, "SET INTERFACE eth0 /DHCP").unwrap();
    let restored = execute(
        &mut executor,
        "SET INTERFACE eth0 /STATIC /ADDRESS=10.0.0.9 /GATEWAY=10.0.0.1 /ENABLE",
    )
    .unwrap();
    assert!(has_unsigned(&restored, "generation", 3));

    let interfaces = execute(&mut executor, "SHOW INTERFACES").unwrap();
    assert!(has_text(&interfaces, "interface1-mode", "static"));
    assert!(has_text(&interfaces, "interface1-address", "10.0.0.9"));
    assert!(lacks_field(&interfaces, "interface1-dhcp-state"));

    execute(&mut executor, "SET INTERFACE eth0 /DISABLE").unwrap();
    let interfaces = execute(&mut executor, "SHOW INTERFACES").unwrap();
    assert!(has_bool(&interfaces, "interface1-enabled", false));
}

#[test]
fn set_route_upserts_and_bumps_generation() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());
    let output = execute(
        &mut executor,
        "SET ROUTE 10.0.0.0/24 /GATEWAY=10.0.0.1 /INTERFACE=eth0 /METRIC=50",
    )
    .unwrap();
    assert!(has_unsigned(&output, "generation", 2));
    assert!(has_unsigned(&output, "route-count", 2));

    let routes = execute(&mut executor, "SHOW ROUTES").unwrap();
    assert!(has_text(&routes, "route1-destination", "0.0.0.0/0"));
    assert!(has_text(&routes, "route2-destination", "10.0.0.0/24"));
    assert!(has_unsigned(&routes, "route2-metric", 50));

    execute(
        &mut executor,
        "SET ROUTE 10.0.0.0/24 /GATEWAY=10.0.0.254 /INTERFACE=eth0 /METRIC=25",
    )
    .unwrap();
    let routes = execute(&mut executor, "SHOW ROUTES").unwrap();
    assert!(has_unsigned(&routes, "route-count", 2));
    assert!(has_text(&routes, "route2-gateway", "10.0.0.254"));
    assert!(has_unsigned(&routes, "route2-metric", 25));
}


#[test]
fn two_seeded_interfaces_fit_output_budget() {
    let mut interfaces = [None; MAX_NETWORK_OUTPUT_ROWS];
    interfaces[0] = Some(NetworkInterfaceView {
        name: text("lo"),
        address: text("127.0.0.1"),
        gateway: None,
        mtu: 65_535,
        enabled: true,
        link_up: true,
        mode: InterfaceAddressMode::Static,
        dhcp: None,
    });
    interfaces[1] = Some(NetworkInterfaceView {
        name: text("eth0"),
        address: text("0.0.0.0"),
        gateway: None,
        mtu: 1500,
        enabled: true,
        link_up: false,
        mode: InterfaceAddressMode::Static,
        dhcp: None,
    });
    let source = FakeNetwork {
        allowed: true,
        view: NetworkView {
            generation: 1,
            hostname: Some(text("synos")),
            interface_count: 2,
            route_count: 0,
            interfaces,
            routes: [None; MAX_NETWORK_OUTPUT_ROWS],
            next_interface: None,
            next_route: None,
        },
    };
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(source);
    let interfaces = execute(&mut executor, "SHOW INTERFACES").expect("show interfaces");
    assert!(has_text(&interfaces, "interface1-name", "lo"));
    assert!(has_text(&interfaces, "interface2-name", "eth0"));
    assert!(syn_shell::render::render(&interfaces, syn_shell::render::OutputFormat::List).is_ok());
}

#[test]
fn four_full_interfaces_paginate_within_output_budget() {
    let mut interfaces = [None; MAX_NETWORK_OUTPUT_ROWS];
    for i in 0..4 {
        let name = match i {
            0 => "eth0",
            1 => "eth1",
            2 => "eth2",
            _ => "eth3",
        };
        interfaces[i] = Some(NetworkInterfaceView {
            name: text(name),
            address: text("10.0.0.2"),
            gateway: Some(text("10.0.0.1")),
            mtu: 1500,
            enabled: true,
            link_up: true,
            mode: InterfaceAddressMode::Dhcp,
            dhcp: Some(DhcpLeaseView {
                state: text("bound"),
                server: Some(text("10.0.0.1")),
                expires_at_ms: Some(60_000),
                dns0: Some(text("10.0.0.53")),
                dns1: Some(text("10.0.0.54")),
            }),
        });
    }
    let source = FakeNetwork {
        allowed: true,
        view: NetworkView {
            generation: 1,
            hostname: Some(text("synos")),
            interface_count: 4,
            route_count: 0,
            interfaces,
            routes: [None; MAX_NETWORK_OUTPUT_ROWS],
            next_interface: None,
            next_route: None,
        },
    };
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(source);
    let output = execute(&mut executor, "SHOW INTERFACES").expect("show interfaces");
    assert!(has_text(&output, "interface1-name", "eth0"));
    assert!(has_text(&output, "interface2-name", "eth1"));
    assert!(lacks_field(&output, "interface3-name"));
    assert!(has_unsigned(&output, "next-interface", 2));
}

#[test]
fn show_interfaces_supports_bounded_pagination_marker() {
    let mut source = FakeNetwork::seeded();
    source.view.next_interface = Some(4);
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(source);
    let interfaces = execute(&mut executor, "SHOW INTERFACES").unwrap();
    assert!(has_unsigned(&interfaces, "next-interface", 4));
}


struct FakeNetwork {
    allowed: bool,
    view: NetworkView,
}

impl FakeNetwork {
    fn denied() -> Self {
        Self {
            allowed: false,
            view: NetworkView::EMPTY,
        }
    }

    fn seeded() -> Self {
        let mut interfaces = [None; MAX_NETWORK_OUTPUT_ROWS];
        interfaces[0] = Some(NetworkInterfaceView {
            name: text("eth0"),
            address: text("10.0.0.2"),
            gateway: Some(text("10.0.0.1")),
            mtu: 1500,
            enabled: true,
            link_up: true,
            mode: InterfaceAddressMode::Static,
            dhcp: None,
        });
        let mut routes = [None; MAX_NETWORK_OUTPUT_ROWS];
        routes[0] = Some(NetworkRouteView {
            destination: text("0.0.0.0/0"),
            gateway: text("10.0.0.1"),
            interface: text("eth0"),
            metric: 100,
        });
        Self {
            allowed: true,
            view: NetworkView {
                generation: 1,
                hostname: Some(text("synos")),
                interface_count: 1,
                route_count: 1,
                interfaces,
                routes,
                next_interface: None,
                next_route: None,
            },
        }
    }

    fn bump(&mut self) {
        self.view.generation = self.view.generation.saturating_add(1);
    }

    fn interface_mut(&mut self) -> &mut NetworkInterfaceView {
        self.view.interfaces[0].as_mut().expect("seeded interface")
    }
}

impl NetworkSource for FakeNetwork {
    fn authorize_mutation(&mut self) -> Result<(), Status> {
        if self.allowed {
            Ok(())
        } else {
            Err(Status::ACCESS_DENIED)
        }
    }

    fn show_network(&mut self) -> Result<NetworkView, Status> {
        Ok(self.view)
    }

    fn show_interfaces(&mut self) -> Result<NetworkView, Status> {
        Ok(self.view)
    }

    fn show_routes(&mut self) -> Result<NetworkView, Status> {
        Ok(self.view)
    }

    fn set_hostname(&mut self, hostname: &str) -> Result<NetworkView, Status> {
        self.view.hostname = Some(text(hostname));
        self.bump();
        Ok(self.view)
    }

    fn set_interface(&mut self, update: InterfaceUpdate<'_>) -> Result<NetworkView, Status> {
        let interface = self.interface_mut();
        if let Some(mode) = update.mode {
            interface.mode = mode;
            if mode == InterfaceAddressMode::Dhcp {
                interface.address = text("10.0.0.50");
                interface.gateway = Some(text("10.0.0.1"));
                interface.dhcp = Some(DhcpLeaseView {
                    state: text("bound"),
                    server: Some(text("10.0.0.1")),
                    expires_at_ms: Some(60_000),
                    dns0: Some(text("10.0.0.53")),
                    dns1: None,
                });
            } else {
                interface.dhcp = None;
            }
        }
        if let Some(address) = update.address {
            interface.address = text(address);
            if update.mode.is_none() {
                interface.mode = InterfaceAddressMode::Static;
                interface.dhcp = None;
            }
        }
        if let Some(gateway) = update.gateway {
            interface.gateway = Some(text(gateway));
        }
        if let Some(mtu) = update.mtu {
            interface.mtu = mtu;
        }
        if let Some(enabled) = update.enabled {
            interface.enabled = enabled;
        }
        self.bump();
        Ok(self.view)
    }

    fn set_route(&mut self, update: RouteUpdate<'_>) -> Result<NetworkView, Status> {
        if let Some(existing) = self
            .view
            .routes
            .iter_mut()
            .flatten()
            .find(|route| route.destination.as_str() == update.destination)
        {
            existing.gateway = text(update.gateway);
            existing.interface = text(update.interface);
            if let Some(metric) = update.metric {
                existing.metric = metric;
            }
        } else {
            let slot = self
                .view
                .routes
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(Status::NO_SPACE)?;
            *slot = Some(NetworkRouteView {
                destination: text(update.destination),
                gateway: text(update.gateway),
                interface: text(update.interface),
                metric: update.metric.unwrap_or(100),
            });
            self.view.route_count = self.view.route_count.saturating_add(1);
        }
        self.bump();
        Ok(self.view)
    }
}
