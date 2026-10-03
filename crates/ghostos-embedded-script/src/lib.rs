#![no_std]
#![deny(unsafe_code)]

#[allow(unsafe_code)]
mod native;

extern crate alloc;

use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt;

use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, ParseError, Position, Scope};
use ghostos_status::{IntoStatus, Status};

pub const MAX_SCRIPT_CAPABILITIES: usize = 16;
pub const MAX_RESOURCE_NAME_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScriptLimits {
    pub max_source_bytes: usize,
    pub max_operations: u64,
    pub max_call_levels: usize,
    pub max_expression_depth: usize,
    pub max_variables: usize,
    pub max_functions: usize,
    pub max_modules: usize,
    pub max_string_bytes: usize,
    pub max_array_items: usize,
    pub max_map_items: usize,
    pub max_requests: usize,
    pub max_request_bytes: usize,
}

impl ScriptLimits {
    pub const DEFAULT: Self = Self {
        max_source_bytes: 64 * 1024,
        max_operations: 100_000,
        max_call_levels: 32,
        max_expression_depth: 64,
        max_variables: 64,
        max_functions: 32,
        max_modules: 8,
        max_string_bytes: 4 * 1024,
        max_array_items: 256,
        max_map_items: 128,
        max_requests: 64,
        max_request_bytes: 4 * 1024,
    };

    pub fn is_valid(self) -> bool {
        crate::native::limits(
            self.max_source_bytes,
            self.max_operations,
            self.max_call_levels,
            self.max_expression_depth,
            self.max_variables,
            self.max_functions,
            self.max_modules,
            self.max_string_bytes,
            self.max_array_items,
            self.max_map_items,
            self.max_requests,
            self.max_request_bytes,
        )
    }
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptCapability {
    resource: String,
    operations: u64,
}

impl ScriptCapability {
    pub fn new(resource: &str, operations: u64) -> Result<Self, Error> {
        if !crate::native::capability(resource.as_bytes(), operations) {
            return Err(Error::InvalidCapability);
        }
        Ok(Self {
            resource: resource.to_string(),
            operations,
        })
    }

    pub fn single(resource: &str, operation: u8) -> Result<Self, Error> {
        let mask = crate::native::operation_mask(operation).ok_or(Error::InvalidOperation)?;
        Self::new(resource, mask)
    }

    pub fn resource(&self) -> &str {
        &self.resource
    }

    pub const fn operations(&self) -> u64 {
        self.operations
    }

