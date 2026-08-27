use ghostos_rms::{
    Database, DatabaseError, DlmBinding, DlmLockMode, DlmLockRange, DlmRecordLocks,
    RecordDescriptor, RecordError, RecordFile, RecordFormat, RecordOrganization, RecordSelector,
    RmsError,
};
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::SynFs;

const BLOCKS: usize = 96;

#[test]
fn indexed_records_support_persistence_style_crud_and_corruption_checks() {
    let descriptor = RecordDescriptor {
        organization: RecordOrganization::Indexed(ghostos_rms::IndexDefinition {
            key_offset: 0,
            key_length: 1,
            unique: true,
        }),
        format: RecordFormat::Fixed { length: 4 },
    };
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem.create_directory("/records", true).expect("create RMS parent");
    let mut scratch = [0; 512];
    let (mut file, info) = RecordFile::create(
        &mut filesystem,
        "/records/index",
        descriptor,
        &[b"a001", b"b002"],
        &mut scratch,
    )
    .expect("create indexed record file");
    assert_eq!(info.record_count, 2);
    let mut output = [0; 4];
    let read = file
        .read(RecordSelector::Key(b"b"), &mut scratch, &mut output)
        .expect("read indexed record");
    assert_eq!(read.position, 1);
    assert_eq!(&output, b"b002");

    let mut image_scratch = [0; 512];
    file.insert(b"c003", &mut scratch, &mut image_scratch)
        .expect("insert record");
    assert_eq!(file.resolve(RecordSelector::Key(b"c"), &mut scratch).unwrap().position, 2);
    assert_eq!(
        file.insert(b"c999", &mut scratch, &mut image_scratch),
        Err(RecordError::Storage(RmsError::DuplicateKey))
    );
    file.update(
        RecordSelector::Key(b"a"),
        b"a900",
        &mut scratch,
        &mut image_scratch,
    )
    .expect("update record");
    file.delete(RecordSelector::Key(b"b"), &mut scratch, &mut image_scratch)
        .expect("delete record");
    assert_eq!(file.info(&mut scratch).unwrap().record_count, 2);

    let mut image = [0; 512];
    let info = file.info(&mut image).unwrap();
    drop(file);
    let mut bytes = vec![0; info.file.size as usize];
    filesystem.read("/records/index", &mut bytes).expect("read record image");
    bytes[0] ^= 1;
    assert!(matches!(
        ghostos_rms::MappedRecordFile::open(
            ghostos_rms::RmsMapHandle::from_capability(1 << 32).unwrap(),
            &bytes,
        ),
        Err(RmsError::NotRecordFile | RmsError::File(_))
    ));
}

#[test]
fn database_transactions_and_key_boundaries_are_deterministic() {
    let mut filesystem = SynFs::<BLOCKS>::new();
    filesystem
        .create_directory("/.ghostos/data/users", true)
        .expect("create database root");
    let mut database = Database::open(&mut filesystem, "users").expect("open database");
    assert_eq!(database.put(b"alice", b"one").unwrap(), 1);
    let mut value = [0; 3];
    assert_eq!(database.get(b"alice", &mut value).unwrap(), 3);
    assert_eq!(&value, b"one");
    assert!(database.contains(b"alice").unwrap());

    let mut transaction = database.transaction::<2>();
    transaction.put(b"bob", b"two").unwrap();
    transaction.delete(b"alice").unwrap();
    assert_eq!(transaction.len(), 2);
    transaction.commit().expect("commit database transaction");
    assert!(!database.contains(b"alice").unwrap());
    assert_eq!(database.get(b"bob", &mut value).unwrap(), 3);
    assert_eq!(database.get(b"", &mut value), Err(DatabaseError::EmptyKey));
}

#[derive(Default)]
struct LockBackend {
    next: u64,
    released: Vec<u64>,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LockError {
    Failed,
}

impl IntoStatus for LockError {
    fn status(self) -> Status {
        Status::BUSY
    }
}

impl DlmBinding for LockBackend {
    type Error = LockError;
    type Handle = u64;

    fn acquire(
        &mut self,
        _resource: ghostos_rms::DlmResource,
        _range: DlmLockRange,
        _mode: DlmLockMode,
        _wait: bool,
    ) -> Result<Self::Handle, Self::Error> {
        self.next += 1;
        Ok(self.next)
    }

    fn release(&mut self, handle: Self::Handle) -> Result<(), Self::Error> {
        self.released.push(handle);
        Ok(())
    }
}

#[test]
fn record_locks_release_on_explicit_release_and_drop() {
    let mut locks = DlmRecordLocks::new(LockBackend::default());
    let guard = locks
        .lock("/records/index", DlmLockRange::Record(2), DlmLockMode::Update, false)
        .expect("acquire record lock");
    guard.release().expect("release record lock");
    assert_eq!(locks.binding().released, vec![1]);
    assert!(matches!(
        locks.lock("", DlmLockRange::WholeFile, DlmLockMode::Read, true),
        Err(ghostos_rms::RecordLockError::InvalidPath)
    ));
    {
        let _guard = locks
            .lock("/records/index", DlmLockRange::WholeFile, DlmLockMode::Read, true)
            .expect("acquire second lock");
    }
    assert_eq!(locks.binding().released, vec![1, 2]);
}
