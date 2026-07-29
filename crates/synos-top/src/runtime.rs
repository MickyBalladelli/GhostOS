use crate::{SnapshotError, TopologySnapshot};

pub trait TopologySource<const NODES: usize, const CAPABILITIES: usize> {
    type Error;

    /// Replace the contents of `snapshot` with one coherent live sample.
    fn sample(
        &mut self,
        now_us: u64,
        snapshot: &mut TopologySnapshot<NODES, CAPABILITIES>,
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Poll<E> {
    Waiting,
    Updated { generation: u64 },
    InvalidSnapshot(SnapshotError),
    Failed(E),
}

/// Fixed-rate sampler used by terminal and framebuffer event loops.
pub struct RealtimeMonitor<
    Source,
    const NODES: usize,
    const CAPABILITIES: usize,
> {
    source: Source,
    snapshot: TopologySnapshot<NODES, CAPABILITIES>,
    interval_us: u64,
    next_sample_us: u64,
}

impl<Source, const NODES: usize, const CAPABILITIES: usize>
    RealtimeMonitor<Source, NODES, CAPABILITIES>
{
    pub fn new(source: Source, interval_us: u64, now_us: u64) -> Option<Self> {
        if interval_us == 0 {
            return None
        }
        Some(Self {
            source,
            snapshot: TopologySnapshot::new(),
            interval_us,
            next_sample_us: now_us,
        })
    }

    pub const fn snapshot(&self) -> &TopologySnapshot<NODES, CAPABILITIES> {
        &self.snapshot
    }

    pub const fn source(&self) -> &Source {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut Source {
        &mut self.source
    }

    pub const fn interval_us(&self) -> u64 {
        self.interval_us
    }

    pub fn set_interval_us(&mut self, interval_us: u64) -> bool {
        if interval_us == 0 {
            return false
        }
        self.interval_us = interval_us;
        true
    }
}

impl<Source, const NODES: usize, const CAPABILITIES: usize>
    RealtimeMonitor<Source, NODES, CAPABILITIES>
where
    Source: TopologySource<NODES, CAPABILITIES>,
{
    pub fn poll(&mut self, now_us: u64) -> Poll<Source::Error> {
        if now_us < self.next_sample_us {
            return Poll::Waiting
        }

        let mut next = self.snapshot;
        next.begin();
        if let Err(error) = self.source.sample(now_us, &mut next) {
            self.next_sample_us = now_us.saturating_add(self.interval_us);
            return Poll::Failed(error)
        }
        let generation = match next.finish(now_us) {
            Ok(generation) => generation,
            Err(error) => {
                self.next_sample_us = now_us.saturating_add(self.interval_us);
                return Poll::InvalidSnapshot(error)
            }
        };
        self.snapshot = next;
        self.next_sample_us = now_us.saturating_add(self.interval_us);
        Poll::Updated { generation }
    }
}