    pub fn allows(&self, operation: u8) -> bool {
        crate::native::allows(self.operations, operation)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutomationRequest {
    pub sequence: u32,
    pub resource: String,
    pub operation: u8,
    pub payload: String,
}

#[derive(Clone, Debug)]
struct AutomationContext {
    capabilities: Vec<ScriptCapability>,
    requests: Vec<AutomationRequest>,
    max_requests: usize,
    max_request_bytes: usize,
}

impl AutomationContext {
    fn new(
        capabilities: &[ScriptCapability],
        max_requests: usize,
        max_request_bytes: usize,
    ) -> Result<Self, Error> {
        let code = if capabilities.len() > MAX_SCRIPT_CAPABILITIES {
            crate::native::names(&[], &[], capabilities.len(), MAX_SCRIPT_CAPABILITIES)
        } else {
            let mut names = [core::ptr::null::<u8>(); MAX_SCRIPT_CAPABILITIES];
            let mut lengths = [0; MAX_SCRIPT_CAPABILITIES];
            for (index, capability) in capabilities.iter().enumerate() {
                names[index] = capability.resource.as_ptr();
                lengths[index] = capability.resource.len();
            }
            crate::native::names(&names[..capabilities.len()], &lengths[..capabilities.len()], capabilities.len(), MAX_SCRIPT_CAPABILITIES)
        };
        match code {
            0 => {}
            1 => return Err(Error::Capacity),
            _ => return Err(Error::DuplicateCapability),
        }
        let mut stored = Vec::with_capacity(capabilities.len());
        for capability in capabilities {
            stored.push(capability.clone())
        }
        Ok(Self {
            capabilities: stored,
            requests: Vec::new(),
            max_requests,
            max_request_bytes,
        })
    }

    fn has(&mut self, resource: &str, operation: i64) -> bool {
        let Ok(operation) = u8::try_from(operation) else {
            return false;
        };
        self.capabilities
            .iter()
            .any(|capability| capability.resource == resource && crate::native::allows(capability.operations, operation))
    }

    fn request(
        &mut self,
        resource: &str,
        operation: i64,
        payload: &str,
    ) -> Result<i64, Box<EvalAltResult>> {
        let parsed = u8::try_from(operation).ok().filter(|operation| *operation < 64);
        let allowed = parsed.is_some_and(|operation| {
            payload.len() <= self.max_request_bytes
                && self.capabilities.iter().any(|capability| {
                    capability.resource == resource && crate::native::allows(capability.operations, operation)
                })
        });
        let sequence = match crate::native::request(
            parsed.is_some(),
            parsed.unwrap_or(0),
            allowed,
            payload.len(),
            self.max_request_bytes,
            self.requests.len(),
            self.max_requests,
        ) {
            Ok(sequence) => sequence,
            Err(1) => return Err(script_error(Error::InvalidOperation)),
            Err(2) => return Err(script_error(Error::PayloadTooLarge)),
            Err(3) => return Err(script_error(Error::AccessDenied)),
            Err(_) => return Err(script_error(Error::Capacity)),
        };
        self.requests.push(AutomationRequest {
            sequence,
            resource: resource.to_string(),
            operation: parsed.unwrap_or(0),
            payload: payload.to_string(),
        });
        Ok(i64::from(sequence))
    }
}

pub struct EmbeddedScriptEngine {
    engine: Engine,
    limits: ScriptLimits,
}

impl EmbeddedScriptEngine {
    pub fn new(limits: ScriptLimits) -> Result<Self, Error> {
        if !limits.is_valid() {
            return Err(Error::InvalidLimits);
        }
        let mut engine = Engine::new();
        engine
            .set_max_operations(limits.max_operations)
            .set_max_call_levels(limits.max_call_levels)
            .set_max_expr_depths(limits.max_expression_depth, limits.max_expression_depth)
            .set_max_variables(limits.max_variables)
            .set_max_functions(limits.max_functions)
            .set_max_modules(limits.max_modules)
            .set_max_string_size(limits.max_string_bytes)
            .set_max_array_size(limits.max_array_items)
            .set_max_map_size(limits.max_map_items);
        engine.register_type_with_name::<AutomationContext>("GhostOS");
        engine.register_fn("has", AutomationContext::has);
        engine.register_fn("request", AutomationContext::request);
        Ok(Self { engine, limits })
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    pub const fn limits(&self) -> ScriptLimits {
        self.limits
    }

    pub fn execute(
        &self,
        source: &str,
        capabilities: &[ScriptCapability],
    ) -> Result<ScriptOutcome, Error> {
        if !crate::native::source(source.len(), self.limits.max_source_bytes) {
            return Err(Error::SourceTooLarge);
        }
        let context = AutomationContext::new(
            capabilities,
            self.limits.max_requests,
            self.limits.max_request_bytes,
        )?;
        let ast = self.engine.compile(source).map_err(Error::Compile)?;
        let mut scope = Scope::new();
        scope.push("ghostos", context);
        let value = self
            .engine
            .eval_ast_with_scope::<Dynamic>(&mut scope, &ast)
            .map_err(Error::Evaluate)?;
        let context = scope
            .get_value::<AutomationContext>("ghostos")
            .ok_or(Error::ContextLost)?;
        Ok(ScriptOutcome {
            value,
            requests: context.requests,
        })
    }
}

impl Default for EmbeddedScriptEngine {
    fn default() -> Self {
        Self::new(ScriptLimits::DEFAULT).expect("default script limits are valid")
    }
}

pub struct ScriptOutcome {
    pub value: Dynamic,
    pub requests: Vec<AutomationRequest>,
}

#[derive(Debug)]
pub enum Error {
    AccessDenied,
    Capacity,
    Compile(ParseError),
    ContextLost,
    DuplicateCapability,
    Evaluate(Box<EvalAltResult>),
    InvalidCapability,
    InvalidLimits,
    InvalidOperation,
    PayloadTooLarge,
    SourceTooLarge,
}

impl Error {
    pub const fn kind(&self) -> ErrorKind {
        match self {
            Self::AccessDenied => ErrorKind::AccessDenied,
            Self::Capacity => ErrorKind::Capacity,
            Self::Compile(_) => ErrorKind::Compile,
            Self::ContextLost => ErrorKind::ContextLost,
            Self::DuplicateCapability => ErrorKind::DuplicateCapability,
            Self::Evaluate(_) => ErrorKind::Evaluate,
            Self::InvalidCapability => ErrorKind::InvalidCapability,
            Self::InvalidLimits => ErrorKind::InvalidLimits,
            Self::InvalidOperation => ErrorKind::InvalidOperation,
            Self::PayloadTooLarge => ErrorKind::PayloadTooLarge,
            Self::SourceTooLarge => ErrorKind::SourceTooLarge,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Compile(error) => error.fmt(formatter),
            Self::Evaluate(error) => error.fmt(formatter),
            _ => formatter.write_str(self.kind().message()),
        }
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self.kind() {
            ErrorKind::AccessDenied => Status::ACCESS_DENIED,
            ErrorKind::Capacity | ErrorKind::PayloadTooLarge | ErrorKind::SourceTooLarge => {
                Status::NO_SPACE
            }
            ErrorKind::ContextLost => Status::CORRUPT,
            ErrorKind::Compile
            | ErrorKind::DuplicateCapability
            | ErrorKind::Evaluate
            | ErrorKind::InvalidCapability
            | ErrorKind::InvalidLimits
            | ErrorKind::InvalidOperation => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    AccessDenied,
    Capacity,
    Compile,
    ContextLost,
    DuplicateCapability,
    Evaluate,
    InvalidCapability,
    InvalidLimits,
    InvalidOperation,
    PayloadTooLarge,
    SourceTooLarge,
}

impl ErrorKind {
    const fn message(self) -> &'static str {
        match self {
            Self::AccessDenied => "script capability denied",
            Self::Capacity => "script capacity exceeded",
            Self::Compile => "script compilation failed",
            Self::ContextLost => "script automation context was removed",
            Self::DuplicateCapability => "duplicate script capability",
            Self::Evaluate => "script evaluation failed",
            Self::InvalidCapability => "invalid script capability",
            Self::InvalidLimits => "invalid script limits",
            Self::InvalidOperation => "invalid script operation",
            Self::PayloadTooLarge => "script request payload is too large",
            Self::SourceTooLarge => "script source is too large",
        }
    }
}

fn script_error(error: Error) -> Box<EvalAltResult> {
    EvalAltResult::ErrorRuntime(
        Dynamic::from(ImmutableString::from(error.kind().message())),
        Position::NONE,
    )
    .into()
}
