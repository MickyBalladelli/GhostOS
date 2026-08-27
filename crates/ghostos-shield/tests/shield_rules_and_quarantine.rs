use ghostos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use ghostos_fabric::dsm::{DsmHeader, DsmPacket, MessageKind};
use ghostos_init::ProcessId;
use ghostos_shield::{
    Error,
    fabric::{FrameKey, FrameVerifier, SignedFrame},
    integrity::{PageHashManifest, PageSample, RuntimePageVerifier},
    probes::{BehaviorMonitor, CapabilityRule, MemoryRule, Operation, ProbeEvent, ProbeOutcome, ProbePolicy, ProbeRecorder},
    response::{ActiveCountermeasures, CapabilityLease, CleanWorkerRequest, HoneypotDecision, HoneypotRegion, IncidentRequest, IncidentResponder, IncidentRuntime as IncidentRuntimeTrait},
};

#[test]
fn shield_rules_deny_bad_memory_and_throttle_bursts() {
    let mut policy = ProbePolicy::new(100, 1);
    policy.add_capability_rule(CapabilityRule { subject: 7, capability: 9, operations: Operation::MemoryRead.mask() | Operation::IpcSend.mask() }).unwrap();
    policy.add_memory_rule(MemoryRule::new(PAGE_SIZE, PAGE_SIZE, false, false).unwrap()).unwrap();
    let mut monitor = BehaviorMonitor::<_, 2>::new(ProbeRecorder::<4>::new(), policy).unwrap();
    let read = ProbeEvent::memory_access(10, NodeId::LOCAL, 7, 9, Operation::MemoryRead, PAGE_SIZE, 16);
    assert_eq!(monitor.observe(read).unwrap(), ProbeOutcome::Allowed);
    assert_eq!(monitor.observe(ProbeEvent::capability_use(20, NodeId::LOCAL, 7, 9, Operation::IpcSend)).unwrap(), ProbeOutcome::Throttled { retry_after_us: 90 });
    let write = ProbeEvent::memory_access(30, NodeId::LOCAL, 7, 9, Operation::MemoryWrite, PAGE_SIZE, 16);
    assert_eq!(monitor.observe(write).unwrap(), ProbeOutcome::Denied);
    assert_eq!(monitor.sink().events().count(), 1);
}

#[test]
fn revocation_and_honeypot_quarantine_are_immediate() {
    let mut countermeasures = ActiveCountermeasures::<2, 2, 2>::new();
    let region = HoneypotRegion::new(1, PAGE_SIZE * 2, PAGE_SIZE).unwrap();
    countermeasures.honeypots.add(region).unwrap();
    let event = ProbeEvent::memory_access(1, NodeId::LOCAL, 4, 5, Operation::MemoryRead, PAGE_SIZE * 2, 8);
    let decision = countermeasures.inspect(event).unwrap();
    assert!(matches!(decision, ghostos_shield::response::CountermeasureDecision::Quarantined { .. }));
    assert_eq!(countermeasures.revocations.check(CapabilityLease::new(4, 5, 1).unwrap()), Err(Error::Unauthorized));
    assert!(matches!(countermeasures.honeypots.inspect(event), HoneypotDecision::Quarantine(_)));
}

#[test]
fn signed_frames_reject_replay_and_out_of_range_memory() {
    let header = DsmHeader {
        kind: MessageKind::PageRequest,
        source: NodeId::LOCAL,
        destination: NodeId::new(2).unwrap(),
        sequence: 1,
        page_address: PAGE_SIZE,
        lease_epoch: 3,
        fragment: 0,
        fragment_count: 1,
    };
    let packet = DsmPacket::new(header, b"request").unwrap();
    let key = FrameKey::new([6; 32]);
    let frame = SignedFrame::new(packet, key).unwrap();
    let mut verifier = FrameVerifier::<1>::new();
    verifier.trust_peer(NodeId::LOCAL, key).unwrap();
    let allowed = AddressRange::new(PAGE_SIZE, PAGE_SIZE * 2).unwrap();
    verifier.verify(&frame, NodeId::new(2).unwrap(), allowed).unwrap();
    assert_eq!(verifier.verify(&frame, NodeId::new(2).unwrap(), allowed), Err(Error::Replay));
    assert_eq!(verifier.verify(&frame, NodeId::LOCAL, allowed), Err(Error::Unauthorized));

    let mut encoded = vec![0; 2048];
    let length = frame.encode(&mut encoded).unwrap();
    encoded[length - 1] ^= 1;
    let tampered = SignedFrame::decode(&encoded[..length]).unwrap();
    assert_eq!(tampered.verify(key), Err(Error::SignatureMismatch));
}

#[test]
fn executable_measurements_report_all_mismatches() {
    let bytes = b"trusted image";
    let package = ghostos_system_model::ContentId::hash(b"package");
    let manifest = PageHashManifest::<2>::from_bytes(package, ghostos_system_model::ContentId::hash(bytes), bytes).unwrap();
    let mut verifier = RuntimePageVerifier::<2, 2, 2>::new();
    verifier.register_image(manifest).unwrap();
    verifier.bind_process(7, package, PAGE_SIZE).unwrap();
    assert_eq!(verifier.verify_page(7, PAGE_SIZE, bytes).unwrap().page_index, 0);
    let report = verifier.audit_process(7, &[PageSample { address: PAGE_SIZE, bytes }, PageSample { address: PAGE_SIZE, bytes: b"tampered data" }]).unwrap();
    assert_eq!(report.pages_checked, 2);
    assert_eq!(report.mismatches, 1);
    assert_eq!(report.first_mismatch_address, Some(PAGE_SIZE));
}

struct IncidentRuntime;

impl IncidentRuntimeTrait for IncidentRuntime {
    type Error = ();

    fn freeze_process_tree(&mut self, _process: ProcessId) -> Result<(), Self::Error> { Ok(()) }
    fn spawn_clean_worker(&mut self, _request: CleanWorkerRequest) -> Result<ProcessId, Self::Error> { Ok(ProcessId::new(8).unwrap()) }
}

#[test]
fn incident_response_pins_forensics_before_clean_replacement() {
    let process = ProcessId::new(7).unwrap();
    let mut filesystem = ghostos_ghostfs::SynFs::<64>::new();
    let mut responder = IncidentResponder::<2>::new();
    let receipt = responder.respond(&mut filesystem, &mut IncidentRuntime, IncidentRequest::new(process, 0x44, 1)).unwrap();
    assert_eq!(receipt.record.replacement, Some(ProcessId::new(8).unwrap()));
    assert_eq!(receipt.record.state, ghostos_shield::response::IncidentState::Replaced);
    assert_eq!(filesystem.diagnostics().unwrap().checkpoints, 1);
}
