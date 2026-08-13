use synos_status::{IntoStatus, Severity, Status, facility};

use crate::ThermalTripPoints;

const SDT_HEADER_BYTES: usize = 36;
const RSDP_V1_BYTES: usize = 20;
const RSDP_V2_BYTES: usize = 36;
const MAX_ACPI_TABLE_BYTES: u32 = 16 * 1024 * 1024;
const PM1_STATUS_POWER_BUTTON: u16 = 1 << 8;
const PM1_STATUS_SLEEP_BUTTON: u16 = 1 << 9;
const PM1_STATUS_RTC: u16 = 1 << 10;
const PM1_STATUS_PCIE_WAKE: u16 = 1 << 14;
const PM1_STATUS_WAKE: u16 = 1 << 15;
const PM1_CONTROL_SCI_ENABLE: u64 = 1;
const PM1_CONTROL_SLEEP_TYPE_MASK: u64 = 7 << 10;
const PM1_CONTROL_SLEEP_ENABLE: u64 = 1 << 13;
const MAX_BATTERIES: usize = 4;
const MAX_METHOD_BYTES: usize = 512;

pub trait AcpiMemory {
    fn read(
        &self,
        physical_address: u64,
        destination: &mut [u8],
    ) -> Result<(), AcpiError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressSpace {
    SystemMemory,
    SystemIo,
    PciConfiguration,
    EmbeddedController,
    SmBus,
    PlatformCommunications,
    FunctionalFixedHardware,
    Other(u8),
}

impl AddressSpace {
    const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::SystemMemory,
            1 => Self::SystemIo,
            2 => Self::PciConfiguration,
            3 => Self::EmbeddedController,
            4 => Self::SmBus,
            0x0a => Self::PlatformCommunications,
            0x7f => Self::FunctionalFixedHardware,
            value => Self::Other(value),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenericAddress {
    pub address_space: AddressSpace,
    pub bit_width: u8,
    pub bit_offset: u8,
    pub access_size: u8,
    pub address: u64,
}

impl GenericAddress {
    pub const fn is_present(self) -> bool {
        self.address != 0 && self.bit_width != 0
    }

