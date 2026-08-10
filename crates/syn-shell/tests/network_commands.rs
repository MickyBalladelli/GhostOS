use syn_shell::{
    network::{
        command_help, interface_update_request, register_network_commands, route_update_request,
        DhcpLeaseView, InterfaceAddressMode, InterfaceUpdate, NetworkExecutor, NetworkInterfaceView,
        NeighborEntryView, NeighborIpVersion, NeighborState, NeighborView, NetworkRouteView,
        NetworkSource, NetworkText, NetworkView, PingIpVersion, PingRequest,
        PingHandle, PingReply, PingResult, PingSummary, PingTarget, ResolvedPingRequest, RouteUpdate,
        SET_HOSTNAME_ROUTE, SET_INTERFACE_ROUTE, SET_ROUTE_ROUTE, SHOW_INTERFACES_ROUTE,
        SHOW_NETWORK_ROUTE, SHOW_ROUTES_ROUTE, PING_ROUTE, SHOW_NEIGHBORS_ROUTE,
        CLEAR_NEIGHBORS_ROUTE, MAX_NETWORK_LINK_EVENTS, MAX_NETWORK_OUTPUT_ROWS,
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
        ("SHOW NEIGHBORS", SHOW_NEIGHBORS_ROUTE),
        ("CLEAR NEIGHBORS /CONFIRM", CLEAR_NEIGHBORS_ROUTE),
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
        (
            "PING 198.51.100.4 /COUNT=3 /TIMEOUT=2500 /SIZE=64 /INTERFACE=eth0 /SOURCE=10.0.0.2 /IPV4",
            PING_ROUTE,
        ),
    ] {
        assert_eq!(
            registry.parse(input).unwrap().stage(0).unwrap().route.raw(),
            route
        );
    }
}

#[test]
fn ping_request_applies_defaults_and_qualifiers() {
    let registry = registry();
    let defaults = registry
        .parse("PING 198.51.100.4")
        .unwrap()
        .stage(0)
        .unwrap();
    assert_eq!(
        syn_shell::network::ping_request(&defaults).unwrap(),
        PingRequest {
            destination: "198.51.100.4",
            count: 3,
            timeout_ms: 1_000,
            size: 32,
            interface: None,
            source: None,
            ip_version: None,
        }
    );

    let qualified = registry
        .parse(
            "PING host.example /COUNT=3 /TIMEOUT=2500 /SIZE=64 /INTERFACE=eth0 /SOURCE=10.0.0.2 /IPV4",
        )
        .unwrap()
        .stage(0)
        .unwrap();
    assert_eq!(
        syn_shell::network::ping_request(&qualified).unwrap(),
        PingRequest {
            destination: "host.example",
            count: 3,
            timeout_ms: 2_500,
            size: 64,
            interface: Some("eth0"),
            source: Some("10.0.0.2"),
            ip_version: Some(PingIpVersion::Ipv4),
        }
    );
}

#[test]
fn ping_resolves_literal_ipv4_before_dns() {
    let registry = registry();
    let literal = registry
        .parse("PING 198.51.100.4 /TIMEOUT=60000")
        .unwrap()
        .stage(0)
        .unwrap();
    let request = syn_shell::network::ping_request(&literal).unwrap();
    let target = syn_shell::network::resolve_literal_ipv4_target(request).unwrap();
    assert_eq!(target.address.as_str(), "198.51.100.4");
    assert_eq!(target.ip_version, PingIpVersion::Ipv4);
    assert_eq!(request.dns_timeout_ms(), 5_000);

    let hostname = registry
        .parse("PING host.example")
        .unwrap()
        .stage(0)
        .unwrap();
    let request = syn_shell::network::ping_request(&hostname).unwrap();
    assert_eq!(
        syn_shell::network::resolve_literal_ipv4_target(request),
        Err(Status::NOT_FOUND)
    );

    let malformed = registry
        .parse("PING 999.1.1.1")
        .unwrap()
        .stage(0)
        .unwrap();
    let request = syn_shell::network::ping_request(&malformed).unwrap();
    assert_eq!(
        syn_shell::network::resolve_literal_ipv4_target(request),
        Err(Status::INVALID_ARGUMENT)
    );
}

