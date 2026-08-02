use synos_ipc::SharedBuffer;

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
pub struct Metadata {
    pub length: u64,
    pub version: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinkMetadata {
    pub length: u64,
    pub version: u32,
    pub link_count: u32,
}

/// Bytes returned by `list_directory` use the bounded SynFS directory wire
/// format. The caller owns the shared buffer and can decode each record while
/// following the returned continuation offset.
pub const DIRECTORY_RECORD_HEADER_BYTES: usize = 22;

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

impl<S: SystemCall> Runtime<S> {
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

    pub fn read_at(&self, file: File, offset: u64, output: SharedBuffer) -> Result<usize, Error> {
        let mut request = Request::new(Operation::SynFsRead)
            .with_capability(file.capability)
            .with_buffer(output);
        request.arguments[4] = offset;
        let response = self.execute(request)?;
        usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)
    }

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
    ) -> Result<DirectoryPage, Error> {
        let mut request = Request::new(Operation::SynFsList).with_buffer(path_and_output);
        request.arguments[4] = continuation;
        let response = self.execute(request)?;
        Ok(DirectoryPage {
            bytes: usize::try_from(response.values[0]).map_err(|_| Error::InvalidResponse)?,
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

    pub fn remove_directory(&self, path: SharedBuffer) -> Result<(), Error> {
        self.execute(Request::new(Operation::SynFsRmdir).with_buffer(path))?;
        Ok(())
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

    use synos_ipc::{SharedRegionId, SharedBuffer};
    use synos_status::Status;

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
}
