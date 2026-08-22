#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use core::fmt;

use ghostos_status::{IntoStatus, Status};
use wasmi::{
    Caller, CompilationMode, Config, EnforcedLimits, Engine, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, errors::LinkerError,
};

pub const MAX_WASM_CAPABILITIES: usize = 16;
pub const HOST_ACCESS_DENIED: i64 = i64::MIN;
pub const HOST_INVALID_OPERATION: i64 = i64::MIN + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WasmLimits {
    pub fuel: u64,
    pub max_module_bytes: usize,
    pub max_memory_bytes: usize,
    pub max_table_elements: usize,
    pub max_instances: usize,
    pub max_memories: usize,
    pub max_tables: usize,
}

impl WasmLimits {
    pub const DEFAULT: Self = Self {
        fuel: 1_000_000,
        max_module_bytes: 2 * 1024 * 1024,
        max_memory_bytes: 4 * 1024 * 1024,
        max_table_elements: 1_024,
        max_instances: 1,
        max_memories: 2,
        max_tables: 2,
    };

    pub const fn is_valid(self) -> bool {
        self.fuel > 0
            && self.max_module_bytes > 0
            && self.max_memory_bytes > 0
            && self.max_table_elements > 0
            && self.max_instances > 0
            && self.max_memories > 0
            && self.max_tables > 0
    }
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityGrant {
    pub handle: u64,
    operations: u64,
}

impl CapabilityGrant {
    pub const fn new(handle: u64, operations: u64) -> Option<Self> {
        if handle == 0 || operations == 0 {
            None
        } else {
            Some(Self { handle, operations })
        }
    }

    pub const fn single(handle: u64, operation: u8) -> Option<Self> {
        if operation >= 64 {
            None
        } else {
            Self::new(handle, 1_u64 << operation)
        }
    }

    pub const fn operations(self) -> u64 {
        self.operations
    }

