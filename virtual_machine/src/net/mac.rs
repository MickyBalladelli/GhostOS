//! MAC address handling.

use std::fmt;

unsafe extern "C" {
    fn ghostos_vm_mac_from_bytes(bytes: *const u8, length: usize, out: *mut MacAddress) -> bool;
    fn ghostos_vm_mac_is_broadcast(address: *const MacAddress) -> bool;
    fn ghostos_vm_mac_is_unicast(address: *const MacAddress) -> bool;
    fn ghostos_vm_mac_is_multicast(address: *const MacAddress) -> bool;
    fn ghostos_vm_mac_matches(
        destination: *const u8,
        length: usize,
        own: *const MacAddress,
        promiscuous: bool,
    ) -> bool;
    fn ghostos_vm_mac_format(address: *const MacAddress, output: *mut u8) -> bool;
}

/// Length of a MAC address in bytes.
pub const MAC_ADDRESS_LEN: usize = 6;

/// A 48-bit Ethernet MAC address.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddress(pub [u8; MAC_ADDRESS_LEN]);

impl MacAddress {
    pub const BROADCAST: Self = Self([0xFF; MAC_ADDRESS_LEN]);
    pub const fn new(bytes: [u8; MAC_ADDRESS_LEN]) -> Self {
        Self(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut address = Self([0; MAC_ADDRESS_LEN]);
        unsafe {
            ghostos_vm_mac_from_bytes(bytes.as_ptr(), bytes.len(), &mut address)
                .then_some(address)
        }
    }
    pub fn to_bytes(self) -> [u8; MAC_ADDRESS_LEN] {
        self.0
    }
    pub fn is_broadcast(&self) -> bool {
        unsafe { ghostos_vm_mac_is_broadcast(self) }
    }
    pub fn is_unicast(&self) -> bool {
        unsafe { ghostos_vm_mac_is_unicast(self) }
    }
    pub fn is_multicast(&self) -> bool {
        unsafe { ghostos_vm_mac_is_multicast(self) }
    }
    pub const fn ghostos_default(slot: u8) -> Self {
        Self([0x52, 0x54, 0x00, 0x12, 0x34, slot])
    }
}

impl Default for MacAddress {
    fn default() -> Self {
        Self::ghostos_default(0x56)
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut output = [0u8; 18];
        if !unsafe { ghostos_vm_mac_format(self, output.as_mut_ptr()) } {
            return Err(fmt::Error)
        }
        let text = std::str::from_utf8(&output[..17]).map_err(|_| fmt::Error)?;
        f.write_str(text)
    }
}

impl From<[u8; MAC_ADDRESS_LEN]> for MacAddress {
    fn from(bytes: [u8; MAC_ADDRESS_LEN]) -> Self {
        Self(bytes)
    }
}

/// Returns `true` when a frame whose destination MAC is `dst` (the first six
/// bytes of the frame) should be accepted by a NIC owning `own`.
pub fn mac_matches(dst: &[u8], own: &MacAddress, promiscuous: bool) -> bool {
    unsafe { ghostos_vm_mac_matches(dst.as_ptr(), dst.len(), own, promiscuous) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_basics() {
        let m = MacAddress::ghostos_default(0x56);
        assert!(m.is_unicast());
        assert!(!m.is_broadcast());
        assert_eq!(m.to_string(), "52:54:00:12:34:56");
        let mc = MacAddress::new([0x01, 0, 0, 0, 0, 0]);
        assert!(mc.is_multicast());
        assert!(MacAddress::BROADCAST.is_broadcast());
        assert!(MacAddress::from_bytes(&[1, 2, 3]).is_none());
    }

    #[test]
    fn mac_filtering() {
        let own = MacAddress::ghostos_default(0x56);
        assert!(mac_matches(&own.0, &own, false));
        assert!(mac_matches(&[0xFF; 6], &own, false));
        let other = MacAddress::ghostos_default(0x57);
        assert!(!mac_matches(&other.0, &own, false));
        assert!(mac_matches(&other.0, &own, true));
    }
}
