use synos_observability::{
    CapabilityDomain, CapabilityTrace, CapabilityTraceStage, Level, decode_record, encode_record,
};

#[test]
fn capability_trace_round_trips_every_operation_domain_and_stage() {
    let domains = [
        CapabilityDomain::Filesystem,
        CapabilityDomain::Network,
        CapabilityDomain::Process,
        CapabilityDomain::Storage,
        CapabilityDomain::Cluster,
        CapabilityDomain::Compiler,
    ];
    let stages = [
        CapabilityTraceStage::Created,
        CapabilityTraceStage::KernelIpc,
        CapabilityTraceStage::DaemonAuthorized,
        CapabilityTraceStage::ShellOutput,
        CapabilityTraceStage::AuditRecorded,
        CapabilityTraceStage::Revoked,
    ];

    for (domain_index, domain) in domains.into_iter().enumerate() {
        for (stage_index, stage) in stages.into_iter().enumerate() {
            let expected = CapabilityTrace::new(
                domain,
                stage,
                0x1000 + domain_index as u64,
                stage_index as u16 + 1,
            )
            .expect("non-zero capability trace");
            let mut bytes = [0; 128];
            encode_record(expected.event(Level::Info), &mut bytes).expect("encode trace");
            let decoded = decode_record(&bytes).expect("decode trace");
            assert_eq!(CapabilityTrace::from_event(decoded), Some(expected));
        }
    }
}
