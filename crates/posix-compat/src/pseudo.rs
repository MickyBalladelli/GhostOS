use ghostos_system_model::{LogicalName, logical::{LogicalError, LogicalTarget, LogicalTargetKind}};
use ghostos_system_model::logical_fast::LogicalFastPath;

pub const MAX_PSEUDO_PATH_BYTES: usize = ghostos_system_model::MAX_NAME_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PseudoResourceKind {
    Proc,
    Sys,
    Dev,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PseudoPath {
    kind: PseudoResourceKind,
    logical: [u8; MAX_PSEUDO_PATH_BYTES],
    logical_length: u8,
}

impl PseudoPath {
    pub fn parse(path: &[u8]) -> Result<Self, PseudoPathError> {
        let (logical, logical_length, kind) = crate::native::pseudo_parse(path)?;
        let kind = match kind {
            0 => PseudoResourceKind::Proc,
            1 => PseudoResourceKind::Sys,
            2 => PseudoResourceKind::Dev,
            _ => unreachable!("native POSIX pseudo kind"),
        };
        Ok(Self { kind, logical, logical_length })
    }

    pub const fn kind(self) -> PseudoResourceKind {
        self.kind
    }

    pub fn logical_name(self) -> Result<LogicalName, PseudoPathError> {
        LogicalName::new(self.logical_name_str()).map_err(|_| PseudoPathError::InvalidPath)
    }

    fn logical_name_str(&self) -> &str {
        core::str::from_utf8(&self.logical[..self.logical_length as usize])
            .unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PseudoPathError {
    NotPseudoPath,
    InvalidPath,
    TooLong,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PseudoResource {
    pub kind: PseudoResourceKind,
    pub target: LogicalTarget,
}

pub trait LogicalNameResolver {
    type Error;

    fn resolve_logical(&self, process_id: u64, name: &str) -> Result<LogicalTarget, Self::Error>;
}

impl<const PROCESS: usize, const SYSTEM: usize> LogicalNameResolver
    for LogicalFastPath<PROCESS, SYSTEM>
{
    type Error = LogicalError;

    fn resolve_logical(&self, process_id: u64, name: &str) -> Result<LogicalTarget, Self::Error> {
        self.resolve(process_id, name).map(|resolved| resolved.target)
    }
}

/// Read-only resolver for the dynamic Linux pseudo-filesystem views.
///
/// A path is converted to a scoped logical name (`/proc/meminfo` becomes
/// `PROC_MEMINFO`) and resolved through the caller's process fast path. The
/// returned target keeps the logical table's file/device/channel kind, so the
/// caller can attach only the capability named by that target.
pub struct PseudoFileSystem;

impl PseudoFileSystem {
    pub const fn new() -> Self {
        Self
    }

    pub fn resolve<R: LogicalNameResolver>(
        &self,
        resolver: &R,
        process_id: u64,
        path: &[u8],
    ) -> Result<PseudoResource, PseudoFsError<R::Error>> {
        let parsed = PseudoPath::parse(path).map_err(PseudoFsError::Path)?;
        let target = resolver
            .resolve_logical(process_id, parsed.logical_name_str())
            .map_err(PseudoFsError::Logical)?;
        Ok(PseudoResource {
            kind: parsed.kind,
            target,
        })
    }
}

impl Default for PseudoFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PseudoFsError<E> {
    Path(PseudoPathError),
    Logical(E),
}

impl PseudoResource {
    pub const fn is_device(self) -> bool {
        matches!(self.target.kind, LogicalTargetKind::Device)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_proc_paths_to_logical_names() {
        let path = PseudoPath::parse(b"/proc/meminfo").expect("valid pseudo path");
        assert_eq!(path.kind(), PseudoResourceKind::Proc);
        assert_eq!(path.logical_name().expect("valid pseudo path").as_str(), "PROC_MEMINFO");
    }

    #[test]
    fn rejects_pseudo_path_traversal() {
        assert_eq!(
            PseudoPath::parse(b"/sys/../dev/null"),
            Err(PseudoPathError::InvalidPath)
        );
    }
}
