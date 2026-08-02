use synos_fabric::{
    Access, AddressRange, NodeId, PageFault,
    dsm::{RemotePageAuthority, SoftwareDlmLease},
};
use synos_kernel::{
    AddressSpaceId, CapabilityHandle, CapabilityInfo, CapabilityObject, CapabilityRevocationHook,
    CapabilitySpace, Rights,
};

use crate::token::{
    CapabilityKey, CryptographicCapability, TokenError, TransportRights,
};

pub const DEFAULT_LENDING_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LendingKind {
    Ram,
    Vram,
    Compute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct LendingRights(Rights);

impl LendingRights {
    pub const READ: Self = Self(Rights::READ);
    pub const WRITE: Self = Self(Rights::WRITE);
    pub const READ_WRITE: Self = Self(Rights::READ.union(Rights::WRITE));
    pub const EXECUTE: Self = Self(Rights::EXECUTE);

    pub const fn kernel_rights(self) -> Rights {
        self.0
    }

    pub const fn permits(self, access: Access) -> bool {
        match access {
            Access::Read => self.0.contains(Rights::READ),
            Access::Write => self.0.contains(Rights::WRITE),
            Access::Execute => self.0.contains(Rights::EXECUTE),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Loan {
    active: bool,
    provider: NodeId,
    borrower: NodeId,
    resource: u64,
    kind: LendingKind,
    range: Option<AddressRange>,
    compute_units: u32,
    rights: LendingRights,
    epoch: u64,
    expires_at_us: u64,
    token: CryptographicCapability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevocationAction {
    pub borrower: NodeId,
    pub resource: u64,
    pub kind: LendingKind,
    pub range: Option<AddressRange>,
    pub unmap_remote_memory: bool,
    pub invalidate_dsm_pages: bool,
    pub stop_compute: bool,
}

/// Owner-controlled, fixed-capacity RAM/VRAM/compute lending table.
pub struct ResourceLender<const CAPACITY: usize = DEFAULT_LENDING_CAPACITY> {
    provider: NodeId,
    key: CapabilityKey,
    loans: [Option<Loan>; CAPACITY],
    pending_revocations: [Option<RevocationAction>; CAPACITY],
    next_nonce: u64,
}

impl<const CAPACITY: usize> ResourceLender<CAPACITY> {
    pub const fn new(provider: NodeId, key: CapabilityKey, boot_nonce: u64) -> Self {
        Self {
            provider,
            key,
            loans: [None; CAPACITY],
            pending_revocations: [None; CAPACITY],
            next_nonce: boot_nonce,
        }
    }

    pub const fn provider(&self) -> NodeId {
        self.provider
    }

    #[allow(clippy::too_many_arguments)]
    pub fn lend_memory(
        &mut self,
        borrower: NodeId,
        resource: u64,
        kind: LendingKind,
        range: AddressRange,
        rights: LendingRights,
        transports: TransportRights,
        now_us: u64,
        duration_us: u64,
    ) -> Result<CryptographicCapability, LendingError> {
        if !matches!(kind, LendingKind::Ram | LendingKind::Vram)
            || rights.0.contains(Rights::EXECUTE)
        {
            return Err(LendingError::Invalid)
        }
        self.issue(
            borrower,
            resource,
            kind,
            Some(range),
            0,
            rights,
            transports,
            now_us,
            duration_us,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn lend_compute(
        &mut self,
        borrower: NodeId,
        resource: u64,
        compute_units: u32,
        transports: TransportRights,
        now_us: u64,
        duration_us: u64,
    ) -> Result<CryptographicCapability, LendingError> {
        if compute_units == 0 {
            return Err(LendingError::Invalid)
        }
        self.issue(
            borrower,
            resource,
            LendingKind::Compute,
            None,
            compute_units,
            LendingRights::EXECUTE,
            transports,
            now_us,
            duration_us,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn issue(
        &mut self,
        borrower: NodeId,
        resource: u64,
        kind: LendingKind,
        range: Option<AddressRange>,
        compute_units: u32,
        rights: LendingRights,
        transports: TransportRights,
        now_us: u64,
        duration_us: u64,
    ) -> Result<CryptographicCapability, LendingError> {
        if resource == 0 || duration_us == 0 {
            return Err(LendingError::Invalid)
        }
        if self
            .loans
            .iter()
            .flatten()
            .any(|loan| loan.active && loan.resource == resource)
        {
            return Err(LendingError::Busy)
        }
        let slot = self
            .loans
            .iter()
            .position(|loan| loan.is_none() || loan.is_some_and(|loan| !loan.active))
            .ok_or(LendingError::Capacity)?;
        let prior_epoch = self.loans[slot].map(|loan| loan.epoch).unwrap_or(0);
        let epoch = prior_epoch.wrapping_add(1).max(1);
        self.next_nonce = self.next_nonce.wrapping_add(1).max(1);
        let expires_at_us = now_us.saturating_add(duration_us);
        let token = CryptographicCapability::issue(
            self.key,
            self.provider,
            borrower,
            resource,
            rights.kernel_rights(),
            transports,
            now_us,
            expires_at_us,
            epoch,
            self.next_nonce,
        )
        .map_err(LendingError::Token)?;
        self.loans[slot] = Some(Loan {
            active: true,
            provider: self.provider,
            borrower,
            resource,
            kind,
            range,
            compute_units,
            rights,
            epoch,
            expires_at_us,
            token,
        });
        Ok(token)
    }

    pub fn authorize_remote_fault(
        &self,
        token: &CryptographicCapability,
        requester: NodeId,
        fault: PageFault,
        transport: TransportRights,
        dlm_lease: SoftwareDlmLease,
        now_us: u64,
    ) -> Result<RemotePageAuthority, LendingError> {
        let loan = self.loan(token.resource)?;
        if loan.provider != self.provider
            || loan.borrower != requester
            || !matches!(loan.kind, LendingKind::Ram | LendingKind::Vram)
            || loan.token.nonce != token.nonce
            || now_us >= loan.expires_at_us
            || !loan.rights.permits(fault.access)
        {
            return Err(LendingError::AccessDenied)
        }
        token
            .verify(
                self.key,
                requester,
                required_right(fault.access),
                transport,
                now_us,
                loan.epoch,
            )
            .map_err(LendingError::Token)?;
        let range = loan.range.ok_or(LendingError::Invalid)?;
        if !range.contains(fault.page_address())
            || dlm_lease.owner != requester
            || dlm_lease.epoch as u64 != loan.epoch
            || now_us >= dlm_lease.expires_at_us
        {
            return Err(LendingError::AccessDenied)
        }
        Ok(RemotePageAuthority {
            subject: requester,
            range,
            read: loan.rights.0.contains(Rights::READ),
            write: loan.rights.0.contains(Rights::WRITE),
            lease_epoch: dlm_lease.epoch,
            expires_at_us: core::cmp::min(loan.expires_at_us, dlm_lease.expires_at_us),
        })
    }

    pub fn authorize_compute(
        &self,
        token: &CryptographicCapability,
        requester: NodeId,
        requested_units: u32,
        transport: TransportRights,
        now_us: u64,
    ) -> Result<(), LendingError> {
        let loan = self.loan(token.resource)?;
        if loan.kind != LendingKind::Compute
            || requested_units == 0
            || requested_units > loan.compute_units
            || loan.borrower != requester
        {
            return Err(LendingError::AccessDenied)
        }
        token
            .verify(
                self.key,
                requester,
                Rights::EXECUTE,
                transport,
                now_us,
                loan.epoch,
            )
            .map_err(LendingError::Token)
    }

    pub fn revoke(&mut self, resource: u64) -> Result<RevocationAction, LendingError> {
        let slot = self
            .loans
            .iter()
            .position(|loan| loan.is_some_and(|loan| loan.active && loan.resource == resource))
            .ok_or(LendingError::NotFound)?;
        let loan = self.loans[slot].as_mut().expect("active loan");
        loan.active = false;
        loan.epoch = loan.epoch.wrapping_add(1).max(1);
        let action = revocation_action(*loan);
        self.queue_revocation(action);
        Ok(action)
    }

    /// Revoke a remote memory loan only through a matching kernel resource
    /// capability carrying REVOKE. The loan epoch is advanced before the
    /// capability subtree is removed, so already-issued remote tokens fail
    /// validation immediately.
    pub fn revoke_with_capability<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &mut CapabilitySpace<CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        resource: u64,
    ) -> Result<RevocationAction, LendingError> {
        let resource_id = synos_kernel::ResourceId::new(resource)
            .ok_or(LendingError::Invalid)?;
        capabilities
            .authorize(
                caller,
                authority,
                CapabilityObject::DistributedResource(resource_id),
                Rights::REVOKE,
            )
            .map_err(|_| LendingError::AccessDenied)?;
        self.loan(resource)?;
        let action = self.revoke(resource)?;
        capabilities
            .revoke_remote_memory(caller, authority, resource_id)
            .map_err(|_| LendingError::AccessDenied)?;
        Ok(action)
    }

    pub fn take_revocation(&mut self) -> Option<RevocationAction> {
        let slot = self
            .pending_revocations
            .iter()
            .position(Option::is_some)?;
        self.pending_revocations[slot].take()
    }

    fn loan(&self, resource: u64) -> Result<&Loan, LendingError> {
        self.loans
            .iter()
            .flatten()
            .find(|loan| loan.active && loan.resource == resource)
            .ok_or(LendingError::NotFound)
    }

    fn queue_revocation(&mut self, action: RevocationAction) {
        if let Some(slot) = self
            .pending_revocations
            .iter_mut()
            .find(|entry| entry.is_none())
        {
            *slot = Some(action)
        }
    }
}

impl<const CAPACITY: usize> CapabilityRevocationHook for ResourceLender<CAPACITY> {
    fn revoke(&mut self, capability: CapabilityInfo) {
        let CapabilityObject::DistributedResource(resource) = capability.object else {
            return
        };
        let matching: [bool; CAPACITY] = core::array::from_fn(|index| {
            self.loans[index].is_some_and(|loan| {
                loan.active && loan.resource == resource.raw()
            })
        });
        for (index, matches) in matching.iter().enumerate() {
            if !matches {
                continue
            }
            let loan = self.loans[index].as_mut().expect("matching loan");
            loan.active = false;
            loan.epoch = loan.epoch.wrapping_add(1).max(1);
            let action = revocation_action(*loan);
            self.queue_revocation(action)
        }
    }
}

fn required_right(access: Access) -> Rights {
    match access {
        Access::Read => Rights::READ,
        Access::Write => Rights::WRITE,
        Access::Execute => Rights::EXECUTE,
    }
}

fn revocation_action(loan: Loan) -> RevocationAction {
    let memory = matches!(loan.kind, LendingKind::Ram | LendingKind::Vram);
    RevocationAction {
        borrower: loan.borrower,
        resource: loan.resource,
        kind: loan.kind,
        range: loan.range,
        unmap_remote_memory: memory,
        invalidate_dsm_pages: memory,
        stop_compute: loan.kind == LendingKind::Compute,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LendingError {
    AccessDenied,
    Busy,
    Capacity,
    Invalid,
    NotFound,
    Token(TokenError),
}
