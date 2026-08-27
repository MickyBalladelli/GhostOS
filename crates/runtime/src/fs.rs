use ghostos_ipc::SharedBuffer;

use crate::{Capability, Error, Operation, Request, Runtime, SystemCall};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenOptions(u16);

impl OpenOptions {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const CREATE: Self = Self(1 << 2);
    pub const TRUNCATE: Self = Self(1 << 3);
    pub const APPEND: Self = Self(1 << 4);
    pub const EXCLUSIVE: Self = Self(1 << 9);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn bits(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct File {
    capability: Capability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMapping {
    capability: Capability,
    offset: u64,
    length: u64,
    writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Metadata {
    pub length: u64,
    pub version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryRemovalMetadata {
    pub length: u64,
    pub version: u32,
    pub removal_generation: u64,
    pub storage_reclamation_pending: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkMetadata {
    pub length: u64,
    pub version: u32,
    pub link_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeleteMetadata {
    pub version: u32,
    pub file_type: u8,
    pub remaining_link_count: u32,
    pub shared_data_reachable: bool,
}

/// Bytes returned by `list_directory` use the bounded GhostFS directory wire
/// format starting at [`LIST_OUTPUT_OFFSET`]. The path prefix before that
/// offset remains intact while the caller follows the returned continuation.
pub const DIRECTORY_RECORD_HEADER_BYTES: usize = 22;

/// LIST keeps the requested path in this prefix of the shared buffer. The
/// directory records start here and remain intact across continuation calls.
pub const LIST_PATH_REGION_BYTES: usize = 192;
pub const LIST_OUTPUT_OFFSET: usize = LIST_PATH_REGION_BYTES;

/// Continuation ordinal for `SynFsList` lives in `arguments[4]` on both
/// filesystem syscall layouts.
pub const LIST_CONTINUATION_ARGUMENT: usize = 4;
/// Path prefix length for `SynFsList` lives in `arguments[5]` so leftover
/// listing bytes are not reused as a path.
pub const LIST_PATH_LENGTH_ARGUMENT: usize = 5;

/// Boot-service filesystem requests: pointer in `[0]`, length in `[1]`,
/// writable flag in `[2]`, continuation in `[4]`, LIST path length in `[5]`.
/// Runtime requests pack a `SharedBuffer` into `[0..3]` via `Request::with_buffer`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootFsBuffer {
    pub address: u64,
    pub length: u64,
    pub writable: u64,
    pub continuation: u64,
    pub path_length: u64,
}

/// Decode the boot-service raw-pointer filesystem layout.
pub const fn decode_boot_fs_buffer(arguments: &[u64; 6]) -> BootFsBuffer {
    BootFsBuffer {
        address: arguments[0],
        length: arguments[1],
        writable: arguments[2],
        continuation: arguments[4],
        path_length: arguments[5],
    }
}

/// Decode the runtime `SharedBuffer` descriptor packed in `arguments[0..3]`.
pub const fn decode_runtime_fs_buffer(arguments: &[u64; 6]) -> (u64, u64, u64, bool) {
    (
        arguments[0],
        arguments[1],
        arguments[2],
        arguments[3] != 0,
    )
}

/// Copy an absolute path into the LIST buffer and return the prefix length
/// that must be placed in `arguments[5]`.
pub fn seed_directory_path(buffer: &mut [u8], path: &[u8]) -> Result<u64, Error> {
    if path.is_empty()
        || path[0] != b'/'
        || path.len() > LIST_PATH_REGION_BYTES
        || buffer.len() <= LIST_PATH_REGION_BYTES
    {
        return Err(Error::InvalidResponse)
    }
    buffer[..path.len()].copy_from_slice(path);
    Ok(path.len() as u64)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PathBuffer {
    buffer: SharedBuffer,
}

impl PathBuffer {
    pub fn new(buffer: SharedBuffer) -> Result<Self, Error> {
        if buffer.length == 0 || buffer.length as usize > crate::pal::MAX_PAL_PATH_BYTES {
            return Err(Error::InvalidResponse);
        }
        if buffer.writable {
            return Err(Error::InvalidResponse);
        }
        Ok(Self { buffer })
    }

    pub const fn buffer(self) -> SharedBuffer {
        self.buffer
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryPage {
    pub bytes: usize,
    pub next: u64,
}

impl File {
    pub const fn capability(self) -> Capability {
        self.capability
    }
}

impl FileMapping {
    pub const fn capability(self) -> Capability {
        self.capability
    }

    pub const fn offset(self) -> u64 {
        self.offset
    }

    pub const fn length(self) -> u64 {
        self.length
    }

    pub const fn writable(self) -> bool {
        self.writable
    }
}

impl<S: SystemCall> Runtime<S> {
    pub fn open_path(&self, path: PathBuffer, options: OpenOptions) -> Result<File, Error> {
        self.open(path.buffer, options)
    }

    pub fn open(&self, path: SharedBuffer, options: OpenOptions) -> Result<File, Error> {
        let mut request = Request::new(Operation::SynFsOpen).with_buffer(path);
        request.flags = options.bits();
        let response = self.execute(request)?;
        let capability = Capability::from_raw(response.values[0]).ok_or(Error::InvalidResponse)?;
        Ok(File { capability })
    }

    pub fn close(&self, file: File) -> Result<(), Error> {
        self.execute(Request::new(Operation::SynFsClose).with_capability(file.capability))?;
        Ok(())
    }

    pub fn map_file(
        &self,
        file: File,
        offset: u64,
        length: u64,
        writable: bool,
    ) -> Result<FileMapping, Error> {
        let mut request = Request::new(Operation::SynFsMap).with_capability(file.capability);
        request.flags = writable as u16;
        request.arguments[4] = offset;
        request.arguments[5] = length;
        let response = self.execute(request)?;
        let capability = Capability::from_raw(response.values[0]).ok_or(Error::InvalidResponse)?;
        if response.values[1] != offset
            || response.values[2] != length
            || response.values[3] != writable as u64
        {
            return Err(Error::InvalidResponse);
        }
        Ok(FileMapping {
            capability,
            offset,
            length,
            writable,
        })
    }

    pub fn unmap_file(&self, mapping: FileMapping) -> Result<(), Error> {
        self.execute(Request::new(Operation::SynFsUnmap).with_capability(mapping.capability))?;
        Ok(())
    }

    pub fn read_at(&self, file: File, offset: u64, output: SharedBuffer) -> Result<usize, Error> {
        let mut request = Request::new(Operation::SynFsRead)
            .with_capability(file.capability)
            .with_buffer(output);
        request.arguments[4] = offset;
        let response = self.execute(request)?;
        usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)
    }

    /// Write bytes into the published GhostFS view. This is volatile until the
    /// filesystem owner completes the documented sync barrier.
    pub fn write_at(&self, file: File, offset: u64, input: SharedBuffer) -> Result<usize, Error> {
        let mut request = Request::new(Operation::SynFsWrite)
            .with_capability(file.capability)
            .with_buffer(input);
        request.arguments[4] = offset;
        let response = self.execute(request)?;
        usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)
    }

    pub fn metadata(&self, file: File) -> Result<Metadata, Error> {
        let response =
            self.execute(Request::new(Operation::SynFsMetadata).with_capability(file.capability))?;
        Ok(Metadata {
            length: response.values[0],
            version: u32::try_from(response.values[1]).map_err(|_| Error::InvalidResponse)?,
        })
    }

    pub fn create_directory(
        &self,
        path: SharedBuffer,
        recursive: bool,
    ) -> Result<Metadata, Error> {
        let mut request = Request::new(Operation::SynFsMkdir).with_buffer(path);
        request.flags = if recursive { 1 << 8 } else { 0 };
        let response = self.execute(request)?;
        Ok(Metadata {
            length: response.values[0],
            version: u32::try_from(response.values[1]).map_err(|_| Error::InvalidResponse)?,
        })
    }

    pub fn list_directory(
        &self,
        path_and_output: SharedBuffer,
        continuation: u64,
        path_length: u64,
    ) -> Result<DirectoryPage, Error> {
        if path_length == 0
            || path_length > LIST_PATH_REGION_BYTES as u64
            || path_and_output.length as usize <= LIST_PATH_REGION_BYTES
        {
            return Err(Error::InvalidResponse)
        }
        let mut request = Request::new(Operation::SynFsList).with_buffer(path_and_output);
        request.arguments[LIST_CONTINUATION_ARGUMENT] = continuation;
        request.arguments[LIST_PATH_LENGTH_ARGUMENT] = path_length;
        let response = self.execute(request)?;
        let bytes = usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)?;
        if bytes > path_and_output.length as usize - LIST_PATH_REGION_BYTES
            || response.values[1] > u64::from(u32::MAX)
        {
            return Err(Error::InvalidResponse)
        }
        Ok(DirectoryPage {
            bytes,
            next: response.values[1],
        })
    }

    pub fn create_file(&self, path: SharedBuffer) -> Result<(File, Metadata), Error> {
        let options = OpenOptions::READ
            .union(OpenOptions::WRITE)
            .union(OpenOptions::CREATE)
            .union(OpenOptions::EXCLUSIVE);
        let file = self.open(path, options)?;
        let metadata = self.metadata(file)?;
        Ok((file, metadata))
    }

    pub fn remove_directory(
        &self,
        path: SharedBuffer,
    ) -> Result<DirectoryRemovalMetadata, Error> {
        let response = self.execute(Request::new(Operation::SynFsRmdir).with_buffer(path))?;
        if response.values[3] > 1 {
            return Err(Error::InvalidResponse);
        }
        Ok(DirectoryRemovalMetadata {
            length: response.values[0],
            version: u32::try_from(response.values[1]).map_err(|_| Error::InvalidResponse)?,
            removal_generation: response.values[2],
            storage_reclamation_pending: response.values[3] != 0,
        })
    }

    pub fn delete(&self, path: SharedBuffer) -> Result<DeleteMetadata, Error> {
        let response = self.execute(Request::new(Operation::SynFsDelete).with_buffer(path))?;
        let file_type = u8::try_from(response.values[1]).map_err(|_| Error::InvalidResponse)?;
        if !matches!(file_type, 1..=3)
            || response.values[3] > 1
            || response.values[2] > u64::from(u32::MAX)
        {
            return Err(Error::InvalidResponse);
        }
        Ok(DeleteMetadata {
            version: u32::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)?,
            file_type,
            remaining_link_count: response.values[2] as u32,
            shared_data_reachable: response.values[3] != 0,
        })
    }

    pub fn link(&self, file: File, new_path: SharedBuffer) -> Result<(), Error> {
        self.link_metadata(file, new_path).map(|_| ())
    }

    pub fn link_metadata(
        &self,
        file: File,
        new_path: SharedBuffer,
    ) -> Result<LinkMetadata, Error> {
        let response = self.execute(
            Request::new(Operation::SynFsLink)
                .with_capability(file.capability)
                .with_buffer(new_path),
        )?;
        Ok(LinkMetadata {
            version: u32::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)?,
            length: response.values[1],
            link_count: u32::try_from(response.values[2]).map_err(|_| Error::InvalidResponse)?,
        })
    }

    pub fn list_links(&self, path_and_output: SharedBuffer) -> Result<usize, Error> {
        self.execute(
            Request::new(Operation::SynFsLinks).with_buffer(path_and_output),
        )
        .and_then(|response| {
            usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)
        })
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;

    use ghostos_ipc::{SharedRegionId, SharedBuffer};
    use ghostos_status::Status;

    use crate::Response;

    use super::*;

    struct MockSystemCall {
        request: Cell<Request>,
        response: Response,
    }

    impl SystemCall for MockSystemCall {
        fn call(&self, request: Request) -> Response {
            self.request.set(request);
            self.response
        }
    }

    fn buffer(writable: bool) -> SharedBuffer {
        SharedBuffer {
            region: SharedRegionId::new(3).expect("valid region"),
            offset: 8,
            length: 64,
            writable,
        }
    }

    fn list_buffer(writable: bool) -> SharedBuffer {
        SharedBuffer {
            region: SharedRegionId::new(3).expect("valid region"),
            offset: 8,
            length: (LIST_PATH_REGION_BYTES + 64) as u32,
            writable,
        }
    }

    #[test]
    fn link_metadata_marshals_handle_and_target_buffer() {
        let system = MockSystemCall {
            request: Cell::new(Request::new(Operation::Yield)),
            response: Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [4, 12, 2, 0],
            },
        };
        let runtime = Runtime::new(system);
        let file = File {
            capability: Capability::from_raw((9_u64 << 32) | 2).expect("valid capability"),
        };
        let metadata = runtime
            .link_metadata(file, buffer(false))
            .expect("link response");

        assert_eq!(metadata.version, 4);
        assert_eq!(metadata.length, 12);
        assert_eq!(metadata.link_count, 2);
        assert_eq!(runtime.system().request.get().operation, Operation::SynFsLink as u16);
        assert_eq!(runtime.system().request.get().capability, file.capability.raw());
        assert_eq!(runtime.system().request.get().arguments[3], 0);
    }

    #[test]
    fn list_links_requires_a_writable_output_buffer() {
        let system = MockSystemCall {
            request: Cell::new(Request::new(Operation::Yield)),
            response: Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [23, 0, 0, 0],
            },
        };
        let runtime = Runtime::new(system);
        assert_eq!(runtime.list_links(buffer(true)), Ok(23));
        let request = runtime.system().request.get();
        assert_eq!(request.operation, Operation::SynFsLinks as u16);
        assert_eq!(request.capability, 0);
        assert_eq!(request.arguments[3], 1);
    }

    #[test]
    fn delete_marshals_path_buffer_and_validates_metadata() {
        let system = MockSystemCall {
            request: Cell::new(Request::new(Operation::Yield)),
            response: Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [7, 1, 2, 1],
            },
        };
        let runtime = Runtime::new(system);
        let metadata = runtime.delete(buffer(false)).expect("delete response");

        assert_eq!(metadata.version, 7);
        assert_eq!(metadata.file_type, 1);
        assert_eq!(metadata.remaining_link_count, 2);
        assert!(metadata.shared_data_reachable);
        let request = runtime.system().request.get();
        assert_eq!(request.operation, Operation::SynFsDelete as u16);
        assert_eq!(request.capability, 0);
        assert_eq!(request.arguments[3], 0);

        let system = MockSystemCall {
            request: Cell::new(Request::new(Operation::Yield)),
            response: Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [u64::MAX, 4, 0, 0],
            },
        };
        let runtime = Runtime::new(system);
        assert_eq!(runtime.delete(buffer(false)), Err(Error::InvalidResponse));
    }

