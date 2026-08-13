use crate::AcpiError;
use synos_status::Status;

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
    next: usize,
    len: usize,
    dropped: u64,
}

impl<const CAPACITY: usize> ThermalEventLog<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            events: [None; CAPACITY],
            next: 0,
            len: 0,
            dropped: 0,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    fn push(&mut self, event: ThermalEvent) {
        if CAPACITY == 0 {
            self.dropped = self.dropped.saturating_add(1);
            return
        }
        if self.len == CAPACITY {
            self.dropped = self.dropped.saturating_add(1)
        } else {
            self.len += 1
        }
        self.events[self.next] = Some(event);
        self.next = if self.next + 1 == CAPACITY {
            0
        } else {
            self.next + 1
        };
    }

    pub fn drain(&mut self, destination: &mut [Option<ThermalEvent>]) -> usize {
        let count = self.len.min(destination.len());
        if CAPACITY == 0 {
            return 0
        }
        let mut index = if self.len == CAPACITY {
            self.next
        } else if self.next >= self.len {
            self.next - self.len
        } else {
            CAPACITY - (self.len - self.next)
        };
        for slot in destination.iter_mut().take(count) {
            *slot = self.events[index].take();
            index = if index + 1 == CAPACITY { 0 } else { index + 1 };
        }
        self.len -= count;
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
        let mut action = previous_action;
        if self
            .trips
            .critical_deci_kelvin
            .is_some_and(|critical| temperature >= critical)
        {
            action = ThermalAction::EmergencyShutdown
        } else if self
            .trips
            .hot_deci_kelvin
            .is_some_and(|hot| temperature >= hot)
        {
            action = ThermalAction::Throttle { percent: 75 }
        } else if let Some(passive) = self.trips.passive_deci_kelvin {
            if temperature >= passive {
                let above = temperature - passive;
                action = ThermalAction::Throttle {
                    percent: 25_u8.saturating_add((above / 2).min(50) as u8),
                }
            } else if temperature.saturating_add(self.hysteresis_deci_kelvin) < passive {
                action = ThermalAction::Normal
            }
        } else {
            action = ThermalAction::Normal
        }
        self.action = action;
        if action != previous_action {
            let kind = match action {
                ThermalAction::EmergencyShutdown => ThermalEventKind::Critical,
                ThermalAction::Normal => ThermalEventKind::Recovered,
                ThermalAction::Throttle { .. } if previous_action == ThermalAction::Normal => {
                    ThermalEventKind::ThrottleStarted
                }
                ThermalAction::Throttle { .. } => ThermalEventKind::ThrottleChanged,
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
