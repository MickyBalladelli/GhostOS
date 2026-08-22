//! The bounded integration layer that makes the network service usable.
//!
//! Drivers own hardware. `ghostos-netd` owns packet queues and policy. This
//! module is the small, explicit boundary between those two worlds: it also
//! owns the durable boot configuration and the two tiny UDP services needed
//! before higher-level applications can start.

use ghostos_ghostfs::{Error as SynFsError, SynFs};

use crate::{
    install_core_network_rules, install_dhcp_client_rules, FirewallError, FirewallPolicy,
    InterfaceConfig, PacketError, PolicyStore, QueueDevice, MAX_CORE_NETWORK_RULES,
    MAX_INTERFACE_NAME,
};

pub const MAX_NETWORK_INTERFACES: usize = 8;
pub const MAX_DNS_SERVERS: usize = 2;
pub const MAX_NTP_SERVERS: usize = 2;
pub const NETWORK_CONFIG_PATH: &str = "/system/network/config";
const NETWORK_CONFIG_MAGIC: &[u8; 8] = b"SYNETCFG";
const NETWORK_CONFIG_VERSION: u8 = 1;
const MAX_CONFIG_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceName {
    bytes: [u8; MAX_INTERFACE_NAME],
    length: u8,
}

impl InterfaceName {
    pub fn new(name: &str) -> Option<Self> {
        if name.is_empty() || name.len() > MAX_INTERFACE_NAME || !name.is_ascii() {
            return None
        }
        let mut bytes = [0; MAX_INTERFACE_NAME];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Some(Self {
            bytes,
            length: name.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize]).unwrap_or("")
    }

