#![no_std]
#![forbid(unsafe_code)]

mod database;
mod locking;
mod record;

pub use database::{
    Database, DatabaseError, DatabaseOperation, DatabaseTransaction, MAX_DATABASE_NAMESPACE_BYTES,
    MappedDatabase, ValuePage,
};
pub use locking::{
    DlmBinding, DlmLockMode, DlmLockRange, DlmRecordLocks, DlmResource, RecordLockError,
    RecordLockGuard,
};
pub use record::{RecordError, RecordFile, RecordLocation, StructuredRecord};
pub use synos_synfs::{
    IndexDefinition, MappedRecordFile, RecordDescriptor, RecordFileInfo, RecordFormat,
    RecordOrganization, RecordRead, RecordSelector, RmsError, RmsMapHandle, TransactionCommit,
};