#[test]
fn ping_results_have_stable_names_and_statuses() {
    let registry = registry();
    let call = registry
        .parse("PING 198.51.100.4")
        .unwrap()
        .stage(0)
        .unwrap();
    let request = syn_shell::network::ping_request(&call).unwrap();
    let target = syn_shell::network::resolve_literal_ipv4_target(request).unwrap();
    let request = ResolvedPingRequest { request, target };
    for result in [
        PingResult::Success,
        PingResult::Timeout,
        PingResult::Unreachable,
        PingResult::NoRoute,
        PingResult::LinkDown,
        PingResult::DnsFailure,
        PingResult::PermissionDenied,
        PingResult::MalformedReply,
        PingResult::Cancelled,
    ] {
        let output = syn_shell::network::ping_result_output(request, result).unwrap();
        assert!(has_text(&output, "result", result.as_str()));
        assert_eq!(output.status(), result.status());
    }

    let timeout = syn_shell::network::ping_result_output(request, PingResult::Timeout).unwrap();
    assert!(has_unsigned(&timeout, "transmitted", 3));
    assert!(has_unsigned(&timeout, "received", 0));
    assert!(has_unsigned(&timeout, "loss-percent", 100));
    assert!(lacks_field(&timeout, "rtt-average-ms"));
}

#[test]
fn ping_summary_contains_loss_rtt_and_human_readable_identity() {
    let registry = registry();
    let call = registry
        .parse("PING 198.51.100.4")
        .unwrap()
        .stage(0)
        .unwrap();
    let request = syn_shell::network::ping_request(&call).unwrap();
    let target = syn_shell::network::resolve_literal_ipv4_target(request).unwrap();
    let output = syn_shell::network::ping_summary_output(
        ResolvedPingRequest { request, target },
        PingResult::Success,
        PingSummary {
            transmitted: 3,
            received: 2,
            minimum_rtt_ms: Some(4),
            average_rtt_ms: Some(7),
            maximum_rtt_ms: Some(10),
            replies: [
                Some(PingReply {
                    sequence: 1,
                    ttl: Some(64),
                    payload_size: 32,
                    rtt_ms: Some(4),
                    error: None,
                }),
                Some(PingReply {
                    sequence: 2,
                    ttl: Some(63),
                    payload_size: 32,
                    rtt_ms: Some(10),
                    error: None,
                }),
                Some(PingReply {
                    sequence: 3,
                    ttl: None,
                    payload_size: 32,
                    rtt_ms: None,
                    error: Some(PingResult::Timeout),
                }),
            ],
        },
    )
    .unwrap();
    assert!(has_text(&output, "destination", "198.51.100.4"));
    assert!(has_text(&output, "address", "198.51.100.4"));
    assert!(has_unsigned(&output, "transmitted", 3));
    assert!(has_unsigned(&output, "received", 2));
    assert!(has_unsigned(&output, "lost", 1));
    assert!(has_unsigned(&output, "loss-percent", 33));
    assert!(has_unsigned(&output, "reply1-sequence", 1));
    assert!(has_unsigned(&output, "reply1-ttl", 64));
    assert!(has_unsigned(&output, "reply1-payload-size", 32));
    assert!(has_unsigned(&output, "reply1-rtt-ms", 4));
    assert!(has_text(&output, "reply3-error", "timeout"));
    assert!(has_unsigned(&output, "rtt-min-ms", 4));
    assert!(has_unsigned(&output, "rtt-average-ms", 7));
    assert!(has_unsigned(&output, "rtt-max-ms", 10));
    let rendered = syn_shell::render::render(&output, syn_shell::render::OutputFormat::List)
        .unwrap();
    assert!(rendered.as_str().contains("Transmitted"));
    assert!(rendered.as_str().contains("Average RTT (ms)"));
    assert!(rendered.as_str().contains("198.51.100.4"));
    assert!(rendered.as_str().contains("Reply 1"));
    assert!(rendered.as_str().contains("Payload size"));
    assert!(rendered.as_str().contains("timeout"));
    let json = syn_shell::render::render(&output, syn_shell::render::OutputFormat::Json)
        .unwrap();
    assert!(json.as_str().contains("\"reply1-sequence\":1"));
    assert!(json.as_str().contains("\"reply3-error\":\"timeout\""));
}

