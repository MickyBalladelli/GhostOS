use crate::AcpiError;
use synos_status::Status;

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

/// Applies ACPI trip points with hysteresis so fan noise and CPU frequency do
/// not oscillate around one sensor value.
pub struct ThermalManager {
    trips: ThermalTripPoints,
    hysteresis_deci_kelvin: u32,
    action: ThermalAction,
    last_reading: Option<ThermalReading>,
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
        self.actuator.set_throttle_percent(match action {
            ThermalAction::Normal => 0,
            ThermalAction::Throttle { percent } => percent,
            ThermalAction::EmergencyShutdown => 100,
        })?;
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
        })
    }

    pub const fn action(&self) -> ThermalAction {
        self.action
    }

    pub const fn last_reading(&self) -> Option<ThermalReading> {
        self.last_reading
    }

    pub fn update(&mut self, reading: ThermalReading) -> ThermalAction {
        self.last_reading = Some(reading);
        let temperature = reading.temperature_deci_kelvin;
        if self
            .trips
            .critical_deci_kelvin
            .is_some_and(|critical| temperature >= critical)
        {
            self.action = ThermalAction::EmergencyShutdown;
            return self.action
        }
        if self
            .trips
            .hot_deci_kelvin
            .is_some_and(|hot| temperature >= hot)
        {
            self.action = ThermalAction::Throttle { percent: 75 };
            return self.action
        }
        if let Some(passive) = self.trips.passive_deci_kelvin {
            if temperature >= passive {
                let above = temperature - passive;
                self.action = ThermalAction::Throttle {
                    percent: 25_u8.saturating_add((above / 2).min(50) as u8),
                };
                return self.action
            }
            if temperature.saturating_add(self.hysteresis_deci_kelvin) < passive {
                self.action = ThermalAction::Normal
            }
        } else {
            self.action = ThermalAction::Normal
        }
        self.action
    }
}
