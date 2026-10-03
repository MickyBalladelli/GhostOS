use core::str;
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{
    Error, MAX_PATH_BYTES, MappedFilePage, ReadOnlySnapshot, RmsMapHandle, SynFs, TransactionCommit,
};

pub const MAX_DATABASE_NAMESPACE_BYTES: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Namespace {
    bytes: [u8; MAX_DATABASE_NAMESPACE_BYTES],
    len: u8,
}

impl Namespace {
    fn new(value: &str) -> Result<Self, DatabaseError> {
        let source = value.as_bytes();
        if crate::native::namespace(source).is_err() {
            return Err(DatabaseError::InvalidNamespace);
        }
        let mut bytes = [0; MAX_DATABASE_NAMESPACE_BYTES];
        bytes[..source.len()].copy_from_slice(source);
        Ok(Self {
            bytes,
            len: source.len() as u8,
        })
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseError {
    Capacity,
    EmptyKey,
    Filesystem(Error),
    InvalidNamespace,
    KeyTooLong,
}

impl From<Error> for DatabaseError {
    fn from(error: Error) -> Self {
        Self::Filesystem(error)
    }
}

impl IntoStatus for DatabaseError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::EmptyKey | Self::InvalidNamespace | Self::KeyTooLong => Status::INVALID_ARGUMENT,
            Self::Filesystem(error) => error.status(),
        }
    }
}

pub struct Database<'fs, const MAX_BLOCKS: usize> {
    filesystem: &'fs mut SynFs<MAX_BLOCKS>,
    namespace: Namespace,
}

impl<'fs, const MAX_BLOCKS: usize> Database<'fs, MAX_BLOCKS> {
    pub fn open(
        filesystem: &'fs mut SynFs<MAX_BLOCKS>,
        namespace: &str,
    ) -> Result<Self, DatabaseError> {
        Ok(Self {
            filesystem,
            namespace: Namespace::new(namespace)?,
        })
    }

    pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<u32, DatabaseError> {
        let mut path = [0; MAX_PATH_BYTES];
        let path = key_path(self.namespace, key, &mut path)?;
        Ok(self.filesystem.write(path, value)?.version)
    }

    pub fn get(&self, key: &[u8], destination: &mut [u8]) -> Result<usize, DatabaseError> {
        let mut path = [0; MAX_PATH_BYTES];
        let path = key_path(self.namespace, key, &mut path)?;
        Ok(self.filesystem.read(path, destination)?.bytes_read)
    }

    pub fn contains(&self, key: &[u8]) -> Result<bool, DatabaseError> {
        let mut path = [0; MAX_PATH_BYTES];
        let path = key_path(self.namespace, key, &mut path)?;
        match self.filesystem.lookup(path) {
            Ok(_) => Ok(true),
            Err(Error::NotFound) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<u32, DatabaseError> {
        let mut path = [0; MAX_PATH_BYTES];
        let path = key_path(self.namespace, key, &mut path)?;
        Ok(self.filesystem.delete(path)?.version)
    }

    pub fn transaction<const OPERATIONS: usize>(
        &mut self,
    ) -> DatabaseTransaction<'_, 'fs, '_, MAX_BLOCKS, OPERATIONS> {
        DatabaseTransaction {
            database: self,
            operations: [None; OPERATIONS],
            len: 0,
        }
    }

    pub fn mapped(&self, capability: RmsMapHandle) -> MappedDatabase<'_, MAX_BLOCKS> {
        MappedDatabase {
            snapshot: self.filesystem.mapped_snapshot(capability),
            namespace: self.namespace,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseOperation<'a> {
    Put { key: &'a [u8], value: &'a [u8] },
    Delete { key: &'a [u8] },
}

pub struct DatabaseTransaction<'db, 'fs, 'data, const MAX_BLOCKS: usize, const OPERATIONS: usize> {
    database: &'db mut Database<'fs, MAX_BLOCKS>,
    operations: [Option<DatabaseOperation<'data>>; OPERATIONS],
    len: usize,
}

impl<'data, const MAX_BLOCKS: usize, const OPERATIONS: usize>
    DatabaseTransaction<'_, '_, 'data, MAX_BLOCKS, OPERATIONS>
{
    pub fn put(&mut self, key: &'data [u8], value: &'data [u8]) -> Result<(), DatabaseError> {
        self.push(DatabaseOperation::Put { key, value })
    }

    pub fn delete(&mut self, key: &'data [u8]) -> Result<(), DatabaseError> {
        self.push(DatabaseOperation::Delete { key })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn commit(self) -> Result<TransactionCommit, DatabaseError> {
        let namespace = self.database.namespace;
        let mut transaction = self.database.filesystem.transaction();
        for operation in self.operations[..self.len].iter().flatten() {
            let mut path = [0; MAX_PATH_BYTES];
            match operation {
                DatabaseOperation::Put { key, value } => {
                    let path = key_path(namespace, key, &mut path)?;
                    transaction.write(path, value)?;
                }
                DatabaseOperation::Delete { key } => {
                    let path = key_path(namespace, key, &mut path)?;
                    transaction.delete(path)?;
                }
            }
        }
        Ok(transaction.commit()?)
    }

    fn push(&mut self, operation: DatabaseOperation<'data>) -> Result<(), DatabaseError> {
        let index = crate::native::reserve(self.len, OPERATIONS).map_err(|_| DatabaseError::Capacity)?;
        self.operations[index] = Some(operation);
        self.len += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValuePage<'a> {
    pub value_offset: u64,
    pub bytes: &'a [u8],
    pub checksum: u64,
}

pub struct MappedDatabase<'a, const MAX_BLOCKS: usize> {
    snapshot: ReadOnlySnapshot<'a, MAX_BLOCKS>,
    namespace: Namespace,
}

impl<'a, const MAX_BLOCKS: usize> MappedDatabase<'a, MAX_BLOCKS> {
    pub const fn generation(&self) -> u64 {
        self.snapshot.generation()
    }

    pub fn visit_value(
        &self,
        key: &[u8],
        mut visitor: impl FnMut(ValuePage<'a>),
    ) -> Result<u64, DatabaseError> {
        let mut path = [0; MAX_PATH_BYTES];
        let path = key_path(self.namespace, key, &mut path)?;
        let file = self
            .snapshot
            .visit_file_pages(path, |page: MappedFilePage<'a>| {
                visitor(ValuePage {
                    value_offset: page.file_offset,
                    bytes: page.bytes,
                    checksum: page.checksum,
                })
            })?;
        Ok(file.size)
    }
}

fn key_path<'a>(
    namespace: Namespace,
    key: &[u8],
    destination: &'a mut [u8; MAX_PATH_BYTES],
) -> Result<&'a str, DatabaseError> {
    let written = match crate::native::key_path(namespace.as_bytes(), key, destination) {
        Ok(written) => written,
        Err(1) => return Err(DatabaseError::EmptyKey),
        Err(_) => return Err(DatabaseError::KeyTooLong),
    };
    str::from_utf8(&destination[..written]).map_err(|_| DatabaseError::KeyTooLong)
}