#[test]
fn ping_request_rejects_conflicting_or_unbounded_qualifiers() {
    let registry = registry();
    for input in [
        "PING 198.51.100.4 /IPV4 /IPV6",
        "PING 198.51.100.4 /COUNT=0",
        "PING 198.51.100.4 /COUNT=65",
        "PING 198.51.100.4 /TIMEOUT=0",
        "PING 198.51.100.4 /TIMEOUT=60001",
        "PING 198.51.100.4 /SIZE=257",
    ] {
        let program = registry.parse(input).expect("PING syntax should parse");
        assert!(
            syn_shell::network::ping_request(&program.stage(0).unwrap()).is_err(),
            "{input}"
        );
    }
    for input in [
        "PING 198.51.100.4 /COUNT=abc",
        "PING 198.51.100.4 /TIMEOUT=abc",
        "PING 198.51.100.4 /SIZE=abc",
        "PING 198.51.100.4 /INTERFACE=",
        "PING 198.51.100.4 /UNKNOWN",
    ] {
        assert!(registry.parse(input).is_err(), "{input}");
    }
}

#[test]
fn ping_dispatches_a_bounded_request_to_the_network_source() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());
    let output = execute(
        &mut executor,
        "PING host.example /COUNT=2 /TIMEOUT=500 /SIZE=16 /IPV4",
    )
    .unwrap();
    assert!(has_text(&output, "operation", "ping"));
    assert!(has_text(&output, "destination", "host.example"));
    assert!(has_text(&output, "address", "198.51.100.4"));
    assert!(has_unsigned(&output, "count", 2));
    assert!(has_unsigned(&output, "timeout-ms", 500));
    assert!(has_unsigned(&output, "size", 16));
    assert!(has_text(&output, "ip-version", "ipv4"));
    assert_eq!(executor.source().command_log[0], PING_ROUTE);
}

