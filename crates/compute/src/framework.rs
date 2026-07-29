use synos_ipc::SharedBuffer;

use crate::{Error, tensor::SharedTensor};

pub const MAX_INVOCATION_BINDINGS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameworkKind {
    Candle,
    Burn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ModelHandle(u32);

impl ModelHandle {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TensorHandle(u32);

impl TensorHandle {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingAccess {
    ReadOnly,
    WriteOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelSource {
    pub buffer: SharedBuffer,
}

impl ModelSource {
    pub fn new(buffer: SharedBuffer) -> Result<Self, Error> {
        if buffer.length == 0 || buffer.writable {
            return Err(Error::InvalidModel);
        }
        Ok(Self { buffer })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TensorImport {
    pub tensor: SharedTensor,
    pub access: BindingAccess,
}

impl TensorImport {
    pub fn new(tensor: SharedTensor, access: BindingAccess) -> Result<Self, Error> {
        tensor.validate()?;
        if access.writable() && !tensor.buffer.writable {
            return Err(Error::ReadOnly);
        }
        Ok(Self { tensor, access })
    }
}

impl BindingAccess {
    pub const fn writable(self) -> bool {
        matches!(self, Self::WriteOnly | Self::ReadWrite)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TensorBinding {
    pub slot: u16,
    pub tensor: TensorHandle,
    pub access: BindingAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Invocation {
    bindings: [Option<TensorBinding>; MAX_INVOCATION_BINDINGS],
    binding_count: u8,
}

impl Invocation {
    pub fn new(bindings: &[TensorBinding]) -> Result<Self, Error> {
        if bindings.len() > MAX_INVOCATION_BINDINGS {
            return Err(Error::InvalidDispatch);
        }
        let mut stored = [None; MAX_INVOCATION_BINDINGS];
        for (index, binding) in bindings.iter().copied().enumerate() {
            if bindings[..index]
                .iter()
                .any(|existing| existing.slot == binding.slot)
            {
                return Err(Error::DuplicateBinding);
            }
            stored[index] = Some(binding)
        }
        Ok(Self {
            bindings: stored,
            binding_count: bindings.len() as u8,
        })
    }

    pub fn bindings(&self) -> impl Iterator<Item = TensorBinding> + '_ {
        self.bindings[..self.binding_count as usize]
            .iter()
            .flatten()
            .copied()
    }
}

/// Native contract implemented by Candle and Burn SynOS ports.
///
/// Model bytes and tensors are imported by shared-page descriptor. An
/// implementation keeps those mappings as runtime storage instead of copying
/// them into a framework-owned heap.
pub trait NativeFramework {
    fn kind(&self) -> FrameworkKind;

    fn load_model(&mut self, model: ModelSource) -> Result<ModelHandle, Error>;

    fn unload_model(&mut self, model: ModelHandle) -> Result<(), Error>;

    fn import_tensor(&mut self, tensor: TensorImport) -> Result<TensorHandle, Error>;

    fn release_tensor(&mut self, tensor: TensorHandle) -> Result<(), Error>;

    fn execute(&mut self, model: ModelHandle, invocation: Invocation) -> Result<(), Error>;
}

/// Small host surface used instead of a libc or foreign-language runtime.
///
/// The `std::sys::synos` ports for Candle and Burn can implement their thread,
/// clock, and accelerator hooks directly on this contract.
pub trait SynosRuntime {
    fn monotonic_time_ns(&self) -> u64;

    fn yield_now(&mut self);

    fn accelerator_available(&self) -> bool;
}