    pub const fn is_empty(self) -> bool {
        self.length == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceDescriptor {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub kind: u16,
    pub mac: [u8; 6],
    pub link_up: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceInventory<const CAPACITY: usize = MAX_NETWORK_INTERFACES> {
    entries: [Option<(InterfaceName, InterfaceDescriptor)>; CAPACITY],
    count: usize,
}

impl<const CAPACITY: usize> InterfaceInventory<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [None; CAPACITY],
            count: 0,
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = (InterfaceName, InterfaceDescriptor)> + '_ {
        self.entries.iter().flatten().copied()
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

impl<const CAPACITY: usize> Default for InterfaceInventory<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Give discovered NICs stable names in PCI enumeration order. The name is
/// independent of the driver kind, so replacing a driver does not rename the
/// interface. `eth0` is therefore always the first discovered NIC.
pub fn discover_interfaces<const CAPACITY: usize>(
    descriptors: &[InterfaceDescriptor],
    inventory: &mut InterfaceInventory<CAPACITY>,
) -> usize {
    inventory.entries = [None; CAPACITY];
    inventory.count = 0;
    for descriptor in descriptors.iter().copied() {
        if inventory.count == CAPACITY {
            break
        }
        if descriptor.mac == [0; 6] {
            continue
        }
        let mut name_bytes = [0; MAX_INTERFACE_NAME];
        let mut cursor = 0;
        name_bytes[cursor] = b'e';
        cursor += 1;
        name_bytes[cursor] = b't';
        cursor += 1;
        name_bytes[cursor] = b'h';
        cursor += 1;
        cursor += write_decimal(&mut name_bytes[cursor..], inventory.count);
        let name = InterfaceName {
            bytes: name_bytes,
            length: cursor as u8,
        };
        inventory.entries[inventory.count] = Some((name, descriptor));
        inventory.count += 1;
    }
    inventory.count
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkMode {
    Static,
    Dhcp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInterfaceConfig {
    pub name: InterfaceName,
    pub mode: NetworkMode,
    pub enabled: bool,
    pub mtu: u32,
    pub address: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: Option<[u8; 4]>,
}

impl NetworkInterfaceConfig {
    pub fn static_config(
        name: &str,
        address: [u8; 4],
        subnet_mask: [u8; 4],
        gateway: Option<[u8; 4]>,
    ) -> Option<Self> {
        Some(Self {
            name: InterfaceName::new(name)?,
            mode: NetworkMode::Static,
            enabled: true,
            mtu: 1500,
            address,
            subnet_mask,
            gateway,
        })
    }

    pub fn dhcp(name: &str) -> Option<Self> {
        Some(Self {
            name: InterfaceName::new(name)?,
            mode: NetworkMode::Dhcp,
            enabled: true,
            mtu: 1500,
            address: [0; 4],
            subnet_mask: [0; 4],
            gateway: None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkConfig<const INTERFACES: usize = MAX_NETWORK_INTERFACES> {
    pub generation: u64,
    pub hostname: InterfaceName,
    interfaces: [Option<NetworkInterfaceConfig>; INTERFACES],
    interface_count: usize,
    pub dns: [[u8; 4]; MAX_DNS_SERVERS],
    pub dns_count: u8,
    pub ntp: [[u8; 4]; MAX_NTP_SERVERS],
    pub ntp_count: u8,
}

impl<const INTERFACES: usize> NetworkConfig<INTERFACES> {
    pub fn new(hostname: &str) -> Option<Self> {
        Some(Self {
            generation: 1,
            hostname: InterfaceName::new(hostname)?,
            interfaces: [None; INTERFACES],
            interface_count: 0,
            dns: [[0; 4]; MAX_DNS_SERVERS],
            dns_count: 0,
            ntp: [[0; 4]; MAX_NTP_SERVERS],
            ntp_count: 0,
        })
    }

    pub fn interfaces(&self) -> impl Iterator<Item = NetworkInterfaceConfig> + '_ {
        self.interfaces[..self.interface_count]
            .iter()
            .flatten()
            .copied()
    }

    pub fn interface(&self, name: &str) -> Option<NetworkInterfaceConfig> {
        self.interfaces()
            .find(|interface| interface.name.as_str() == name)
    }

    pub fn interface_config(
        &self,
        name: &str,
    ) -> Option<InterfaceConfig> {
        let interface = self.interface(name)?;
        if interface.mode == NetworkMode::Dhcp {
            return None
        }
        let mut config = InterfaceConfig::new(interface.address, interface.subnet_mask);
        config.gateway = interface.gateway;
        Some(config)
    }

    pub fn boot_action(&self, name: &str) -> Option<NetworkBootAction> {
        let interface = self.interface(name)?;
        if !interface.enabled {
            return Some(NetworkBootAction::Disabled)
        }
        match interface.mode {
            NetworkMode::Static => self
                .interface_config(name)
                .map(NetworkBootAction::Static),
            NetworkMode::Dhcp => Some(NetworkBootAction::Dhcp(interface.name)),
        }
    }

    pub fn add_interface(&mut self, interface: NetworkInterfaceConfig) -> Result<(), NetworkConfigError> {
        if interface.name.is_empty()
            || self.interfaces().any(|current| current.name == interface.name)
            || self.interface_count == INTERFACES
            || interface.mtu < 576
            || interface.mtu > 65_535
            || (interface.mode == NetworkMode::Static && !valid_ipv4(interface.address))
            || (interface.mode == NetworkMode::Static && !valid_mask(interface.subnet_mask))
            || interface.gateway.is_some_and(|gateway| !valid_ipv4(gateway))
        {
            return Err(NetworkConfigError::Invalid)
        }
        self.interfaces[self.interface_count] = Some(interface);
        self.interface_count += 1;
        self.generation = self.generation.saturating_add(1);
        Ok(())
    }

    pub fn set_dns(&mut self, servers: &[[u8; 4]]) -> Result<(), NetworkConfigError> {
        if servers.len() > MAX_DNS_SERVERS || servers.iter().any(|server| !valid_ipv4(*server)) {
            return Err(NetworkConfigError::Invalid)
        }
        self.dns = [[0; 4]; MAX_DNS_SERVERS];
        self.dns[..servers.len()].copy_from_slice(servers);
        self.dns_count = servers.len() as u8;
        self.generation = self.generation.saturating_add(1);
        Ok(())
    }

    pub fn set_ntp(&mut self, servers: &[[u8; 4]]) -> Result<(), NetworkConfigError> {
        if servers.len() > MAX_NTP_SERVERS || servers.iter().any(|server| !valid_ipv4(*server)) {
            return Err(NetworkConfigError::Invalid)
        }
        self.ntp = [[0; 4]; MAX_NTP_SERVERS];
        self.ntp[..servers.len()].copy_from_slice(servers);
        self.ntp_count = servers.len() as u8;
        self.generation = self.generation.saturating_add(1);
        Ok(())
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, NetworkConfigError> {
        if output.len() < MAX_CONFIG_BYTES || self.interface_count > INTERFACES {
            return Err(NetworkConfigError::BufferTooSmall)
        }
        let mut cursor = 0;
        output[..MAX_CONFIG_BYTES].fill(0);
        put_bytes(output, &mut cursor, NETWORK_CONFIG_MAGIC)?;
        put_u8(output, &mut cursor, NETWORK_CONFIG_VERSION)?;
        put_u8(output, &mut cursor, self.interface_count as u8)?;
        put_u64(output, &mut cursor, self.generation)?;
        put_u8(output, &mut cursor, self.hostname.length)?;
        put_bytes(output, &mut cursor, &self.hostname.bytes)?;
        for interface in self.interfaces[..self.interface_count].iter().flatten() {
            put_u8(output, &mut cursor, interface.name.length)?;
            put_bytes(output, &mut cursor, &interface.name.bytes)?;
            put_u8(output, &mut cursor, matches!(interface.mode, NetworkMode::Dhcp) as u8)?;
            put_u8(output, &mut cursor, interface.enabled as u8)?;
            put_u32(output, &mut cursor, interface.mtu)?;
            put_bytes(output, &mut cursor, &interface.address)?;
            put_bytes(output, &mut cursor, &interface.subnet_mask)?;
            put_u8(output, &mut cursor, interface.gateway.is_some() as u8)?;
            put_bytes(output, &mut cursor, &interface.gateway.unwrap_or([0; 4]))?;
        }
        put_u8(output, &mut cursor, self.dns_count)?;
        for server in self.dns {
            put_bytes(output, &mut cursor, &server)?;
        }
        put_u8(output, &mut cursor, self.ntp_count)?;
        for server in self.ntp {
            put_bytes(output, &mut cursor, &server)?;
        }
        Ok(cursor)
    }

    pub fn decode(input: &[u8]) -> Result<Self, NetworkConfigError> {
        let mut cursor = 0;
        if input.len() < 18 || take_bytes(input, &mut cursor, 8)? != NETWORK_CONFIG_MAGIC {
            return Err(NetworkConfigError::Invalid)
        }
        if take_u8(input, &mut cursor)? != NETWORK_CONFIG_VERSION {
            return Err(NetworkConfigError::Invalid)
        }
        let count = take_u8(input, &mut cursor)? as usize;
        if count > INTERFACES {
            return Err(NetworkConfigError::Invalid)
        }
        let generation = take_u64(input, &mut cursor)?;
        let hostname_len = take_u8(input, &mut cursor)? as usize;
        let mut hostname_bytes = [0; MAX_INTERFACE_NAME];
        hostname_bytes.copy_from_slice(take_bytes(input, &mut cursor, MAX_INTERFACE_NAME)?);
        let hostname = InterfaceName {
            bytes: hostname_bytes,
            length: hostname_len as u8,
        };
        if hostname.as_str().is_empty() || hostname_len > MAX_INTERFACE_NAME {
            return Err(NetworkConfigError::Invalid)
        }
        let mut config = Self {
            generation,
            hostname,
            interfaces: [None; INTERFACES],
            interface_count: 0,
            dns: [[0; 4]; MAX_DNS_SERVERS],
            dns_count: 0,
            ntp: [[0; 4]; MAX_NTP_SERVERS],
            ntp_count: 0,
        };
        for _ in 0..count {
            let name_len = take_u8(input, &mut cursor)? as usize;
            let mut name_bytes = [0; MAX_INTERFACE_NAME];
            name_bytes.copy_from_slice(take_bytes(input, &mut cursor, MAX_INTERFACE_NAME)?);
            let mode = if take_u8(input, &mut cursor)? == 0 {
                NetworkMode::Static
            } else {
                NetworkMode::Dhcp
            };
            let enabled = take_u8(input, &mut cursor)? != 0;
            let mtu = take_u32(input, &mut cursor)?;
            let mut address = [0; 4];
            address.copy_from_slice(take_bytes(input, &mut cursor, 4)?);
            let mut subnet_mask = [0; 4];
            subnet_mask.copy_from_slice(take_bytes(input, &mut cursor, 4)?);
            let has_gateway = take_u8(input, &mut cursor)? != 0;
            let mut gateway_bytes = [0; 4];
            gateway_bytes.copy_from_slice(take_bytes(input, &mut cursor, 4)?);
            let name = InterfaceName {
                bytes: name_bytes,
                length: name_len as u8,
            };
            if name.as_str().is_empty() || name_len > MAX_INTERFACE_NAME {
                return Err(NetworkConfigError::Invalid)
            }
            config.add_interface(NetworkInterfaceConfig {
                name,
                mode,
                enabled,
                mtu,
                address,
                subnet_mask,
                gateway: has_gateway.then_some(gateway_bytes),
            })?;
        }
        config.generation = generation;
        config.dns_count = take_u8(input, &mut cursor)?;
        if config.dns_count as usize > MAX_DNS_SERVERS {
            return Err(NetworkConfigError::Invalid)
        }
        for server in &mut config.dns {
            server.copy_from_slice(take_bytes(input, &mut cursor, 4)?);
        }
        config.ntp_count = take_u8(input, &mut cursor)?;
        if config.ntp_count as usize > MAX_NTP_SERVERS {
            return Err(NetworkConfigError::Invalid)
        }
        for server in &mut config.ntp {
            server.copy_from_slice(take_bytes(input, &mut cursor, 4)?);
        }
        if config.dns[..config.dns_count as usize]
            .iter()
            .any(|server| !valid_ipv4(*server))
            || config.ntp[..config.ntp_count as usize]
                .iter()
                .any(|server| !valid_ipv4(*server))
        {
            return Err(NetworkConfigError::Invalid)
        }
        Ok(config)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkConfigError {
    Invalid,
    BufferTooSmall,
    Storage(SynFsError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkBootAction {
    Disabled,
    Static(InterfaceConfig),
    Dhcp(InterfaceName),
}

/// Startup owns policy publication. The daemon can then receive only a
/// published image, instead of silently starting with an empty firewall.
pub struct NetworkFirewallStartup<
    const RULES: usize = { MAX_CORE_NETWORK_RULES + 8 },
    const VERSIONS: usize = 8,
> {
    pub policy: FirewallPolicy<RULES>,
    pub images: PolicyStore<VERSIONS>,
}

impl<const RULES: usize, const VERSIONS: usize> NetworkFirewallStartup<RULES, VERSIONS> {
    pub fn new() -> Result<Self, FirewallError> {
        let mut policy = FirewallPolicy::new();
        install_core_network_rules(&mut policy)?;
        install_dhcp_client_rules(&mut policy)?;
        Ok(Self {
            policy,
            images: PolicyStore::new(),
        })
    }

    pub fn activate(&mut self) -> Result<u64, FirewallError> {
        self.images.publish(&self.policy)
    }

    pub const fn active_version(&self) -> Option<u64> {
        self.images.current()
    }
}

pub struct NetworkConfigStore;

impl NetworkConfigStore {
    pub fn save<const BLOCKS: usize, const INTERFACES: usize>(
        filesystem: &mut SynFs<BLOCKS>,
        config: &NetworkConfig<INTERFACES>,
    ) -> Result<(), NetworkConfigError> {
        match filesystem.create_directory("/system/network", true) {
            Ok(_) | Err(SynFsError::AlreadyExists) => {}
            Err(error) => return Err(NetworkConfigError::Storage(error)),
        }
        let mut bytes = [0; MAX_CONFIG_BYTES];
        let length = config.encode(&mut bytes)?;
        let mut transaction = filesystem.transaction();
        transaction
            .write(NETWORK_CONFIG_PATH, &bytes[..length])
            .map_err(NetworkConfigError::Storage)?;
        transaction
            .commit()
            .map(|_| ())
            .map_err(NetworkConfigError::Storage)
    }

    pub fn load<const BLOCKS: usize, const INTERFACES: usize>(
        filesystem: &SynFs<BLOCKS>,
    ) -> Result<NetworkConfig<INTERFACES>, NetworkConfigError> {
        let mut bytes = [0; MAX_CONFIG_BYTES];
        let read = filesystem
            .read(NETWORK_CONFIG_PATH, &mut bytes)
            .map_err(NetworkConfigError::Storage)?;
        NetworkConfig::decode(&bytes[..read.bytes_read])
    }
}

pub trait NetworkNic {
    fn mac(&self) -> [u8; 6];
    fn link_up(&mut self) -> bool;
    fn set_admin_up(&mut self, enabled: bool);
    fn receive(&mut self, buffer: &mut [u8]) -> Result<Option<usize>, NicError>;
    fn transmit(&mut self, frame: &[u8]) -> Result<(), NicError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NicError {
    LinkDown,
    QueueFull,
    FrameTooLarge,
    DeviceUnavailable,
}

pub struct NicQueueBridge<D, const CAPACITY: usize, const MTU: usize> {
    driver: D,
    pub device: QueueDevice<CAPACITY, MTU>,
    rx_frames: u64,
    tx_frames: u64,
    errors: u64,
}

impl<D: NetworkNic, const CAPACITY: usize, const MTU: usize> NicQueueBridge<D, CAPACITY, MTU> {
    pub fn new(driver: D) -> Self {
        Self {
            driver,
            device: QueueDevice::new(),
            rx_frames: 0,
            tx_frames: 0,
            errors: 0,
        }
    }

    pub fn driver(&self) -> &D {
        &self.driver
    }

    pub fn driver_mut(&mut self) -> &mut D {
        &mut self.driver
    }

    pub fn poll(&mut self, rx_budget: usize, tx_budget: usize) -> (usize, usize) {
        if !self.driver.link_up() {
            return (0, 0)
        }
        let mut received = 0;
        for _ in 0..rx_budget {
            let mut writer = match self.device.ingress.reserve() {
                Ok(writer) => writer,
                Err(PacketError::Full) => break,
                Err(_) => break,
            };
            match self.driver.receive(writer.buffer()) {
                Ok(Some(length)) if length <= writer.capacity() => {
                    if writer.commit(length).is_ok() {
                        received += 1;
                        self.rx_frames = self.rx_frames.saturating_add(1);
                    } else {
                        self.errors = self.errors.saturating_add(1);
                    }
                }
                Ok(Some(_)) => self.errors = self.errors.saturating_add(1),
                Ok(None) => break,
                Err(_) => self.errors = self.errors.saturating_add(1),
            }
        }
        let mut transmitted = 0;
        for _ in 0..tx_budget {
            let reader = match self.device.egress.dequeue() {
                Ok(reader) => reader,
                Err(PacketError::Empty) => break,
                Err(_) => break,
            };
            match self.driver.transmit(reader.frame()) {
                Ok(()) => {
                    transmitted += 1;
                    self.tx_frames = self.tx_frames.saturating_add(1);
                }
                Err(_) => self.errors = self.errors.saturating_add(1),
            }
        }
        (received, transmitted)
    }

    pub const fn counters(&self) -> (u64, u64, u64) {
        (self.rx_frames, self.tx_frames, self.errors)
    }

    pub fn set_admin_up(&mut self, enabled: bool) {
        self.driver.set_admin_up(enabled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkRecoveryAction {
    None,
    StopTraffic,
    RestartDhcp,
    ReapplyStatic,
}

pub struct NetworkRecovery {
    link_up: bool,
    service_generation: u64,
    observed_generation: u64,
}

impl NetworkRecovery {
    pub const fn new(link_up: bool) -> Self {
        Self {
            link_up,
            service_generation: 1,
            observed_generation: 1,
        }
    }

    pub fn observe_link(&mut self, link_up: bool) -> NetworkRecoveryAction {
        let action = match (self.link_up, link_up) {
            (true, false) => NetworkRecoveryAction::StopTraffic,
            (false, true) => NetworkRecoveryAction::RestartDhcp,
            _ => NetworkRecoveryAction::None,
        };
        self.link_up = link_up;
        action
    }

    pub fn service_restarted(&mut self) -> NetworkRecoveryAction {
        self.service_generation = self.service_generation.saturating_add(1);
        self.observed_generation = self.service_generation;
        if self.link_up {
            NetworkRecoveryAction::RestartDhcp
        } else {
            NetworkRecoveryAction::StopTraffic
        }
    }

    pub fn static_recovery(&self) -> NetworkRecoveryAction {
        if self.link_up {
            NetworkRecoveryAction::ReapplyStatic
        } else {
            NetworkRecoveryAction::StopTraffic
        }
    }

    pub const fn link_up(&self) -> bool {
        self.link_up
    }

    pub const fn service_generation(&self) -> u64 {
        self.service_generation
    }
}

pub struct DnsResolver {
    servers: [[u8; 4]; MAX_DNS_SERVERS],
    server_count: u8,
    next_server: u8,
    next_id: u16,
}

impl DnsResolver {
    pub const fn new() -> Self {
        Self {
            servers: [[0; 4]; MAX_DNS_SERVERS],
            server_count: 0,
            next_server: 0,
            next_id: 1,
        }
    }

    pub fn set_servers(&mut self, servers: &[[u8; 4]]) -> Result<(), DnsError> {
        if servers.is_empty() || servers.len() > MAX_DNS_SERVERS || servers.iter().any(|server| !valid_ipv4(*server)) {
            return Err(DnsError::InvalidServer)
        }
        self.servers = [[0; 4]; MAX_DNS_SERVERS];
        self.servers[..servers.len()].copy_from_slice(servers);
        self.server_count = servers.len() as u8;
        self.next_server = 0;
        Ok(())
    }

    pub fn next_server(&mut self) -> Option<([u8; 4], u16)> {
        if self.server_count == 0 {
            return None
        }
        let server = self.servers[self.next_server as usize];
        self.next_server = (self.next_server + 1) % self.server_count;
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        Some((server, id))
    }

    pub fn encode_query(name: &str, id: u16, output: &mut [u8]) -> Result<usize, DnsError> {
        if name.is_empty() || name.len() > 253 || output.len() < 12 {
            return Err(DnsError::InvalidName)
        }
        output[..12].fill(0);
        output[..2].copy_from_slice(&id.to_be_bytes());
        output[2..4].copy_from_slice(&0x0100u16.to_be_bytes());
        output[4..6].copy_from_slice(&1u16.to_be_bytes());
        let mut cursor = 12;
        for label in name.split('.') {
            if label.is_empty() || label.len() > 63 || !label.is_ascii() || cursor + label.len() + 2 > output.len() {
                return Err(DnsError::InvalidName)
            }
            output[cursor] = label.len() as u8;
            cursor += 1;
            output[cursor..cursor + label.len()].copy_from_slice(label.as_bytes());
            cursor += label.len();
        }
        if cursor + 5 > output.len() {
            return Err(DnsError::BufferTooSmall)
        }
        output[cursor] = 0;
        output[cursor + 1..cursor + 3].copy_from_slice(&1u16.to_be_bytes());
        output[cursor + 3..cursor + 5].copy_from_slice(&1u16.to_be_bytes());
        Ok(cursor + 5)
    }

    pub fn parse_a_response(
        packet: &[u8],
        expected_id: u16,
        addresses: &mut [[u8; 4]; MAX_DNS_SERVERS],
    ) -> Result<usize, DnsError> {
        if packet.len() < 12 || u16::from_be_bytes([packet[0], packet[1]]) != expected_id {
            return Err(DnsError::InvalidPacket)
        }
        let flags = u16::from_be_bytes([packet[2], packet[3]]);
        let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
        if flags & 0x8000 == 0 || flags & 0x000f != 0 {
            return Err(DnsError::ServerFailure)
        }
        let mut cursor = 12;
        skip_name(packet, &mut cursor)?;
        if cursor + 4 > packet.len() {
            return Err(DnsError::InvalidPacket)
        }
        cursor += 4;
        let mut found = 0;
        for _ in 0..answers {
            skip_name(packet, &mut cursor)?;
            if cursor + 10 > packet.len() {
                return Err(DnsError::InvalidPacket)
            }
            let record_type = u16::from_be_bytes([packet[cursor], packet[cursor + 1]]);
            let class = u16::from_be_bytes([packet[cursor + 2], packet[cursor + 3]]);
            let length = u16::from_be_bytes([packet[cursor + 8], packet[cursor + 9]]) as usize;
            cursor += 10;
            if cursor + length > packet.len() {
                return Err(DnsError::InvalidPacket)
            }
            if record_type == 1 && class == 1 && length == 4 && found < addresses.len() {
                addresses[found].copy_from_slice(&packet[cursor..cursor + 4]);
                found += 1;
            }
            cursor += length;
        }
        if found == 0 {
            Err(DnsError::NoAddress)
        } else {
            Ok(found)
        }
    }
}

impl Default for DnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsError {
    InvalidName,
    InvalidServer,
    InvalidPacket,
    BufferTooSmall,
    ServerFailure,
    NoAddress,
}

pub struct NtpClient {
    servers: [[u8; 4]; MAX_NTP_SERVERS],
    server_count: u8,
    next_server: u8,
    sequence: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NtpSample {
    pub seconds: u32,
    pub fraction: u32,
    pub stratum: u8,
}

impl NtpClient {
    pub const fn new() -> Self {
        Self {
            servers: [[0; 4]; MAX_NTP_SERVERS],
            server_count: 0,
            next_server: 0,
            sequence: 1,
        }
    }

    pub fn set_servers(&mut self, servers: &[[u8; 4]]) -> Result<(), NtpError> {
        if servers.is_empty() || servers.len() > MAX_NTP_SERVERS || servers.iter().any(|server| !valid_ipv4(*server)) {
            return Err(NtpError::InvalidServer)
        }
        self.servers = [[0; 4]; MAX_NTP_SERVERS];
        self.servers[..servers.len()].copy_from_slice(servers);
        self.server_count = servers.len() as u8;
        self.next_server = 0;
        Ok(())
    }

    pub fn encode_request(&mut self, output: &mut [u8; 48]) -> Result<([u8; 4], u32), NtpError> {
        if self.server_count == 0 {
            return Err(NtpError::NoServer)
        }
        let server = self.servers[self.next_server as usize];
        self.next_server = (self.next_server + 1) % self.server_count;
        let sequence = self.sequence;
        self.sequence = self.sequence.wrapping_add(1).max(1);
        output.fill(0);
        output[0] = 0x23;
        output[24..28].copy_from_slice(&sequence.to_be_bytes());
        Ok((server, sequence))
    }

    pub fn parse_response(packet: &[u8], sequence: u32) -> Result<NtpSample, NtpError> {
        if packet.len() < 48 || packet[0] & 0x07 != 4 && packet[0] & 0x07 != 5 {
            return Err(NtpError::InvalidPacket)
        }
        let stratum = packet[1];
        if stratum == 0
            || stratum > 15
            || u32::from_be_bytes([packet[24], packet[25], packet[26], packet[27]]) != sequence
        {
            return Err(NtpError::InvalidPacket)
        }
        let seconds = u32::from_be_bytes([packet[40], packet[41], packet[42], packet[43]]);
        let fraction = u32::from_be_bytes([packet[44], packet[45], packet[46], packet[47]]);
        if seconds == 0 {
            return Err(NtpError::Unsynchronized)
        }
        Ok(NtpSample { seconds, fraction, stratum })
    }
}

impl Default for NtpClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NtpError {
    InvalidServer,
    NoServer,
    InvalidPacket,
    Unsynchronized,
}

fn valid_ipv4(address: [u8; 4]) -> bool {
    address != [0; 4] && address != [255; 4] && address[0] & 0xf0 != 0xe0
}

fn valid_mask(mask: [u8; 4]) -> bool {
    let value = u32::from_be_bytes(mask);
    let prefix = value.leading_ones();
    let expected = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix) };
    value == expected
}

fn write_decimal(output: &mut [u8], mut value: usize) -> usize {
    let mut digits = [0; 20];
    let mut count = 0;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break
        }
    }
    for index in 0..count {
        output[index] = digits[count - index - 1];
    }
    count
}

fn put_u8(output: &mut [u8], cursor: &mut usize, value: u8) -> Result<(), NetworkConfigError> {
    if *cursor >= output.len() {
        return Err(NetworkConfigError::BufferTooSmall)
    }
    output[*cursor] = value;
    *cursor += 1;
    Ok(())
}

fn put_u32(output: &mut [u8], cursor: &mut usize, value: u32) -> Result<(), NetworkConfigError> {
    put_bytes(output, cursor, &value.to_le_bytes())
}

fn put_u64(output: &mut [u8], cursor: &mut usize, value: u64) -> Result<(), NetworkConfigError> {
    put_bytes(output, cursor, &value.to_le_bytes())
}

fn put_bytes(output: &mut [u8], cursor: &mut usize, bytes: &[u8]) -> Result<(), NetworkConfigError> {
    let end = (*cursor).saturating_add(bytes.len());
    if end > output.len() {
        return Err(NetworkConfigError::BufferTooSmall)
    }
    output[*cursor..end].copy_from_slice(bytes);
    *cursor = end;
    Ok(())
}

fn take_bytes<'a>(input: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], NetworkConfigError> {
    let end = cursor.checked_add(length).ok_or(NetworkConfigError::Invalid)?;
    let bytes = input.get(*cursor..end).ok_or(NetworkConfigError::Invalid)?;
    *cursor = end;
    Ok(bytes)
}

fn take_u8(input: &[u8], cursor: &mut usize) -> Result<u8, NetworkConfigError> {
    Ok(take_bytes(input, cursor, 1)?[0])
}

fn take_u32(input: &[u8], cursor: &mut usize) -> Result<u32, NetworkConfigError> {
    Ok(u32::from_le_bytes(take_bytes(input, cursor, 4)?.try_into().unwrap_or([0; 4])))
}

fn take_u64(input: &[u8], cursor: &mut usize) -> Result<u64, NetworkConfigError> {
    Ok(u64::from_le_bytes(take_bytes(input, cursor, 8)?.try_into().unwrap_or([0; 8])))
}

fn skip_name(packet: &[u8], cursor: &mut usize) -> Result<(), DnsError> {
    let mut position = *cursor;
    let mut jumps = 0;
    loop {
        let length = *packet.get(position).ok_or(DnsError::InvalidPacket)?;
        if length & 0xc0 == 0xc0 {
            if packet.get(position + 1).is_none() {
                return Err(DnsError::InvalidPacket)
            }
            if jumps == 0 {
                *cursor = position + 2;
            }
            jumps += 1;
            if jumps > 16 {
                return Err(DnsError::InvalidPacket)
            }
            position = ((length as usize & 0x3f) << 8) | packet[position + 1] as usize;
            continue
        }
        if length == 0 {
            if jumps == 0 {
                *cursor = position + 1;
            }
            return Ok(())
        }
        if length > 63 {
            return Err(DnsError::InvalidPacket)
        }
        position = position
            .checked_add(length as usize + 1)
            .ok_or(DnsError::InvalidPacket)?;
        if position > packet.len() {
            return Err(DnsError::InvalidPacket)
        }
    }
}
