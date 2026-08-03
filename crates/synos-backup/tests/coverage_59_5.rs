use synos_backup::{BackupError, BackupJob, BackupSink, BackupState};
use synos_status::Status;
use synos_synfs::{RmsMapHandle, SynFs};

struct Sink {
    bytes: Vec<u8>,
    fail: bool,
}

impl BackupSink for Sink {
    fn append(&mut self, bytes: &[u8]) -> Result<(), Status> {
        if self.fail {
            return Err(Status::BUSY)
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn backup_streams_a_pinned_snapshot_and_releases_it() {
    let mut filesystem = SynFs::<128>::new();
    filesystem.write("/before", b"old").expect("write source file");
    let capability = RmsMapHandle::from_capability(1 << 32).unwrap();
    let mut job = BackupJob::start(&mut filesystem, capability).expect("start backup");
    filesystem.write("/before", b"new").expect("write after checkpoint");
    filesystem.write("/after", b"not in backup").expect("write later file");

    let mut sink = Sink {
        bytes: Vec::new(),
        fail: false,
    };
    loop {
        let progress = job.poll(&filesystem, &mut sink, 7).expect("poll backup");
        if progress.state == BackupState::Complete {
            break
        }
    }
    let report = job.finish(&mut filesystem).expect("finish backup");
    assert_eq!(report.files_streamed, 1);
    assert!(sink.bytes.starts_with(b"SYNBACK1"));
    assert!(sink.bytes.windows(3).any(|window| window == b"old"));
    assert!(!sink.bytes.windows(3).any(|window| window == b"new"));
    assert!(filesystem.diagnostics().unwrap().checkpoints == 0);
}

#[test]
fn backup_rejects_invalid_budget_and_sink_failure_without_losing_checkpoint() {
    let mut filesystem = SynFs::<64>::new();
    filesystem.write("/file", b"data").unwrap();
    let capability = RmsMapHandle::from_capability(1 << 32).unwrap();
    let mut job = BackupJob::start(&mut filesystem, capability).unwrap();
    let mut sink = Sink {
        bytes: Vec::new(),
        fail: true,
    };
    assert_eq!(job.poll(&filesystem, &mut sink, 0), Err(BackupError::InvalidBudget));
    assert!(matches!(job.poll(&filesystem, &mut sink, 8), Err(BackupError::Sink(_))));
    assert_eq!(filesystem.diagnostics().unwrap().checkpoints, 1);
    job.cancel(&mut filesystem).expect("cancel failed backup");
    assert_eq!(filesystem.diagnostics().unwrap().checkpoints, 0);
}
