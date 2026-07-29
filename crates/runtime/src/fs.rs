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
}
