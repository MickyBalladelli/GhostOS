use crate::AcpiError;
use ghostos_status::Status;

pub const MAX_THERMAL_EVENTS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThermalTripPoints {
    /// ACPI temperatures use tenths of one Kelvin.
    pub passive_deci_kelvin: Option<u32>,
    pub hot_deci_kelvin: Option<u32>,
    pub critical_deci_kelvin: Option<u32>,
}

impl ThermalTripPoints {
    pub const NONE: Self = Self {
        passive_deci_kelvin: None,
        hot_deci_kelvin: None,
        critical_deci_kelvin: None,
    };

    pub const fn validate(self) -> Result<Self, AcpiError> {
        if let (Some(passive), Some(hot)) =
            (self.passive_deci_kelvin, self.hot_deci_kelvin)
            && passive >= hot
        {
            return Err(AcpiError::InvalidThermalPolicy)
        }
        if let (Some(hot), Some(critical)) =
            (self.hot_deci_kelvin, self.critical_deci_kelvin)
            && hot >= critical
        {
            return Err(AcpiError::InvalidThermalPolicy)
        }
        if let (Some(passive), Some(critical)) =
            (self.passive_deci_kelvin, self.critical_deci_kelvin)
            && passive >= critical
        {
            return Err(AcpiError::InvalidThermalPolicy)
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThermalReading {
    pub temperature_deci_kelvin: u32,
    pub timestamp_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThermalAction {
    Normal,
    Throttle { percent: u8 },
    EmergencyShutdown,
}

impl ThermalAction {
    pub const fn throttle_percent(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Throttle { percent } => percent,
            Self::EmergencyShutdown => 100,
        }
    }

    pub const fn code(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Throttle { .. } => 1,
            Self::EmergencyShutdown => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ThermalEventKind {
    ThrottleStarted = 1,
    ThrottleChanged = 2,
    Recovered = 3,
    Critical = 4,
}

impl ThermalEventKind {
    pub const fn code(self) -> u8 {
        self as u8
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ThermalEvent {
    pub kind: ThermalEventKind,
    pub reading: ThermalReading,
    pub previous_action: ThermalAction,
    pub action: ThermalAction,
}

pub struct ThermalEventLog<const CAPACITY: usize> {
    events: [Option<ThermalEvent>; CAPACITY],
    state: crate::native::Log,
}

impl<const CAPACITY: usize> ThermalEventLog<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            events: [None; CAPACITY],
            state: crate::native::Log::EMPTY,
        }
    }

    pub const fn len(&self) -> usize {
        self.state.len
    }

    pub const fn dropped(&self) -> u64 {
        self.state.dropped
    }

    fn push(&mut self, event: ThermalEvent) {
        if let Some(slot) = self.state.push(CAPACITY) {
            self.events[slot] = Some(event)
        }
    }

    pub fn drain(&mut self, destination: &mut [Option<ThermalEvent>]) -> usize {
        let (mut index, count) = self.state.drain(CAPACITY, destination.len());
        for slot in destination.iter_mut().take(count) {
            *slot = self.events[index].take();
            index = if index + 1 == CAPACITY { 0 } else { index + 1 };
        }
        count
    }
}

impl<const CAPACITY: usize> Default for ThermalEventLog<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Applies ACPI trip points with hysteresis so fan noise and CPU frequency do
/// not oscillate around one sensor value.
pub struct ThermalManager {
    trips: ThermalTripPoints,
    hysteresis_deci_kelvin: u32,
    action: ThermalAction,
    last_reading: Option<ThermalReading>,
    events: ThermalEventLog<MAX_THERMAL_EVENTS>,
}

pub trait ThermalSensor {
    fn read(&mut self) -> Result<ThermalReading, Status>;
}

pub trait ThermalActuator {
    /// Zero means unthrottled. One hundred means fully throttled.
    fn set_throttle_percent(&mut self, percent: u8) -> Result<(), Status>;
}

pub struct ThermalSupervisor<Sensor, Actuator> {
    manager: ThermalManager,
    sensor: Sensor,
    actuator: Actuator,
}

impl<Sensor: ThermalSensor, Actuator: ThermalActuator>
    ThermalSupervisor<Sensor, Actuator>
{
    pub const fn new(
        manager: ThermalManager,
        sensor: Sensor,
        actuator: Actuator,
    ) -> Self {
        Self {
            manager,
            sensor,
            actuator,
        }
    }

    pub fn poll(&mut self) -> Result<ThermalAction, Status> {
        let action = self.manager.update(self.sensor.read()?);
        self.actuator.set_throttle_percent(action.throttle_percent())?;
        Ok(action)
    }

    pub fn into_parts(self) -> (ThermalManager, Sensor, Actuator) {
        (self.manager, self.sensor, self.actuator)
    }
}

impl ThermalManager {
    pub fn new(
        trips: ThermalTripPoints,
        hysteresis_deci_kelvin: u32,
    ) -> Result<Self, AcpiError> {
        Ok(Self {
            trips: trips.validate()?,
            hysteresis_deci_kelvin,
            action: ThermalAction::Normal,
            last_reading: None,
            events: ThermalEventLog::new(),
        })
    }

    pub const fn action(&self) -> ThermalAction {
        self.action
    }

    pub const fn last_reading(&self) -> Option<ThermalReading> {
        self.last_reading
    }

    pub const fn thermal_events_pending(&self) -> usize {
        self.events.len()
    }

    pub const fn dropped_thermal_events(&self) -> u64 {
        self.events.dropped()
    }

    pub fn drain_thermal_events(
        &mut self,
        destination: &mut [Option<ThermalEvent>],
    ) -> usize {
        self.events.drain(destination)
    }

    pub fn update(&mut self, reading: ThermalReading) -> ThermalAction {
        self.last_reading = Some(reading);
        let temperature = reading.temperature_deci_kelvin;
        let previous_action = self.action;
        let (action, event) = crate::native::decide(
            self.trips, self.hysteresis_deci_kelvin, temperature, previous_action,
        );
        self.action = action;
        if event != 0 {
            let kind = match event {
                1 => ThermalEventKind::ThrottleStarted,
                2 => ThermalEventKind::ThrottleChanged,
                3 => ThermalEventKind::Recovered,
                4 => ThermalEventKind::Critical,
                _ => unreachable!("native thermal event"),
            };
            self.events.push(ThermalEvent {
                kind,
                reading,
                previous_action,
                action,
            })
        }
        action
    }
}