    #[test]
    fn list_directory_marshals_continuation_and_rejects_oversized_pages() {
        let system = MockSystemCall {
            request: Cell::new(Request::new(Operation::Yield)),
            response: Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [32, 7, 0, 0],
            },
        };
        let runtime = Runtime::new(system);
        let page = runtime
            .list_directory(list_buffer(true), 5, 1)
            .expect("directory page");
        assert_eq!(page.bytes, 32);
        assert_eq!(page.next, 7);
        let request = runtime.system().request.get();
        assert_eq!(request.operation, Operation::SynFsList as u16);
        assert_eq!(request.arguments[LIST_CONTINUATION_ARGUMENT], 5);
        assert_eq!(request.arguments[LIST_PATH_LENGTH_ARGUMENT], 1);
        let decoded = decode_runtime_fs_buffer(&request.arguments);
        assert_eq!(decoded.2, (LIST_PATH_REGION_BYTES + 64) as u64);
        assert!(decoded.3);

        let mut seeded = [0u8; LIST_PATH_REGION_BYTES + 64];
        assert_eq!(seed_directory_path(&mut seeded, b"/").unwrap(), 1);
        assert_eq!(&seeded[..1], b"/");
        assert_eq!(seed_directory_path(&mut seeded, b"relative"), Err(Error::InvalidResponse));

        let boot = decode_boot_fs_buffer(&[0x8000_0000, 4096, 1, 0, 9, 1]);
        assert_eq!(boot.address, 0x8000_0000);
        assert_eq!(boot.length, 4096);
        assert_eq!(boot.writable, 1);
        assert_eq!(boot.continuation, 9);
        assert_eq!(boot.path_length, 1);

        let system = MockSystemCall {
            request: Cell::new(Request::new(Operation::Yield)),
            response: Response {
                status: Status::NORMAL.raw(),
                flags: 0,
                values: [65, 0, 0, 0],
            },
        };
        let runtime = Runtime::new(system);
        assert_eq!(runtime.list_directory(list_buffer(true), 0, 1), Err(Error::InvalidResponse));
        assert_eq!(runtime.list_directory(list_buffer(true), 0, 193), Err(Error::InvalidResponse));
    }
}
