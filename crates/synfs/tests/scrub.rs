use synos_synfs::{Error, RepairAuthorization, ScrubScope, SynFs};

const BLOCKS: usize = 32;
const FINDINGS: usize = 8;

#[test]
fn scrub_is_read_only_and_covers_each_online_storage_scope() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/system", true)
        .expect("create system directory");
    filesystem
        .write("/system/state", b"healthy")
        .expect("write system state");
    let generation = filesystem.generation();

    for scope in [
        ScrubScope::SynFs,
        ScrubScope::PackageStore,
        ScrubScope::Journals,
        ScrubScope::Snapshots,
        ScrubScope::SystemMetadata,
    ] {
        let preview = filesystem
            .repair_preview::<FINDINGS>(scope)
            .expect("build read-only repair preview");
        assert!(preview.authorization_required);
        assert!(preview.plan.report.read_only);
        assert_eq!(preview.plan.report.scope, scope);
        assert_eq!(preview.plan.report.generation, generation);
    }

    let plan = filesystem
        .scrub::<FINDINGS>(ScrubScope::SynFs)
        .expect("scrub healthy filesystem");
    assert_eq!(filesystem.generation(), generation);
    assert!(plan.report.repairable_count > 0);
    assert_eq!(
        filesystem.repair(plan, None),
        Err(Error::RepairUnauthorized)
    );

    let plan = filesystem
        .scrub::<FINDINGS>(ScrubScope::SynFs)
        .expect("rescrub healthy filesystem");
    let authorization = RepairAuthorization::from_operator_confirmation(1)
        .expect("nonzero confirmation authorizes repair");
    let receipt = filesystem
        .repair(plan, Some(authorization))
        .expect("authorized repair");
    assert!(receipt.changed_blocks > 0);
    assert_eq!(
        receipt.evidence.iter().flatten().count(),
        receipt.changed_blocks
    );
    assert!(receipt
        .evidence
        .iter()
        .flatten()
        .all(|evidence| evidence.before_fingerprint != 0 && evidence.after_fingerprint == 0));

    let clean = filesystem
        .scrub::<FINDINGS>(ScrubScope::SynFs)
        .expect("scrub after authorized repair");
    assert_eq!(clean.report.issue_count, 0);
}