#[test]
fn ping_stays_pending_and_can_be_cancelled() {
    let mut source = FakeNetwork::seeded();
    source.ping_pending = true;
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(source);
    let call = registry()
        .parse("PING 198.51.100.4 /COUNT=3 /TIMEOUT=200")
        .unwrap()
        .stage(0)
        .unwrap();
    let token = executor.submit(call, None).unwrap();
    assert!(executor.poll(token).is_none());
    assert!(executor.source().ping_completion.is_some());
    executor.cancel(token).unwrap();
    assert!(executor.source().ping_completion.is_none());
    assert!(executor.source().ping_cancelled);
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
    let ping = command_help("PING").unwrap();
    assert!(ping.qualifiers.contains("/COUNT"));
    assert!(ping.qualifiers.contains("/TIMEOUT"));
    assert!(ping.qualifiers.contains("/SIZE"));
    assert!(ping.qualifiers.contains("/INTERFACE"));
    assert!(ping.qualifiers.contains("/SOURCE"));
    assert!(ping.qualifiers.contains("/IPV4"));
    assert!(ping.qualifiers.contains("/IPV6"));
    let neighbors = command_help("SHOW-NEIGHBORS").unwrap();
    assert!(neighbors.description.contains("ARP"));
    let clear_neighbors = command_help("CLEAR-NEIGHBORS").unwrap();
    assert!(clear_neighbors.qualifiers.contains("/CONFIRM"));
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
    let ping = registry()
        .parse("PING 198.51.100.4")
        .unwrap()
        .stage(0)
        .unwrap();
    assert!(matches!(
        syn_shell::network::dispatch_network_command(executor.source_mut(), ping),
        Err(Status::ACCESS_DENIED)
    ));
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
    assert!(rendered.as_str().contains("ADDRESS"));
    assert!(rendered.as_str().contains("10.0.0.2"));

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
fn network_command_capture_preserves_mutation_order_and_generation() {
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(FakeNetwork::seeded());
    execute(&mut executor, "SET HOSTNAME node-1").unwrap();
    execute(&mut executor, "SET INTERFACE eth0 /DISABLE").unwrap();
    execute(
        &mut executor,
        "SET ROUTE 10.0.0.0/24 /GATEWAY=10.0.0.1 /INTERFACE=eth0",
    )
    .unwrap();

    let source = executor.source();
    assert_eq!(source.command_count, 3);
    assert_eq!(
        &source.command_log[..source.command_count],
        &[SET_HOSTNAME_ROUTE, SET_INTERFACE_ROUTE, SET_ROUTE_ROUTE]
    );
    assert_eq!(source.view.generation, 4);
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
    assert!(has_unsigned(&interfaces, "interface1-dhcp-transaction-id", 0x1234));
    assert!(has_text(
        &interfaces,
        "interface1-dhcp-client-mac",
        "02:00:00:00:00:01"
    ));
    assert!(has_unsigned(&interfaces, "interface1-dhcp-attempt", 1));
    assert!(has_text(&interfaces, "interface1-dhcp-server", "10.0.0.1"));
    assert!(has_text(
        &interfaces,
        "interface1-dhcp-offered-address",
        "10.0.0.2"
    ));
    assert!(has_unsigned(&interfaces, "interface1-dhcp-t1-ms", 50_000));
    assert!(has_unsigned(&interfaces, "interface1-dhcp-t2-ms", 87_000));
    assert!(has_unsigned(&interfaces, "interface1-dhcp-expires-ms", 60_000));
    assert!(has_unsigned(
        &interfaces,
        "interface1-dhcp-last-packet-ms",
        1_000
    ));
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
        prefix_len: None,
        mac: None,
        gateway: None,
        mtu: 65_535,
        enabled: true,
        link_up: true,
        rx_queue: None,
        tx_queue: None,
        mode: InterfaceAddressMode::Static,
        dhcp: None,
    });
    interfaces[1] = Some(NetworkInterfaceView {
        name: text("eth0"),
        address: text("0.0.0.0"),
        prefix_len: None,
        mac: None,
        gateway: None,
        mtu: 1500,
        enabled: true,
        link_up: false,
        rx_queue: None,
        tx_queue: None,
        mode: InterfaceAddressMode::Static,
        dhcp: None,
    });
    let source = FakeNetwork {
        allowed: true,
        command_log: [0; 8],
        command_count: 0,
        view: NetworkView {
            generation: 1,
            hostname: Some(text("synos")),
            interface_count: 2,
            route_count: 0,
            interfaces,
            routes: [None; MAX_NETWORK_OUTPUT_ROWS],
            link_events: [None; MAX_NETWORK_LINK_EVENTS],
            next_interface: None,
            next_route: None,
        },
        ping_completion: None,
        ping_pending: false,
        ping_cancelled: false,
        neighbors: NeighborView::EMPTY,
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
            prefix_len: Some(24),
            mac: None,
            gateway: Some(text("10.0.0.1")),
            mtu: 1500,
            enabled: true,
            link_up: true,
            rx_queue: None,
            tx_queue: None,
            mode: InterfaceAddressMode::Dhcp,
            dhcp: Some(DhcpLeaseView {
                state: text("bound"),
                transaction_id: Some(0x1234),
                client_mac: Some(text("02:00:00:00:00:01")),
                attempt: Some(1),
                server: Some(text("10.0.0.1")),
                offered_address: Some(text("10.0.0.2")),
                bound_at_ms: Some(1_000),
                next_action_ms: Some(50_000),
                t1_at_ms: Some(50_000),
                t2_at_ms: Some(87_000),
                expires_at_ms: Some(60_000),
                failure_reason: None,
                last_packet_at_ms: Some(1_000),
                dns0: Some(text("10.0.0.53")),
                dns1: Some(text("10.0.0.54")),
            }),
        });
    }
    let source = FakeNetwork {
        allowed: true,
        command_log: [0; 8],
        command_count: 0,
        view: NetworkView {
            generation: 1,
            hostname: Some(text("synos")),
            interface_count: 4,
            route_count: 0,
            interfaces,
            routes: [None; MAX_NETWORK_OUTPUT_ROWS],
            link_events: [None; MAX_NETWORK_LINK_EVENTS],
            next_interface: None,
            next_route: None,
        },
        ping_completion: None,
        ping_pending: false,
        ping_cancelled: false,
        neighbors: NeighborView::EMPTY,
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

#[test]
fn neighbors_show_both_ip_versions_and_clear_requires_confirmation() {
    let mut source = FakeNetwork::seeded();
    source.neighbors = NeighborView {
        generation: 7,
        entry_count: 2,
        entries: [
            Some(NeighborEntryView {
                interface: text("eth0"),
                address: text("10.0.0.1"),
                ip_version: NeighborIpVersion::Ipv4,
                hardware_address: Some(text("02:00:00:00:00:01")),
                state: NeighborState::Reachable,
                last_seen_ms: 100,
                expires_at_ms: Some(60_100),
                attempts: 0,
            }),
            Some(NeighborEntryView {
                interface: text("eth0"),
                address: text("2001:db8::1"),
                ip_version: NeighborIpVersion::Ipv6,
                hardware_address: None,
                state: NeighborState::Pending,
                last_seen_ms: 200,
                expires_at_ms: Some(1_200),
                attempts: 1,
            }),
        ],
        next_entry: None,
    };
    let mut executor: NetworkExecutor<_, 8> = NetworkExecutor::new(source);

    let output = execute(&mut executor, "SHOW NEIGHBORS").unwrap();
    assert!(has_text(&output, "operation", "show-neighbors"));
    assert!(has_unsigned(&output, "entry-count", 2));
    assert!(has_text(&output, "neighbor1-address", "10.0.0.1"));
    assert!(has_text(&output, "neighbor1-ip-version", "ipv4"));
    assert!(has_text(&output, "neighbor1-state", "reachable"));
    assert!(has_text(&output, "neighbor2-address", "2001:db8::1"));
    assert!(has_text(&output, "neighbor2-ip-version", "ipv6"));
    assert!(syn_shell::render::render(&output, syn_shell::render::OutputFormat::List)
        .unwrap()
        .as_str()
        .contains("Neighbor cache"));
    assert!(syn_shell::render::render(&output, syn_shell::render::OutputFormat::Json)
        .unwrap()
        .as_str()
        .contains("\"neighbor2-address\":\"2001:db8::1\""));

    assert!(matches!(
        execute(&mut executor, "CLEAR NEIGHBORS"),
        Err(Status::INVALID_ARGUMENT)
    ));
    let cleared = execute(&mut executor, "CLEAR NEIGHBORS /CONFIRM").unwrap();
    assert!(has_text(&cleared, "operation", "clear-neighbors"));
    assert!(has_unsigned(&cleared, "cleared-count", 2));
    let empty = execute(&mut executor, "SHOW NEIGHBORS").unwrap();
    assert!(has_unsigned(&empty, "entry-count", 0));
}


struct FakeNetwork {
    allowed: bool,
    command_log: [u16; 8],
    command_count: usize,
    view: NetworkView,
    ping_completion: Option<synos_system_model::command::StructuredOutput>,
    ping_pending: bool,
    ping_cancelled: bool,
    neighbors: NeighborView,
}

impl FakeNetwork {
    fn denied() -> Self {
        Self {
            allowed: false,
            command_log: [0; 8],
            command_count: 0,
            view: NetworkView::EMPTY,
            ping_completion: None,
            ping_pending: false,
            ping_cancelled: false,
            neighbors: NeighborView::EMPTY,
        }
    }

    fn seeded() -> Self {
        let mut interfaces = [None; MAX_NETWORK_OUTPUT_ROWS];
        interfaces[0] = Some(NetworkInterfaceView {
            name: text("eth0"),
            address: text("10.0.0.2"),
            prefix_len: Some(24),
            mac: None,
            gateway: Some(text("10.0.0.1")),
            mtu: 1500,
            enabled: true,
            link_up: true,
            rx_queue: None,
            tx_queue: None,
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
            command_log: [0; 8],
            command_count: 0,
            view: NetworkView {
                generation: 1,
                hostname: Some(text("synos")),
                interface_count: 1,
                route_count: 1,
                interfaces,
                routes,
                link_events: [None; MAX_NETWORK_LINK_EVENTS],
                next_interface: None,
                next_route: None,
            },
            ping_completion: None,
            ping_pending: false,
            ping_cancelled: false,
            neighbors: NeighborView::EMPTY,
        }
    }

    fn bump(&mut self) {
        self.view.generation = self.view.generation.saturating_add(1);
    }

    fn interface_mut(&mut self) -> &mut NetworkInterfaceView {
        self.view.interfaces[0].as_mut().expect("seeded interface")
    }

    fn capture_command(&mut self, route: u16) {
        if let Some(slot) = self.command_log.get_mut(self.command_count) {
            *slot = route;
            self.command_count += 1;
        }
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

    fn authorize_ping(
        &mut self,
        _request: ResolvedPingRequest<'_>,
    ) -> Result<u64, Status> {
        if self.allowed {
            Ok(0x50494e47)
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

    fn show_neighbors(&mut self) -> Result<NeighborView, Status> {
        Ok(self.neighbors)
    }

    fn clear_neighbors(&mut self) -> Result<u64, Status> {
        let cleared = self.neighbors.entry_count;
        self.neighbors = NeighborView::EMPTY;
        Ok(cleared)
    }

    fn set_hostname(&mut self, hostname: &str) -> Result<NetworkView, Status> {
        self.capture_command(SET_HOSTNAME_ROUTE);
        self.view.hostname = Some(text(hostname));
        self.bump();
        Ok(self.view)
    }

    fn set_interface(&mut self, update: InterfaceUpdate<'_>) -> Result<NetworkView, Status> {
        self.capture_command(SET_INTERFACE_ROUTE);
        let interface = self.interface_mut();
        if let Some(mode) = update.mode {
            interface.mode = mode;
            if mode == InterfaceAddressMode::Dhcp {
                interface.address = text("10.0.0.50");
                interface.gateway = Some(text("10.0.0.1"));
                interface.dhcp = Some(DhcpLeaseView {
                    state: text("bound"),
                    transaction_id: Some(0x1234),
                    client_mac: Some(text("02:00:00:00:00:01")),
                    attempt: Some(1),
                    server: Some(text("10.0.0.1")),
                    offered_address: Some(text("10.0.0.2")),
                    bound_at_ms: Some(1_000),
                    next_action_ms: Some(50_000),
                    t1_at_ms: Some(50_000),
                    t2_at_ms: Some(87_000),
                    expires_at_ms: Some(60_000),
                    failure_reason: None,
                    last_packet_at_ms: Some(1_000),
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
        self.capture_command(SET_ROUTE_ROUTE);
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

    fn resolve_ping_hostname(
        &mut self,
        hostname: &str,
        ip_version: PingIpVersion,
        _timeout_ms: u32,
    ) -> Result<PingTarget, Status> {
        if hostname != "host.example" {
            return Err(Status::NOT_FOUND)
        }
        Ok(PingTarget {
            address: text("198.51.100.4"),
            ip_version,
        })
    }

    fn start_ping(
        &mut self,
        request: ResolvedPingRequest<'_>,
        schedule: syn_shell::network::PingSchedule,
    ) -> Result<PingHandle, Status> {
        assert_eq!(schedule.count, request.request.count);
        assert_eq!(schedule.packet_timeout_ms, request.request.timeout_ms);
        assert_eq!(schedule.first_sequence, 1);
        self.capture_command(PING_ROUTE);
        self.ping_completion = Some(syn_shell::network::ping_request_output(request)?);
        PingHandle::new(1).ok_or(Status::INVALID_ARGUMENT)
    }

    fn poll_ping(
        &mut self,
        _handle: PingHandle,
    ) -> Option<Result<synos_system_model::command::StructuredOutput, Status>> {
        if self.ping_pending {
            self.ping_pending = false;
            return None
        }
        self.ping_completion.take().map(Ok)
    }

    fn cancel_ping(&mut self, _handle: PingHandle) -> Result<(), Status> {
        self.ping_completion = None;
        self.ping_cancelled = true;
        Ok(())
    }

    fn ping(
        &mut self,
        request: ResolvedPingRequest<'_>,
    ) -> Result<synos_system_model::command::StructuredOutput, Status> {
        self.capture_command(PING_ROUTE);
        syn_shell::network::ping_request_output(request)
    }
}
