use synos_system_model::{LogicalName, logical::{LogicalError, LogicalTarget, LogicalTargetKind}};
use synos_system_model::logical_fast::LogicalFastPath;

pub const MAX_PSEUDO_PATH_BYTES: usize = synos_system_model::MAX_NAME_BYTES;

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
        let (prefix, mount_length, kind) = if path == b"/proc" || path.starts_with(b"/proc/") {
            (b"PROC_" as &[u8], b"/proc".len(), PseudoResourceKind::Proc)
        } else if path == b"/sys" || path.starts_with(b"/sys/") {
            (b"SYS_" as &[u8], b"/sys".len(), PseudoResourceKind::Sys)
        } else if path == b"/dev" || path.starts_with(b"/dev/") {
            (b"DEV_" as &[u8], b"/dev".len(), PseudoResourceKind::Dev)
        } else {
            return Err(PseudoPathError::NotPseudoPath);
        };
        if path.contains(&0) || path.contains(&b'\\') {
            return Err(PseudoPathError::InvalidPath);
        }

        let suffix = if path.len() == mount_length {
            b"ROOT" as &[u8]
        } else {
            &path[mount_length + 1..]
        };
        if suffix.is_empty()
            || suffix == b"/"
            || suffix.split(|byte| *byte == b'/').any(|part| part.is_empty() || part == b".")
            || suffix.split(|byte| *byte == b'/').any(|part| part == b"..")
        {
            return Err(PseudoPathError::InvalidPath);
        }
        let mut logical = [0; MAX_PSEUDO_PATH_BYTES];
        let mut length = 0usize;
        for byte in prefix.iter().chain(suffix.iter()) {
            let mapped = match byte {
                b'/' => b'_',
                b'a'..=b'z' => byte.to_ascii_uppercase(),
                b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' => *byte,
                _ => return Err(PseudoPathError::InvalidPath),
            };
            if length == logical.len() {
                return Err(PseudoPathError::TooLong);
            }
            logical[length] = mapped;
            length += 1;
        }
        let logical_name = core::str::from_utf8(&logical[..length])
            .map_err(|_| PseudoPathError::InvalidPath)?;
        LogicalName::new(logical_name).map_err(|_| PseudoPathError::InvalidPath)?;
        Ok(Self {
            kind,
            logical,
            logical_length: length as u8,
        })
    }

    pub const fn kind(self) -> PseudoResourceKind {
        self.kind
    }

    pub fn logical_name(self) -> LogicalName {
        LogicalName::new(self.logical_name_str()).expect("PseudoPath invariant")
    }

    fn logical_name_str(&self) -> &str {
        core::str::from_utf8(&self.logical[..self.logical_length as usize])
            .expect("PseudoPath invariant")
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
        assert_eq!(path.logical_name().as_str(), "PROC_MEMINFO");
    }

    #[test]
    fn rejects_pseudo_path_traversal() {
        assert_eq!(
            PseudoPath::parse(b"/sys/../dev/null"),
            Err(PseudoPathError::InvalidPath)
        );
    }
}
