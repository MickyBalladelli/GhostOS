use synos_shield::key_provider::{
    ApprovalProof, ApprovalScope, ApproverId, Isolation, KeyHandle, KeyId, KeyProvider,
    KeyProviderError, KeyPurpose, OfflineRevocation, ProviderId, QuorumPolicy, RecoveryAuthorization,
};
use synos_shield::key_provider::KeyAuthority;

struct IsolatedProvider {
    id: ProviderId,
    next_key: u8,
    rotations: u8,
    recoveries: u8,
    revocations: u8,
}

impl IsolatedProvider {
    fn new() -> Self {
        Self {
            id: ProviderId::new([1; 16]).unwrap(),
            next_key: 10,
            rotations: 0,
            recoveries: 0,
            revocations: 0,
        }
    }
}

impl KeyProvider for IsolatedProvider {
    fn provider_id(&self) -> ProviderId { self.id }

    fn isolation(&self) -> Isolation { Isolation::HardwareBacked }

    fn generate_key(&mut self, _purpose: KeyPurpose, generation: u64) -> Result<KeyHandle, KeyProviderError> {
        let id = KeyId::new([self.next_key; 16]).unwrap();
        self.next_key += 1;
        KeyHandle::new(self.id, id, generation)
    }

    fn sign(&mut self, _key: KeyHandle, message: &[u8], signature: &mut [u8]) -> Result<usize, KeyProviderError> {
        if signature.len() < message.len() {
            return Err(KeyProviderError::SignatureBufferTooSmall)
        }
        signature[..message.len()].copy_from_slice(message);
        Ok(message.len())
    }

    fn verify_approval(
        &self,
        _scope: ApprovalScope,
        _approver: ApproverId,
        _proof: ApprovalProof,
    ) -> Result<(), KeyProviderError> {
        Ok(())
    }

    fn commit_rotation(&mut self, _old_key: KeyHandle, _new_key: KeyHandle) -> Result<(), KeyProviderError> {
        self.rotations += 1;
        Ok(())
    }

    fn complete_recovery(&mut self, _authorization: RecoveryAuthorization) -> Result<(), KeyProviderError> {
        self.recoveries += 1;
        Ok(())
    }

    fn emergency_revoke(&mut self, _command: OfflineRevocation) -> Result<(), KeyProviderError> {
        self.revocations += 1;
        Ok(())
    }
}

fn proof(value: u8) -> ApprovalProof {
    ApprovalProof::new([value; 32]).unwrap()
}

fn approver(value: u64) -> ApproverId {
    ApproverId::new(value).unwrap()
}

#[test]
fn rotation_and_recovery_keep_secret_material_inside_provider() {
    let quorum = QuorumPolicy::new(2, 3).unwrap();
    let mut authority = KeyAuthority::<_, 4, 4>::new(IsolatedProvider::new()).unwrap();
    let old = authority.generate_and_register(KeyPurpose::Attestation, quorum).unwrap();
    let new_handle = authority.provider_mut().generate_key(KeyPurpose::Attestation, 2).unwrap();

    authority.begin_rotation(7, old.handle, new_handle, 100).unwrap();
    authority.approve_rotation(7, approver(1), proof(1), 10).unwrap();
    assert_eq!(authority.commit_rotation(7, 10), Err(KeyProviderError::Unauthorized));
    authority.approve_rotation(7, approver(2), proof(2), 10).unwrap();
    let rotated = authority.commit_rotation(7, 10).unwrap();
    assert_eq!(rotated.handle, new_handle);
    assert_eq!(authority.sign(old.handle, b"old", &mut [0; 8]), Err(KeyProviderError::NotFound));

    authority.begin_recovery(9, new_handle, 100).unwrap();
    authority.approve_recovery(9, approver(1), proof(3), 10).unwrap();
    authority.approve_recovery(9, approver(2), proof(4), 10).unwrap();
    let recovered = authority.complete_recovery(9, 10).unwrap();
    assert_eq!(recovered.state, synos_shield::key_provider::KeyState::Active);
    assert_eq!(authority.provider().rotations, 1);
    assert_eq!(authority.provider().recoveries, 1);
}

#[test]
fn offline_emergency_revocation_is_immediate_and_snapshot_is_metadata_only() {
    let quorum = QuorumPolicy::new(1, 1).unwrap();
    let mut authority = KeyAuthority::<_, 2, 2>::new(IsolatedProvider::new()).unwrap();
    let key = authority.generate_and_register(KeyPurpose::Recovery, quorum).unwrap();
    let command = OfflineRevocation {
        command_id: 11,
        key: key.handle,
        epoch: 4,
        authorization: proof(9),
    };
    let revoked = authority.emergency_revoke(command).unwrap();
    assert_eq!(revoked.state, synos_shield::key_provider::KeyState::Revoked);
    assert_eq!(authority.sign(key.handle, b"data", &mut [0; 8]), Err(KeyProviderError::Unauthorized));

    let mut snapshot = [key; 2];
    assert_eq!(authority.snapshot(&mut snapshot), 1);
    assert_eq!(snapshot[0].handle, key.handle);
    assert_eq!(authority.provider().revocations, 1);
}

#[test]
fn process_memory_provider_is_rejected() {
    struct UnsafeProvider;
    impl KeyProvider for UnsafeProvider {
        fn provider_id(&self) -> ProviderId { ProviderId::from_bytes([2; 16]) }
        fn isolation(&self) -> Isolation { Isolation::ProcessMemory }
        fn generate_key(&mut self, _: KeyPurpose, _: u64) -> Result<KeyHandle, KeyProviderError> { unreachable!() }
        fn sign(&mut self, _: KeyHandle, _: &[u8], _: &mut [u8]) -> Result<usize, KeyProviderError> { unreachable!() }
        fn verify_approval(&self, _: ApprovalScope, _: ApproverId, _: ApprovalProof) -> Result<(), KeyProviderError> { unreachable!() }
        fn commit_rotation(&mut self, _: KeyHandle, _: KeyHandle) -> Result<(), KeyProviderError> { unreachable!() }
        fn complete_recovery(&mut self, _: RecoveryAuthorization) -> Result<(), KeyProviderError> { unreachable!() }
        fn emergency_revoke(&mut self, _: OfflineRevocation) -> Result<(), KeyProviderError> { unreachable!() }
    }

    assert!(matches!(
        KeyAuthority::<_, 1, 1>::new(UnsafeProvider),
        Err(KeyProviderError::ProviderRejected)
    ));
}
