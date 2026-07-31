//! MAC address handling.

use std::fmt;

/// Length of a MAC address in bytes.
pub const MAC_ADDRESS_LEN: usize = 6;

/// A 48-bit Ethernet MAC address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddress(pub [u8; MAC_ADDRESS_LEN]);

impl MacAddress {
    pub const BROADCAST: Self = Self([0xFF; MAC_ADDRESS_LEN]);
    pub const fn new(bytes: [u8; MAC_ADDRESS_LEN]) -> Self {
        Self(bytes)
    }
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; MAC_ADDRESS_LEN] = bytes.try_into().ok()?;
        Some(Self(arr))
    }
    pub fn to_bytes(self) -> [u8; MAC_ADDRESS_LEN] {
        self.0
    }
    pub fn is_broadcast(&self) -> bool {
        *self == Self::BROADCAST
    }
    pub fn is_unicast(&self) -> bool {
        self.0[0] & 1 == 0
    }
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 != 0 && !self.is_broadcast()
    }
    pub const fn synos_default(slot: u8) -> Self {
        Self([0x52, 0x54, 0x00, 0x12, 0x34, slot])
    }
}

impl Default for MacAddress {
    fn default() -> Self {
        Self::synos_default(0x56)
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
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
    if promiscuous {
        return true;
    }
    let Some(dst_mac) = MacAddress::from_bytes(dst) else {
        return false;
    };
    dst_mac.is_broadcast() || dst_mac.is_multicast() || dst_mac.0 == own.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_basics() {
        let m = MacAddress::synos_default(0x56);
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
        let own = MacAddress::synos_default(0x56);
        assert!(mac_matches(&own.0, &own, false));
        assert!(mac_matches(&[0xFF; 6], &own, false));
        let other = MacAddress::synos_default(0x57);
        assert!(!mac_matches(&other.0, &own, false));
        assert!(mac_matches(&other.0, &own, true));
    }
}