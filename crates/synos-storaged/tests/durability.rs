use synos_storaged::{CacheMode, CowCache, RemoteFileBackend, StoragePath};

struct Backend {
    events: Vec<&'static str>,
    fail_flush: bool,
}

impl Backend {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            fail_flush: false,
        }
    }
}

impl RemoteFileBackend for Backend {
    fn read(&mut self, _path: StoragePath, _destination: &mut [u8]) -> Result<usize, u16> {
        Ok(0)
    }

    fn write(&mut self, _path: StoragePath, _contents: &[u8]) -> Result<(), u16> {
        self.events.push("write");
        Ok(())
    }

    fn flush(&mut self) -> Result<(), u16> {
        self.events.push("flush");
        if self.fail_flush { Err(7) } else { Ok(()) }
    }
}

#[test]
fn cache_flush_fences_remote_write_before_clearing_dirty_state() {
    let path = StoragePath::new("SYS$STORAGE:REMOTE/state").unwrap();
    let mut backend = Backend::new();
    let mut cache = CowCache::<2, 32>::new(CacheMode::CopyOnWrite);
    cache.write(&mut backend, path, b"state").unwrap();
    assert_eq!(cache.dirty_entries(), 1);

    assert_eq!(cache.flush(&mut backend), Ok(1));
    assert_eq!(backend.events, ["write", "flush"]);
    assert_eq!(cache.dirty_entries(), 0);
}

#[test]
fn failed_remote_fence_keeps_dirty_copy_for_retry() {
    let path = StoragePath::new("SYS$STORAGE:REMOTE/state").unwrap();
    let mut backend = Backend::new();
    backend.fail_flush = true;
    let mut cache = CowCache::<2, 32>::new(CacheMode::CopyOnWrite);
    cache.write(&mut backend, path, b"state").unwrap();

    assert_eq!(cache.flush(&mut backend), Err(synos_storaged::CacheError::Remote(7)));
    assert_eq!(cache.dirty_entries(), 1);
}
