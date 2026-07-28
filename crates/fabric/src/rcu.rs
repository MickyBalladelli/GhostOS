use core::sync::atomic::{AtomicU64, Ordering};

use crate::Error;

pub const DEFAULT_RCU_READERS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RcuSnapshot<T: Copy> {
    pub epoch: u64,
    pub value: T,
}

#[derive(Clone, Copy)]
struct Retired<T: Copy> {
    epoch: u64,
    value: T,
}

/// Fixed-storage epoch RCU for read-heavy cluster state.
///
/// Readers pin an epoch and copy an immutable snapshot without taking a lock.
/// A writer publishes a replacement and retains the prior value until every
/// registered reader reports a quiescent or newer epoch.
pub struct EpochRcu<T: Copy, const READERS: usize = DEFAULT_RCU_READERS> {
    current: T,
    retired: Option<Retired<T>>,
    epoch: AtomicU64,
    readers: [AtomicU64; READERS],
}

impl<T: Copy, const READERS: usize> EpochRcu<T, READERS> {
    pub fn new(initial: T) -> Self {
        Self {
            current: initial,
            retired: None,
            epoch: AtomicU64::new(1),
            readers: core::array::from_fn(|_| AtomicU64::new(0)),
        }
    }

    pub fn read(&self, reader: usize) -> Result<RcuSnapshot<T>, Error> {
        let slot = self.readers.get(reader).ok_or(Error::InvalidRange)?;
        loop {
            let epoch = self.epoch.load(Ordering::Acquire);
            slot.store(epoch, Ordering::Release);
            let value = self.current;
            if self.epoch.load(Ordering::Acquire) == epoch {
                return Ok(RcuSnapshot { epoch, value });
            }
        }
    }

    pub fn quiescent(&self, reader: usize) -> Result<(), Error> {
        self.readers
            .get(reader)
            .ok_or(Error::InvalidRange)?
            .store(0, Ordering::Release);
        Ok(())
    }

    pub fn publish(&mut self, next: T) -> Result<u64, Error> {
        self.collect();
        if self.retired.is_some() {
            return Err(Error::Busy);
        }
        let old_epoch = self.epoch.load(Ordering::Relaxed);
        let old = self.current;
        self.current = next;
        let new_epoch = old_epoch.wrapping_add(1).max(1);
        self.retired = Some(Retired {
            epoch: old_epoch,
            value: old,
        });
        self.epoch.store(new_epoch, Ordering::Release);
        Ok(new_epoch)
    }

    pub fn collect(&mut self) -> Option<T> {
        let retired = self.retired?;
        let safe = self.readers.iter().all(|reader| {
            let epoch = reader.load(Ordering::Acquire);
            epoch == 0 || epoch > retired.epoch
        });
        if !safe {
            return None;
        }
        self.retired = None;
        Some(retired.value)
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
}

impl<T: Copy + Default, const READERS: usize> Default for EpochRcu<T, READERS> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
