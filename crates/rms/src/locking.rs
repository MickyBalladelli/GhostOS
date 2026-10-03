use crate::record::RecordLocation;
use ghostos_status::{IntoStatus, Status};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DlmResource {
    pub id: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DlmLockRange {
    WholeFile,
    Record(u32),
}

impl From<RecordLocation> for DlmLockRange {
    fn from(location: RecordLocation) -> Self {
        Self::Record(location.position)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DlmLockMode {
    Read,
    Update,
}

/// Native DLM service binding used by RMS record locks.
///
/// A kernel IPC client implements this trait by mapping `Read` to protected
/// read, `Update` to protected write, and `Record(n)` to a one-byte DLM range
/// beginning at `n`.
pub trait DlmBinding {
    type Error: IntoStatus;
    type Handle: Copy;

    fn acquire(
        &mut self,
        resource: DlmResource,
        range: DlmLockRange,
        mode: DlmLockMode,
        wait: bool,
    ) -> Result<Self::Handle, Self::Error>;

    fn release(&mut self, handle: Self::Handle) -> Result<(), Self::Error>;
}

#[derive(Debug, Eq, PartialEq)]
pub enum RecordLockError<E> {
    Backend(E),
    InvalidPath,
}

impl<E: IntoStatus> IntoStatus for RecordLockError<E> {
    fn status(self) -> Status {
        match self {
            Self::Backend(error) => error.status(),
            Self::InvalidPath => Status::INVALID_ARGUMENT,
        }
    }
}

pub struct DlmRecordLocks<B> {
    binding: B,
}

impl<B> DlmRecordLocks<B> {
    pub const fn new(binding: B) -> Self {
        Self { binding }
    }

    pub fn binding(&self) -> &B {
        &self.binding
    }

    pub fn binding_mut(&mut self) -> &mut B {
        &mut self.binding
    }

    pub fn into_binding(self) -> B {
        self.binding
    }
}

impl<B: DlmBinding> DlmRecordLocks<B> {
    pub fn lock(
        &mut self,
        path: &str,
        range: DlmLockRange,
        mode: DlmLockMode,
        wait: bool,
    ) -> Result<RecordLockGuard<'_, B>, RecordLockError<B::Error>> {
        if crate::native::lock_path(path.as_bytes()).is_err() {
            return Err(RecordLockError::InvalidPath);
        }
        let handle = self
            .binding
            .acquire(
                DlmResource {
                    id: crate::native::resource_id(path.as_bytes()),
                },
                range,
                mode,
                wait,
            )
            .map_err(RecordLockError::Backend)?;
        Ok(RecordLockGuard {
            binding: &mut self.binding,
            handle: Some(handle),
        })
    }
}

pub struct RecordLockGuard<'a, B: DlmBinding> {
    binding: &'a mut B,
    handle: Option<B::Handle>,
}

impl<B: DlmBinding> RecordLockGuard<'_, B> {
    pub fn release(mut self) -> Result<(), B::Error> {
        let handle = self.handle.take().expect("live record lock");
        self.binding.release(handle)
    }
}

impl<B: DlmBinding> Drop for RecordLockGuard<'_, B> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = self.binding.release(handle);
        }
    }
}

