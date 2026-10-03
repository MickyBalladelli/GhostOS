#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Advisory {
    pub id: [u8; 64], pub package_hash: [u8; 32], pub id_length: u8, pub source: u8, pub occupied: bool,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Obsolete { pub node: u32, pub package: [u8; 32], pub reason: u8, pub occupied: bool }
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Finding {
    pub package: [u8; 32], pub advisory_package: [u8; 32], pub id: [u8; 64], pub id_length: u8,
    pub source: u8, pub severity: u8, pub withdrawn: bool, pub occupied: bool,
}
pub(crate) fn identifier(id: &[u8]) -> bool {
    unsafe { ghostos_audit_id(id.as_ptr(), id.len()) == 0 }
}
pub(crate) fn advisory_slot(records: &[Advisory], source: u8, id: &[u8], package: &[u8; 32]) -> Result<usize, i32> {
    let mut index = 0;
    let code = unsafe { ghostos_audit_advisory_slot(records.as_ptr(), records.len(), source,
        id.as_ptr(), id.len(), package.as_ptr(), &mut index) };
    if code == 0 { Ok(index) } else { Err(code) }
}
pub(crate) fn obsolete_valid(name_empty: bool, reason: u8, installed: [u32; 3], latest: [u32; 3]) -> bool {
    unsafe { ghostos_audit_obsolete(name_empty, reason, installed[0], installed[1], installed[2],
        latest[0], latest[1], latest[2]) == 0 }
}
pub(crate) fn obsolete_slot(records: &[Obsolete], node: u32, package: &[u8; 32], reason: u8) -> Result<usize, i32> {
    let mut index = 0;
    let code = unsafe { ghostos_audit_obsolete_slot(records.as_ptr(), records.len(), node,
        package.as_ptr(), reason, &mut index) };
    if code == 0 { Ok(index) } else { Err(code) }
}
pub(crate) fn finding_slot(records: &[Finding], package: &[u8; 32], source: u8, id: &[u8],
    advisory_package: &[u8; 32], severity: u8, withdrawn: bool) -> Result<usize, i32> {
    let mut index = 0;
    let code = unsafe { ghostos_audit_finding_slot(records.as_ptr(), records.len(), package.as_ptr(),
        source, id.as_ptr(), id.len(), advisory_package.as_ptr(), severity, withdrawn, &mut index) };
    if code == 0 { Ok(index) } else { Err(code) }
}
pub(crate) fn budget(package_budget: usize) -> bool {
    unsafe { ghostos_audit_budget(package_budget) == 0 }
}
const _: () = {
    assert!(core::mem::size_of::<Advisory>() == 99);
    assert!(core::mem::offset_of!(Advisory, occupied) == 98);
    assert!(core::mem::size_of::<Obsolete>() == 40);
    assert!(core::mem::offset_of!(Obsolete, occupied) == 37);
    assert!(core::mem::size_of::<Finding>() == 133);
    assert!(core::mem::offset_of!(Finding, occupied) == 132);
};
unsafe extern "C" {
    fn ghostos_audit_id(id: *const u8, length: usize) -> i32;
    fn ghostos_audit_advisory_slot(records: *const Advisory, count: usize, source: u8,
        id: *const u8, id_length: usize, package_hash: *const u8, index: *mut usize) -> i32;
    fn ghostos_audit_obsolete(name_empty: bool, reason: u8, installed_major: u32,
        installed_minor: u32, installed_patch: u32, latest_major: u32, latest_minor: u32,
        latest_patch: u32) -> i32;
    fn ghostos_audit_obsolete_slot(records: *const Obsolete, count: usize, node: u32,
        package: *const u8, reason: u8, index: *mut usize) -> i32;
    fn ghostos_audit_finding_slot(records: *const Finding, count: usize, package: *const u8,
        source: u8, id: *const u8, id_length: usize, advisory_package: *const u8, severity: u8,
        withdrawn: bool, index: *mut usize) -> i32;
    fn ghostos_audit_budget(package_budget: usize) -> i32;
}