    fn byte_offset(self, bytes: u64, bit_width: u8) -> Option<Self> {
        Some(Self {
            address: self.address.checked_add(bytes)?,
            bit_width,
            ..self
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResetRegister {
    pub register: GenericAddress,
    pub value: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedHardware {
    pub pm1a_event: Option<GenericAddress>,
    pub pm1b_event: Option<GenericAddress>,
    pub pm1a_control: Option<GenericAddress>,
    pub pm1b_control: Option<GenericAddress>,
    pub pm1_event_bytes: u8,
    pub smi_command_port: u32,
    pub acpi_enable_value: u8,
    pub reset: Option<ResetRegister>,
    pub reduced_hardware: bool,
    pub sleep_control: Option<GenericAddress>,
    pub sleep_status: Option<GenericAddress>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SleepTypes {
    pub type_a: u8,
    pub type_b: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcpiPlatform {
    pub revision: u8,
    pub fixed: FixedHardware,
    pub suspend: Option<SleepTypes>,
    pub soft_off: Option<SleepTypes>,
    pub thermal: ThermalTripPoints,
    battery: BatterySupport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BatterySupport {
    aml_start: u64,
    aml_length: u32,
    ac_adapter_status: Option<u32>,
    batteries: [Option<BatteryMethods>; MAX_BATTERIES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BatteryMethods {
    status: Option<u32>,
    info: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerSource {
    Ac,
    Battery,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BatteryState {
    Charging,
    Discharging,
    Critical,
    Idle,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatteryStatus {
    pub state: BatteryState,
    pub state_flags: u32,
    pub present_rate: Option<u32>,
    pub remaining_capacity: Option<u32>,
    pub voltage: Option<u32>,
    pub capacity_percent: Option<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatteryReport {
    pub source: PowerSource,
    pub ac_online: Option<bool>,
    pub battery_count: u8,
    pub primary: Option<BatteryStatus>,
}

impl BatteryReport {
    pub const UNKNOWN: Self = Self {
        source: PowerSource::Unknown,
        ac_online: None,
        battery_count: 0,
        primary: None,
    };
}

impl AcpiPlatform {
    pub fn discover(
        memory: &impl AcpiMemory,
        rsdp_address: u64,
    ) -> Result<Self, AcpiError> {
        if rsdp_address == 0 {
            return Err(AcpiError::MissingRsdp)
        }
        let mut rsdp = [0; RSDP_V2_BYTES];
        memory.read(rsdp_address, &mut rsdp[..RSDP_V1_BYTES])?;
        if &rsdp[..8] != b"RSD PTR " || checksum(&rsdp[..RSDP_V1_BYTES]) != 0 {
            return Err(AcpiError::BadChecksum)
        }

        let revision = rsdp[15];
        let (root, entry_bytes, signature) = if revision >= 2 {
            memory.read(
                rsdp_address + RSDP_V1_BYTES as u64,
                &mut rsdp[RSDP_V1_BYTES..],
            )?;
            let length = read_u32(&rsdp, 20)?;
            if length < RSDP_V2_BYTES as u32
                || length > 4096
                || checksum_range(memory, rsdp_address, length)? != 0
            {
                return Err(AcpiError::BadChecksum)
            }
            let xsdt = read_u64(&rsdp, 24)?;
            if xsdt != 0 {
                (xsdt, 8, *b"XSDT")
            } else {
                (read_u32(&rsdp, 16)? as u64, 4, *b"RSDT")
            }
        } else {
            (read_u32(&rsdp, 16)? as u64, 4, *b"RSDT")
        };

        let root_header = read_sdt_header(memory, root)?;
        if root_header.signature != signature
            || (root_header.length as usize - SDT_HEADER_BYTES) % entry_bytes != 0
        {
            return Err(AcpiError::MalformedTable)
        }
        validate_sdt(memory, root, root_header)?;

        let entries = (root_header.length as usize - SDT_HEADER_BYTES) / entry_bytes;
        let mut fadt_address = None;
        for index in 0..entries {
            let address = if entry_bytes == 8 {
                read_memory_u64(
                    memory,
                    root + SDT_HEADER_BYTES as u64 + index as u64 * 8,
                )?
            } else {
                read_memory_u32(
                    memory,
                    root + SDT_HEADER_BYTES as u64 + index as u64 * 4,
                )? as u64
            };
            if address == 0 {
                continue
            }
            let header = read_sdt_header(memory, address)?;
            if &header.signature == b"FACP" {
                validate_sdt(memory, address, header)?;
                fadt_address = Some((address, header));
                break
            }
        }
        let (fadt_address, fadt) = fadt_address.ok_or(AcpiError::MissingFadt)?;
        parse_fadt(memory, revision, fadt_address, fadt)
    }

    pub fn battery_report<M: AcpiMemory>(&self, memory: &M) -> BatteryReport {
        let ac_online = self
            .battery
            .ac_adapter_status
            .and_then(|offset| read_method_integer(memory, self.battery, offset).ok().flatten())
            .map(|value| value != 0);
        let mut primary = None;
        let mut battery_count = 0;

        for methods in self.battery.batteries.iter().flatten() {
            if methods.status.is_none() && methods.info.is_none() {
                continue
            }
            battery_count += 1;
            if primary.is_none() {
                primary = read_battery_status(memory, self.battery, *methods).ok().flatten()
            }
        }

        let source = match ac_online {
            Some(true) => PowerSource::Ac,
            Some(false) if battery_count != 0 => PowerSource::Battery,
            Some(false) => PowerSource::Unknown,
            None if battery_count != 0 => PowerSource::Battery,
            None => PowerSource::Unknown,
        };
        BatteryReport {
            source,
            ac_online,
            battery_count,
            primary,
        }
    }
}

pub trait PowerIo {
    fn read(&mut self, register: GenericAddress) -> Result<u64, AcpiError>;

    fn write(
        &mut self,
        register: GenericAddress,
        value: u64,
    ) -> Result<(), AcpiError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerState {
    Suspend,
    SoftOff,
    Reboot,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedEvent {
    PowerButton,
    SleepButton,
    RtcAlarm,
    PcieWake,
    Wake,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedEvents(u16);

impl FixedEvents {
    pub const NONE: Self = Self(0);

    pub const fn contains(self, event: FixedEvent) -> bool {
        self.0 & event.mask() != 0
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

impl FixedEvent {
    const fn mask(self) -> u16 {
        match self {
            FixedEvent::PowerButton => PM1_STATUS_POWER_BUTTON,
            FixedEvent::SleepButton => PM1_STATUS_SLEEP_BUTTON,
            FixedEvent::RtcAlarm => PM1_STATUS_RTC,
            FixedEvent::PcieWake => PM1_STATUS_PCIE_WAKE,
            FixedEvent::Wake => PM1_STATUS_WAKE,
        }
    }
}

pub struct PowerController<Io> {
    platform: AcpiPlatform,
    io: Io,
}

impl<Io: PowerIo> PowerController<Io> {
    pub const fn new(platform: AcpiPlatform, io: Io) -> Self {
        Self { platform, io }
    }

    pub fn enable_acpi(&mut self, mut spin_limit: usize) -> Result<(), AcpiError> {
        if self.platform.fixed.reduced_hardware {
            return Ok(())
        }
        let control = self
            .platform
            .fixed
            .pm1a_control
            .ok_or(AcpiError::Unsupported)?;
        if self.io.read(control)? & PM1_CONTROL_SCI_ENABLE != 0 {
            return Ok(())
        }
        if self.platform.fixed.smi_command_port == 0
            || self.platform.fixed.acpi_enable_value == 0
        {
            return Err(AcpiError::Unsupported)
        }
        self.io.write(
            GenericAddress {
                address_space: AddressSpace::SystemIo,
                bit_width: 8,
                bit_offset: 0,
                access_size: 1,
                address: self.platform.fixed.smi_command_port as u64,
            },
            self.platform.fixed.acpi_enable_value as u64,
        )?;
        loop {
            if self.io.read(control)? & PM1_CONTROL_SCI_ENABLE != 0 {
                return Ok(())
            }
            if spin_limit == 0 {
                return Err(AcpiError::TimedOut)
            }
            spin_limit -= 1;
            core::hint::spin_loop()
        }
    }

    pub fn poll_fixed_events(&mut self) -> Result<FixedEvents, AcpiError> {
        let mut asserted = 0_u16;
        for event in [
            self.platform.fixed.pm1a_event,
            self.platform.fixed.pm1b_event,
        ]
        .into_iter()
        .flatten()
        {
            let half = self.platform.fixed.pm1_event_bytes / 2;
            if half == 0 {
                return Err(AcpiError::MalformedTable)
            }
            let width = half.saturating_mul(8);
            let enable = event
                .byte_offset(half as u64, width)
                .ok_or(AcpiError::MalformedTable)?;
            let status = self.io.read(GenericAddress {
                bit_width: width,
                ..event
            })? as u16;
            let enabled = self.io.read(enable)? as u16;
            let active = status & enabled;
            asserted |= active;
            if active != 0 {
                self.io.write(
                    GenericAddress {
                        bit_width: width,
                        ..event
                    },
                    active as u64,
                )?
            }
        }
        Ok(FixedEvents(asserted))
    }

    pub fn set_fixed_event_enabled(
        &mut self,
        event: FixedEvent,
        enabled: bool,
    ) -> Result<(), AcpiError> {
        let mut configured = false;
        for block in [
            self.platform.fixed.pm1a_event,
            self.platform.fixed.pm1b_event,
        ]
        .into_iter()
        .flatten()
        {
            let half = self.platform.fixed.pm1_event_bytes / 2;
            if half == 0 {
                return Err(AcpiError::MalformedTable)
            }
            let register = block
                .byte_offset(half as u64, half.saturating_mul(8))
                .ok_or(AcpiError::MalformedTable)?;
            let previous = self.io.read(register)?;
            let mask = event.mask() as u64;
            self.io.write(
                register,
                if enabled {
                    previous | mask
                } else {
                    previous & !mask
                },
            )?;
            configured = true
        }
        if configured {
            Ok(())
        } else {
            Err(AcpiError::Unsupported)
        }
    }

    pub fn request(&mut self, state: PowerState) -> Result<(), AcpiError> {
        match state {
            PowerState::Suspend => self.suspend(),
            PowerState::SoftOff => self.soft_off(),
            PowerState::Reboot => {
                let reset = self.platform.fixed.reset.ok_or(AcpiError::Unsupported)?;
                self.io.write(reset.register, reset.value as u64)
            }
        }
    }

    pub fn into_io(self) -> Io {
        self.io
    }

    fn soft_off(&mut self) -> Result<(), AcpiError> {
        let sleep = self.platform.soft_off.ok_or(AcpiError::Unsupported)?;
        self.write_sleep_state(sleep)
    }

    fn suspend(&mut self) -> Result<(), AcpiError> {
        let sleep = self.platform.suspend.ok_or(AcpiError::Unsupported)?;
        self.write_sleep_state(sleep)
    }

    fn write_sleep_state(&mut self, sleep: SleepTypes) -> Result<(), AcpiError> {
        if self.platform.fixed.reduced_hardware {
            let register = self
                .platform
                .fixed
                .sleep_control
                .ok_or(AcpiError::Unsupported)?;
            return self.io.write(
                register,
                ((sleep.type_a as u64 & 7) << 2) | 1 << 5,
            )
        }
        self.write_sleep_control(
            self.platform.fixed.pm1a_control,
            sleep.type_a,
        )?;
        if self.platform.fixed.pm1b_control.is_some() {
            self.write_sleep_control(
                self.platform.fixed.pm1b_control,
                sleep.type_b,
            )?
        }
        Ok(())
    }

    fn write_sleep_control(
        &mut self,
        register: Option<GenericAddress>,
        sleep_type: u8,
    ) -> Result<(), AcpiError> {
        let register = register.ok_or(AcpiError::Unsupported)?;
        let previous = self.io.read(register)?;
        let value = previous & !PM1_CONTROL_SLEEP_TYPE_MASK
            | ((sleep_type as u64 & 7) << 10)
            | PM1_CONTROL_SLEEP_ENABLE;
        self.io.write(register, value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcpiError {
    BadChecksum,
    InvalidAddress,
    InvalidThermalPolicy,
    MalformedAml,
    MalformedTable,
    MissingFadt,
    MissingRsdp,
    TimedOut,
    Unsupported,
}

impl IntoStatus for AcpiError {
    fn status(self) -> Status {
        match self {
            Self::MissingFadt | Self::MissingRsdp => Status::NOT_FOUND,
            Self::TimedOut => Status::BUSY,
            Self::BadChecksum | Self::MalformedAml | Self::MalformedTable => {
                Status::CORRUPT
            }
            Self::InvalidAddress | Self::InvalidThermalPolicy | Self::Unsupported => {
                Status::new(Severity::Error, facility::DRIVER, 30, 0)
                    .expect("valid power status")
            }
        }
    }
}

#[derive(Clone, Copy)]
struct SdtHeader {
    signature: [u8; 4],
    length: u32,
}

fn read_sdt_header(
    memory: &impl AcpiMemory,
    address: u64,
) -> Result<SdtHeader, AcpiError> {
    let mut bytes = [0; SDT_HEADER_BYTES];
    memory.read(address, &mut bytes)?;
    let mut signature = [0; 4];
    signature.copy_from_slice(&bytes[..4]);
    let length = read_u32(&bytes, 4)?;
    if length < SDT_HEADER_BYTES as u32 || length > MAX_ACPI_TABLE_BYTES {
        return Err(AcpiError::MalformedTable)
    }
    Ok(SdtHeader { signature, length })
}

fn validate_sdt(
    memory: &impl AcpiMemory,
    address: u64,
    header: SdtHeader,
) -> Result<(), AcpiError> {
    if checksum_range(memory, address, header.length)? != 0 {
        Err(AcpiError::BadChecksum)
    } else {
        Ok(())
    }
}

fn parse_fadt(
    memory: &impl AcpiMemory,
    revision: u8,
    address: u64,
    header: SdtHeader,
) -> Result<AcpiPlatform, AcpiError> {
    if header.length < 90 {
        return Err(AcpiError::MalformedTable)
    }
    let smi_command_port = read_memory_u32(memory, address + 48)?;
    let acpi_enable_value = read_memory_u8(memory, address + 52)?;
    let pm1_event_bytes = read_memory_u8(memory, address + 88)?;
    let pm1_control_bytes = read_memory_u8(memory, address + 89)?;

    let legacy_event_width = pm1_event_bytes.saturating_mul(8);
    let legacy_control_width = pm1_control_bytes.saturating_mul(8);
    let mut fixed = FixedHardware {
        pm1a_event: gas_from_legacy(
            read_memory_u32(memory, address + 56)?,
            legacy_event_width,
        ),
        pm1b_event: gas_from_legacy(
            read_memory_u32(memory, address + 60)?,
            legacy_event_width,
        ),
        pm1a_control: gas_from_legacy(
            read_memory_u32(memory, address + 64)?,
            legacy_control_width,
        ),
        pm1b_control: gas_from_legacy(
            read_memory_u32(memory, address + 68)?,
            legacy_control_width,
        ),
        pm1_event_bytes,
        smi_command_port,
        acpi_enable_value,
        reset: None,
        reduced_hardware: false,
        sleep_control: None,
        sleep_status: None,
    };

    let mut dsdt = read_memory_u32(memory, address + 40)? as u64;
    if header.length >= 129 {
        let register = read_gas(memory, address + 116)?;
        let value = read_memory_u8(memory, address + 128)?;
        if register.is_present() {
            fixed.reset = Some(ResetRegister { register, value })
        }
    }
    if header.length >= 116 {
        let flags = read_memory_u32(memory, address + 112)?;
        fixed.reduced_hardware = flags & (1 << 20) != 0
    }
    if header.length >= 148 {
        let extended_dsdt = read_memory_u64(memory, address + 140)?;
        if extended_dsdt != 0 {
            dsdt = extended_dsdt
        }
    }
    if header.length >= 196 {
        fixed.pm1a_event = prefer_gas(
            read_gas(memory, address + 148)?,
            fixed.pm1a_event,
        );
        fixed.pm1b_event = prefer_gas(
            read_gas(memory, address + 160)?,
            fixed.pm1b_event,
        );
        fixed.pm1a_control = prefer_gas(
            read_gas(memory, address + 172)?,
            fixed.pm1a_control,
        );
        fixed.pm1b_control = prefer_gas(
            read_gas(memory, address + 184)?,
            fixed.pm1b_control,
        )
    }
    if header.length >= 268 {
        fixed.sleep_control = prefer_gas(read_gas(memory, address + 244)?, None);
        fixed.sleep_status = prefer_gas(read_gas(memory, address + 256)?, None)
    }

    let (soft_off, thermal, battery) = if dsdt == 0 {
        (None, ThermalTripPoints::NONE, BatterySupport::NONE)
    } else {
        let dsdt_header = read_sdt_header(memory, dsdt)?;
        if &dsdt_header.signature != b"DSDT" {
            return Err(AcpiError::MalformedTable)
        }
        validate_sdt(memory, dsdt, dsdt_header)?;
        let aml_start = dsdt + SDT_HEADER_BYTES as u64;
        let aml_length = dsdt_header.length - SDT_HEADER_BYTES as u32;
        let battery = parse_battery_support(memory, aml_start, aml_length)?;
        (
            parse_sleep_types(memory, aml_start, aml_length, b"_S5_")?,
            ThermalTripPoints {
                passive_deci_kelvin: parse_named_integer(
                    memory,
                    aml_start,
                    aml_length,
                    b"_PSV",
                )?
                .map(|value| value as u32),
                hot_deci_kelvin: parse_named_integer(
                    memory,
                    aml_start,
                    aml_length,
                    b"_HOT",
                )?
                .map(|value| value as u32),
                critical_deci_kelvin: parse_named_integer(
                    memory,
                    aml_start,
                    aml_length,
                    b"_CRT",
                )?
                .map(|value| value as u32),
            },
            battery,
        )
    };
    let suspend = if dsdt == 0 {
        None
    } else {
        let dsdt_header = read_sdt_header(memory, dsdt)?;
        if &dsdt_header.signature != b"DSDT" {
            return Err(AcpiError::MalformedTable)
        }
        validate_sdt(memory, dsdt, dsdt_header)?;
        let aml_start = dsdt + SDT_HEADER_BYTES as u64;
        let aml_length = dsdt_header.length - SDT_HEADER_BYTES as u32;
        parse_sleep_types(memory, aml_start, aml_length, b"_S3_")?
    };

    Ok(AcpiPlatform {
        revision,
        fixed,
        suspend,
        soft_off,
        thermal,
        battery,
    })
}

impl BatterySupport {
    const NONE: Self = Self {
        aml_start: 0,
        aml_length: 0,
        ac_adapter_status: None,
        batteries: [None; MAX_BATTERIES],
    };
}

fn parse_battery_support(
    memory: &impl AcpiMemory,
    aml_start: u64,
    aml_length: u32,
) -> Result<BatterySupport, AcpiError> {
    let ac_adapter_status = find_name(memory, aml_start, aml_length, b"_PSR")?
        .map(|offset| offset as u32);
    let status = find_name(memory, aml_start, aml_length, b"_BST")?
        .map(|offset| offset as u32);
    let info = find_name(memory, aml_start, aml_length, b"_BIF")?
        .map(|offset| offset as u32);
    let mut batteries = [None; MAX_BATTERIES];
    if status.is_some() || info.is_some() {
        batteries[0] = Some(BatteryMethods { status, info })
    }
    Ok(BatterySupport {
        aml_start,
        aml_length,
        ac_adapter_status,
        batteries,
    })
}

fn read_battery_status(
    memory: &impl AcpiMemory,
    support: BatterySupport,
    methods: BatteryMethods,
) -> Result<Option<BatteryStatus>, AcpiError> {
    let Some(status_offset) = methods.status else {
        return Ok(None)
    };
    let Some(values) = read_method_package(memory, support, status_offset)? else {
        return Ok(None)
    };
    if values.len < 4 {
        return Err(AcpiError::MalformedAml)
    }
    let state_flags = values.values[0] as u32;
    let remaining_capacity = acpi_value(values.values[2]);
    let last_full_capacity = methods
        .info
        .and_then(|offset| read_method_package(memory, support, offset).ok().flatten())
        .and_then(|values| values.values.get(2).copied())
        .and_then(acpi_value);
    let capacity_percent = last_full_capacity
        .filter(|capacity| *capacity != 0)
        .and_then(|capacity| remaining_capacity.map(|remaining| {
            ((remaining as u64)
                .saturating_mul(100)
                .checked_div(capacity as u64)
                .unwrap_or(0)
                .min(100)) as u8
        }));
    Ok(Some(BatteryStatus {
        state: battery_state(state_flags),
        state_flags,
        present_rate: acpi_value(values.values[1]),
        remaining_capacity,
        voltage: acpi_value(values.values[3]),
        capacity_percent,
    }))
}

#[derive(Clone, Copy)]
struct AmlPackage {
    values: [u64; 13],
    len: usize,
}

fn read_method_package(
    memory: &impl AcpiMemory,
    support: BatterySupport,
    offset: u32,
) -> Result<Option<AmlPackage>, AcpiError> {
    let bytes = read_method_bytes(memory, support, offset)?;
    if is_name_object(memory, support, offset)? && bytes.get(4).copied() == Some(0x12) {
        return parse_aml_package(&bytes[5..]).map(Some)
    }
    let Some(return_position) = bytes.iter().position(|byte| *byte == 0xa4) else {
        return Ok(None)
    };
    let Some(package) = bytes.get(return_position + 1..) else {
        return Err(AcpiError::MalformedAml)
    };
    if package.first().copied() != Some(0x12) {
        return Ok(None)
    }
    parse_aml_package(&package[1..]).map(Some)
}

fn read_method_integer(
    memory: &impl AcpiMemory,
    support: BatterySupport,
    offset: u32,
) -> Result<Option<u64>, AcpiError> {
    let bytes = read_method_bytes(memory, support, offset)?;
    if is_name_object(memory, support, offset)?
        && let Ok((value, _)) = parse_aml_integer(&bytes[4..])
    {
        return Ok(Some(value))
    }
    let Some(return_position) = bytes.iter().position(|byte| *byte == 0xa4) else {
        return Ok(None)
    };
    parse_aml_integer(
        bytes
            .get(return_position + 1..)
            .ok_or(AcpiError::MalformedAml)?,
    )
    .map(|(value, _)| Some(value))
}

fn read_method_bytes(
    memory: &impl AcpiMemory,
    support: BatterySupport,
    offset: u32,
) -> Result<[u8; MAX_METHOD_BYTES], AcpiError> {
    let offset = offset as u64;
    if support.aml_start == 0 || offset >= support.aml_length as u64 {
        return Err(AcpiError::MalformedAml)
    }
    let amount = (support.aml_length as u64 - offset).min(MAX_METHOD_BYTES as u64) as usize;
    let mut bytes = [0; MAX_METHOD_BYTES];
    memory.read(support.aml_start + offset, &mut bytes[..amount])?;
    Ok(bytes)
}

fn is_name_object(
    memory: &impl AcpiMemory,
    support: BatterySupport,
    offset: u32,
) -> Result<bool, AcpiError> {
    if offset == 0 {
        return Ok(false)
    }
    let mut opcode = [0];
    memory.read(support.aml_start + offset as u64 - 1, &mut opcode)?;
    Ok(opcode[0] == 0x08)
}

fn parse_aml_package(bytes: &[u8]) -> Result<AmlPackage, AcpiError> {
    let (package_length, length_bytes) = parse_package_length(bytes)?;
    let package_end = package_length;
    if package_end > bytes.len()
        || package_end <= length_bytes
        || length_bytes >= bytes.len()
    {
        return Err(AcpiError::MalformedAml)
    }
    let count_position = length_bytes;
    let count = *bytes.get(count_position).ok_or(AcpiError::MalformedAml)? as usize;
    let mut values = [0; 13];
    let mut len = 0;
    let mut cursor = count_position + 1;
    while len < count && len < values.len() && cursor < package_end {
        let Ok((value, used)) = parse_aml_integer(&bytes[cursor..package_end]) else {
            if len >= 3 {
                break
            }
            return Err(AcpiError::MalformedAml)
        };
        values[len] = value;
        len += 1;
        cursor += used;
    }
    if len < count.min(3) {
        return Err(AcpiError::MalformedAml)
    }
    Ok(AmlPackage { values, len })
}

fn acpi_value(value: u64) -> Option<u32> {
    if value == u32::MAX as u64 {
        None
    } else {
        u32::try_from(value).ok()
    }
}

fn battery_state(flags: u32) -> BatteryState {
    if flags & 4 != 0 {
        BatteryState::Critical
    } else if flags & 1 != 0 {
        BatteryState::Charging
    } else if flags & 2 != 0 {
        BatteryState::Discharging
    } else if flags == 0 {
        BatteryState::Idle
    } else {
        BatteryState::Unknown
    }
}

fn prefer_gas(
    candidate: GenericAddress,
    fallback: Option<GenericAddress>,
) -> Option<GenericAddress> {
    if candidate.is_present() {
        Some(candidate)
    } else {
        fallback
    }
}

fn gas_from_legacy(address: u32, width: u8) -> Option<GenericAddress> {
    if address == 0 || width == 0 {
        None
    } else {
        Some(GenericAddress {
            address_space: AddressSpace::SystemIo,
            bit_width: width,
            bit_offset: 0,
            access_size: match width {
                0..=8 => 1,
                9..=16 => 2,
                17..=32 => 3,
                _ => 4,
            },
            address: address as u64,
        })
    }
}

fn read_gas(
    memory: &impl AcpiMemory,
    address: u64,
) -> Result<GenericAddress, AcpiError> {
    let mut bytes = [0; 12];
    memory.read(address, &mut bytes)?;
    Ok(GenericAddress {
        address_space: AddressSpace::from_raw(bytes[0]),
        bit_width: bytes[1],
        bit_offset: bytes[2],
        access_size: bytes[3],
        address: read_u64(&bytes, 4)?,
    })
}

fn parse_sleep_types(
    memory: &impl AcpiMemory,
    aml_start: u64,
    aml_length: u32,
    name: &[u8; 4],
) -> Result<Option<SleepTypes>, AcpiError> {
    let Some(position) = find_name(memory, aml_start, aml_length, name)? else {
        return Ok(None)
    };
    let mut bytes = [0; 40];
    let available = (aml_length as u64 - position - 4).min(bytes.len() as u64) as usize;
    memory.read(aml_start + position + 4, &mut bytes[..available])?;
    let Some(package) = bytes[..available].iter().position(|byte| *byte == 0x12) else {
        return Err(AcpiError::MalformedAml)
    };
    let (_, length_bytes) = parse_package_length(&bytes[package + 1..available])?;
    let mut cursor = package + 1 + length_bytes;
    if cursor >= available {
        return Err(AcpiError::MalformedAml)
    }
    cursor += 1;
    let (type_a, used_a) = parse_aml_integer(&bytes[cursor..available])?;
    cursor += used_a;
    let (type_b, _) = parse_aml_integer(&bytes[cursor..available])?;
    if type_a > 7 || type_b > 7 {
        return Err(AcpiError::MalformedAml)
    }
    Ok(Some(SleepTypes {
        type_a: type_a as u8,
        type_b: type_b as u8,
    }))
}

fn parse_named_integer(
    memory: &impl AcpiMemory,
    aml_start: u64,
    aml_length: u32,
    name: &[u8; 4],
) -> Result<Option<u64>, AcpiError> {
    let Some(position) = find_name(memory, aml_start, aml_length, name)? else {
        return Ok(None)
    };
    let mut bytes = [0; 10];
    let available = (aml_length as u64 - position - 4).min(bytes.len() as u64) as usize;
    memory.read(aml_start + position + 4, &mut bytes[..available])?;
    parse_aml_integer(&bytes[..available])
        .map(|(value, _)| Some(value))
        .or(Ok(None))
}

fn find_name(
    memory: &impl AcpiMemory,
    start: u64,
    length: u32,
    name: &[u8; 4],
) -> Result<Option<u64>, AcpiError> {
    let mut window = [0; 256];
    let mut overlap = [0; 3];
    let mut overlap_length = 0;
    let mut offset = 0_u64;
    while offset < length as u64 {
        let amount = (length as u64 - offset).min(window.len() as u64) as usize;
        memory.read(start + offset, &mut window[..amount])?;
        let mut combined = [0; 259];
        combined[..overlap_length].copy_from_slice(&overlap[..overlap_length]);
        combined[overlap_length..overlap_length + amount].copy_from_slice(&window[..amount]);
        let total = overlap_length + amount;
        if let Some(found) = combined[..total]
            .windows(4)
            .position(|candidate| candidate == name)
        {
            return Ok(Some(
                offset.saturating_sub(overlap_length as u64) + found as u64,
            ))
        }
        overlap_length = total.min(3);
        overlap[..overlap_length]
            .copy_from_slice(&combined[total - overlap_length..total]);
        offset += amount as u64
    }
    Ok(None)
}

fn parse_package_length(bytes: &[u8]) -> Result<(usize, usize), AcpiError> {
    let lead = *bytes.first().ok_or(AcpiError::MalformedAml)?;
    let following = (lead >> 6) as usize;
    if bytes.len() < following + 1 {
        return Err(AcpiError::MalformedAml)
    }
    let mut length = if following == 0 {
        (lead & 0x3f) as usize
    } else {
        (lead & 0x0f) as usize
    };
    for index in 0..following {
        length |= (bytes[index + 1] as usize) << (4 + index * 8)
    }
    Ok((length, following + 1))
}

fn parse_aml_integer(bytes: &[u8]) -> Result<(u64, usize), AcpiError> {
    let opcode = *bytes.first().ok_or(AcpiError::MalformedAml)?;
    match opcode {
        0x00 => Ok((0, 1)),
        0x01 => Ok((1, 1)),
        0xff => Ok((u64::MAX, 1)),
        0x0a if bytes.len() >= 2 => Ok((bytes[1] as u64, 2)),
        0x0b if bytes.len() >= 3 => Ok((u16::from_le_bytes([bytes[1], bytes[2]]) as u64, 3)),
        0x0c if bytes.len() >= 5 => Ok((
            u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as u64,
            5,
        )),
        0x0e if bytes.len() >= 9 => Ok((
            u64::from_le_bytes([
                bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                bytes[8],
            ]),
            9,
        )),
        _ => Err(AcpiError::MalformedAml),
    }
}

fn checksum_range(
    memory: &impl AcpiMemory,
    address: u64,
    length: u32,
) -> Result<u8, AcpiError> {
    let mut buffer = [0; 256];
    let mut sum = 0_u8;
    let mut offset = 0_u64;
    while offset < length as u64 {
        let amount = (length as u64 - offset).min(buffer.len() as u64) as usize;
        memory.read(address + offset, &mut buffer[..amount])?;
        for byte in &buffer[..amount] {
            sum = sum.wrapping_add(*byte)
        }
        offset += amount as u64
    }
    Ok(sum)
}

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte))
}

fn read_memory_u8(
    memory: &impl AcpiMemory,
    address: u64,
) -> Result<u8, AcpiError> {
    let mut bytes = [0];
    memory.read(address, &mut bytes)?;
    Ok(bytes[0])
}

fn read_memory_u32(
    memory: &impl AcpiMemory,
    address: u64,
) -> Result<u32, AcpiError> {
    let mut bytes = [0; 4];
    memory.read(address, &mut bytes)?;
    read_u32(&bytes, 0)
}

fn read_memory_u64(
    memory: &impl AcpiMemory,
    address: u64,
) -> Result<u64, AcpiError> {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes)?;
    read_u64(&bytes, 0)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, AcpiError> {
    let source = bytes
        .get(offset..offset + 4)
        .ok_or(AcpiError::MalformedTable)?;
    Ok(u32::from_le_bytes([
        source[0], source[1], source[2], source[3],
    ]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, AcpiError> {
    let source = bytes
        .get(offset..offset + 8)
        .ok_or(AcpiError::MalformedTable)?;
    Ok(u64::from_le_bytes([
        source[0], source[1], source[2], source[3], source[4], source[5], source[6],
        source[7],
    ]))
}