    pub const fn allows(self, operation: u8) -> bool {
        operation < 64 && self.operations & (1_u64 << operation) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CapabilitySet {
    grants: [Option<CapabilityGrant>; MAX_WASM_CAPABILITIES],
}

impl CapabilitySet {
    fn new(grants: &[CapabilityGrant]) -> Result<Self, Error> {
        if grants.len() > MAX_WASM_CAPABILITIES {
            return Err(Error::CapabilityCapacity);
        }
        let mut result = Self {
            grants: [None; MAX_WASM_CAPABILITIES],
        };
        for (index, grant) in grants.iter().copied().enumerate() {
            if grant.handle == 0 || grant.operations == 0 {
                return Err(Error::InvalidCapability);
            }
            if result
                .grants
                .iter()
                .flatten()
                .any(|existing| existing.handle == grant.handle)
            {
                return Err(Error::DuplicateCapability);
            }
            result.grants[index] = Some(grant)
        }
        Ok(result)
    }

    fn find(&self, handle: u64) -> Option<CapabilityGrant> {
        self.grants
            .iter()
            .flatten()
            .find(|grant| grant.handle == handle)
            .copied()
    }

    fn allows(&self, handle: u64, operation: u8) -> bool {
        self.find(handle)
            .is_some_and(|grant| grant.allows(operation))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostCall {
    pub capability: CapabilityGrant,
    pub operation: u8,
    pub arguments: [i64; 4],
}

pub trait WasmHost {
    /// Runs one already-authorized operation. The host remains responsible for
    /// resolving the opaque generation-checked handle at the service boundary.
    fn invoke(&mut self, call: HostCall) -> i64;
}

impl WasmHost for () {
    fn invoke(&mut self, _call: HostCall) -> i64 {
        HOST_ACCESS_DENIED
    }
}

struct StoreState<H> {
    host: H,
    capabilities: CapabilitySet,
    limits: StoreLimits,
    denied_calls: u64,
}

pub struct WasmRuntime {
    engine: Engine,
    limits: WasmLimits,
}

impl WasmRuntime {
    pub fn new(limits: WasmLimits) -> Result<Self, Error> {
        if !limits.is_valid() {
            return Err(Error::InvalidLimits);
        }
        let mut config = Config::default();
        config.consume_fuel(true);
        config.floats(false);
        config.compilation_mode(CompilationMode::Eager);
        config.enforced_limits(EnforcedLimits::strict());
        let engine = Engine::new(&config);
        Ok(Self { engine, limits })
    }

    pub const fn limits(&self) -> WasmLimits {
        self.limits
    }

    pub fn execute<H: WasmHost + 'static>(
        &self,
        wasm: &[u8],
        entry: &str,
        arguments: [i64; 2],
        capabilities: &[CapabilityGrant],
        host: H,
    ) -> Result<WasmOutcome<H>, Error> {
        if entry.is_empty() {
            return Err(Error::InvalidEntry);
        }
        if wasm.len() > self.limits.max_module_bytes {
            return Err(Error::ModuleTooLarge);
        }
        let capabilities = CapabilitySet::new(capabilities)?;
        let module = Module::new(&self.engine, wasm).map_err(Error::Compile)?;
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(self.limits.max_memory_bytes)
            .table_elements(self.limits.max_table_elements)
            .instances(self.limits.max_instances)
            .memories(self.limits.max_memories)
            .tables(self.limits.max_tables)
            .trap_on_grow_failure(true)
            .build();
        let state = StoreState {
            host,
            capabilities,
            limits: store_limits,
            denied_calls: 0,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(self.limits.fuel).map_err(Error::Fuel)?;

        let mut linker = Linker::<StoreState<H>>::new(&self.engine);
        linker
            .func_wrap(
                "ghostos",
                "capability_check",
                |caller: Caller<'_, StoreState<H>>, handle: i64, operation: i32| -> i32 {
                    let Ok(operation) = u8::try_from(operation) else {
                        return 0;
                    };
                    i32::from(caller.data().capabilities.allows(handle as u64, operation))
                },
            )
            .map_err(Error::Link)?;
        linker
            .func_wrap(
                "ghostos",
                "invoke",
                |mut caller: Caller<'_, StoreState<H>>,
                 handle: i64,
                 operation: i32,
                 arg0: i64,
                 arg1: i64,
                 arg2: i64,
                 arg3: i64|
                 -> i64 {
                    let handle = handle as u64;
                    let Ok(operation) = u8::try_from(operation) else {
                        caller.data_mut().denied_calls += 1;
                        return HOST_INVALID_OPERATION;
                    };
                    let Some(capability) = caller.data().capabilities.find(handle) else {
                        caller.data_mut().denied_calls += 1;
                        return HOST_ACCESS_DENIED;
                    };
                    if !capability.allows(operation) {
                        caller.data_mut().denied_calls += 1;
                        return HOST_ACCESS_DENIED;
                    }
                    caller.data_mut().host.invoke(HostCall {
                        capability,
                        operation,
                        arguments: [arg0, arg1, arg2, arg3],
                    })
                },
            )
            .map_err(Error::Link)?;

        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(Error::Instantiate)?;
        let function = instance
            .get_typed_func::<(i64, i64), i64>(&store, entry)
            .map_err(Error::InvalidExport)?;
        let value = function
            .call(&mut store, (arguments[0], arguments[1]))
            .map_err(Error::Trap)?;
        let fuel_remaining = store.get_fuel().map_err(Error::Fuel)?;
        let state = store.into_data();
        Ok(WasmOutcome {
            value,
            fuel_consumed: self.limits.fuel.saturating_sub(fuel_remaining),
            denied_calls: state.denied_calls,
            host: state.host,
        })
    }
}

impl Default for WasmRuntime {
    fn default() -> Self {
        Self::new(WasmLimits::DEFAULT).expect("default Wasm limits are valid")
    }
}

pub struct WasmOutcome<H> {
    pub value: i64,
    pub fuel_consumed: u64,
    pub denied_calls: u64,
    pub host: H,
}

#[derive(Debug)]
pub enum Error {
    CapabilityCapacity,
    Compile(wasmi::Error),
    DuplicateCapability,
    Fuel(wasmi::Error),
    Instantiate(wasmi::Error),
    InvalidCapability,
    InvalidEntry,
    InvalidExport(wasmi::Error),
    InvalidLimits,
    Link(LinkerError),
    ModuleTooLarge,
    Trap(wasmi::Error),
}

impl Error {
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::CapabilityCapacity => ErrorKind::CapabilityCapacity,
            Self::Compile(_) => ErrorKind::Compile,
            Self::DuplicateCapability => ErrorKind::DuplicateCapability,
            Self::Fuel(_) => ErrorKind::Fuel,
            Self::Instantiate(_) => ErrorKind::Instantiate,
            Self::InvalidCapability => ErrorKind::InvalidCapability,
            Self::InvalidEntry => ErrorKind::InvalidEntry,
            Self::InvalidExport(_) => ErrorKind::InvalidExport,
            Self::InvalidLimits => ErrorKind::InvalidLimits,
            Self::Link(_) => ErrorKind::Link,
            Self::ModuleTooLarge => ErrorKind::ModuleTooLarge,
            Self::Trap(_) => ErrorKind::Trap,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error)
            | Self::Fuel(error)
            | Self::Instantiate(error)
            | Self::InvalidExport(error)
            | Self::Trap(error) => error.fmt(formatter),
            Self::Link(error) => error.fmt(formatter),
            _ => formatter.write_str(self.kind().message()),
        }
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self.kind() {
            ErrorKind::CapabilityCapacity | ErrorKind::ModuleTooLarge => Status::NO_SPACE,
            ErrorKind::DuplicateCapability
            | ErrorKind::InvalidCapability
            | ErrorKind::InvalidEntry
            | ErrorKind::InvalidExport
            | ErrorKind::InvalidLimits => Status::INVALID_ARGUMENT,
            ErrorKind::Compile | ErrorKind::Instantiate | ErrorKind::Link => Status::CORRUPT,
            ErrorKind::Fuel | ErrorKind::Trap => Status::ACCESS_DENIED,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    CapabilityCapacity,
    Compile,
    DuplicateCapability,
    Fuel,
    Instantiate,
    InvalidCapability,
    InvalidEntry,
    InvalidExport,
    InvalidLimits,
    Link,
    ModuleTooLarge,
    Trap,
}

impl ErrorKind {
    const fn message(self) -> &'static str {
        match self {
            Self::CapabilityCapacity => "too many Wasm capabilities",
            Self::Compile => "Wasm compilation failed",
            Self::DuplicateCapability => "duplicate Wasm capability",
            Self::Fuel => "Wasm fuel configuration failed",
            Self::Instantiate => "Wasm instantiation failed",
            Self::InvalidCapability => "invalid Wasm capability",
            Self::InvalidEntry => "invalid Wasm entry point",
            Self::InvalidExport => "Wasm entry point has the wrong signature",
            Self::InvalidLimits => "invalid Wasm limits",
            Self::Link => "Wasm host ABI registration failed",
            Self::ModuleTooLarge => "Wasm module is too large",
            Self::Trap => "Wasm execution trapped",
        }
    }
}
