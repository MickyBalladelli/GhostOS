use ghostos_shell::{
    cluster::{
        command_help, register_cluster_commands, validate, ABANDON_NODE_ROUTE,
        ACCEPT_CLUSTER_ROUTE, ACCEPT_NODE_ROUTE, CREATE_CLUSTER_ROUTE, DELETE_CLUSTER_ROUTE,
        DRAIN_NODE_ROUTE, FENCE_NODE_ROUTE, INVITE_CLUSTER_ROUTE, INVITE_NODE_ROUTE,
        JOIN_CLUSTER_ROUTE, LEAVE_CLUSTER_ROUTE, LIST_CLUSTERS_ROUTE, MODIFY_CLUSTER_ROUTE,
        RECOVER_NODE_ROUTE, REJECT_CLUSTER_ROUTE, REJECT_NODE_ROUTE, REMOVE_FEDERATION_ROUTE,
        REMOVE_NODE_ROUTE, RENAME_CLUSTER_ROUTE, REJOIN_NODE_ROUTE, RESYNC_NODE_ROUTE,
        RETRY_NODE_ROUTE, ROLLBACK_CLUSTER_ROUTE, SET_CLUSTER_ROUTE, SHOW_CLUSTER_ROUTE,
        UNFENCE_NODE_ROUTE, USE_CLUSTER_ROUTE, CLUSTER_COMMAND_COUNT,
    },
    parser::CommandRegistry,
};
use ghostos_status::Status;

fn registry() -> CommandRegistry<32> {
    let mut registry = CommandRegistry::new();
    register_cluster_commands(&mut registry).expect("register cluster dictionary");
    registry
}

fn parse_and_validate(input: &str, route: u16) {
    let registry = registry();
    let program = registry.parse(input).expect("parse cluster command");
    let call = program.stage(0).expect("cluster command stage");
    assert_eq!(call.route.raw(), route, "wrong route for {input}");
    validate(call).expect("valid cluster command");
}

#[test]
fn every_cluster_route_has_a_valid_dcl_invocation() {
    let commands = [
        ("SHOW CLUSTER", SHOW_CLUSTER_ROUTE),
        ("LIST CLUSTERS /LIMIT=2 /PAGE=1", LIST_CLUSTERS_ROUTE),
        ("CREATE CLUSTER compute", CREATE_CLUSTER_ROUTE),
        ("JOIN CLUSTER compute /INVITATION=token /ENDPOINT=node-2", JOIN_CLUSTER_ROUTE),
        ("LEAVE CLUSTER compute /DRAIN /CONFIRM", LEAVE_CLUSTER_ROUTE),
        ("REMOVE CLUSTER compute /DRAIN /CONFIRM", DELETE_CLUSTER_ROUTE),
        ("MODIFY CLUSTER compute /DESCRIPTION=next /CONFIRM", MODIFY_CLUSTER_ROUTE),
        ("RENAME CLUSTER old new", RENAME_CLUSTER_ROUTE),
        ("SET CLUSTER compute /DESCRIPTION=next /CONFIRM", SET_CLUSTER_ROUTE),
        ("USE CLUSTER compute", USE_CLUSTER_ROUTE),
        ("INVITE NODE node-2 /EXPIRATION=100 /ROLE=operator", INVITE_NODE_ROUTE),
        ("ACCEPT NODE node-2 /INVITATION=token /CONFIRM", ACCEPT_NODE_ROUTE),
        ("REJECT NODE node-2 /INVITATION=token /CONFIRM", REJECT_NODE_ROUTE),
        ("REMOVE NODE node-2 /DRAIN /CONFIRM", REMOVE_NODE_ROUTE),
        ("DRAIN NODE node-2 /TIMEOUT=100", DRAIN_NODE_ROUTE),
        ("FENCE NODE node-2 /CONFIRM", FENCE_NODE_ROUTE),
        ("REJOIN NODE node-2 /INVITATION=token /ENDPOINT=node-2", REJOIN_NODE_ROUTE),
        ("INVITE CLUSTER peer /EXPIRATION=100 /SCOPE=compute", INVITE_CLUSTER_ROUTE),
        ("ACCEPT CLUSTER peer /INVITATION=token /CONFIRM", ACCEPT_CLUSTER_ROUTE),
        ("REJECT CLUSTER peer /INVITATION=token /CONFIRM", REJECT_CLUSTER_ROUTE),
        ("REMOVE FEDERATION peer /CONFIRM", REMOVE_FEDERATION_ROUTE),
        ("RETRY NODE node-2", RETRY_NODE_ROUTE),
        ("RESYNC NODE node-2", RESYNC_NODE_ROUTE),
        ("RECOVER NODE node-2 /CONFIRM", RECOVER_NODE_ROUTE),
        ("UNFENCE NODE node-2 /CONFIRM", UNFENCE_NODE_ROUTE),
        ("ROLLBACK CLUSTER /CONFIRM", ROLLBACK_CLUSTER_ROUTE),
        ("ABANDON NODE node-2 /FORCE /CONFIRM", ABANDON_NODE_ROUTE),
    ];

    assert_eq!(commands.len(), CLUSTER_COMMAND_COUNT);
    for (input, route) in commands {
        parse_and_validate(input, route);
    }
}

#[test]
fn destructive_actions_and_ambiguous_views_need_safe_guards() {
    let registry = registry();

    for input in [
        "LEAVE CLUSTER compute /DRAIN",
        "REMOVE NODE node-2 /FORCE",
        "FENCE NODE node-2",
        "ACCEPT CLUSTER peer /INVITATION=token",
        "RECOVER NODE node-2",
    ] {
        let call = registry.parse(input).unwrap().stage(0).unwrap();
        assert_eq!(validate(call), Err(Status::INVALID_ARGUMENT), "{input}");
    }

    let call = registry
        .parse("SHOW CLUSTER /MEMBERS /HEALTH")
        .unwrap()
        .stage(0)
        .unwrap();
    assert_eq!(validate(call), Err(Status::INVALID_ARGUMENT));

    let call = registry
        .parse("LIST CLUSTERS /LIMIT=0")
        .unwrap()
        .stage(0)
        .unwrap();
    assert_eq!(validate(call), Err(Status::INVALID_ARGUMENT));
}

#[test]
fn help_and_aliases_cover_the_stable_command_surface() {
    let registry = registry();
    assert_eq!(registry.registrations().count(), CLUSTER_COMMAND_COUNT + 1);
    for name in [
        "SHOW-CLUSTER",
        "LIST-CLUSTERS",
        "CREATE-CLUSTER",
        "JOIN-CLUSTER",
        "LEAVE-CLUSTER",
        "DELETE-CLUSTER",
        "INVITE-NODE",
        "FENCE-NODE",
        "INVITE-CLUSTER",
        "REMOVE-FEDERATION",
        "RECOVER-NODE",
        "ABANDON-NODE",
    ] {
        assert!(command_help(name).is_some(), "missing help for {name}");
    }
    assert_eq!(command_help("CLUSTER").unwrap().name, "SHOW-CLUSTER");
    assert_eq!(command_help("REMOVE-CLUSTER").unwrap().name, "DELETE-CLUSTER");

    assert_eq!(registry.parse("CLUSTER /HEALTH").unwrap().stage(0).unwrap().route.raw(), SHOW_CLUSTER_ROUTE);
    assert_eq!(registry.parse("REMOVE CLUSTER old /CONFIRM").unwrap().stage(0).unwrap().route.raw(), DELETE_CLUSTER_ROUTE);
}
