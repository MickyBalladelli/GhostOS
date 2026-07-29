use synos_fabric::NodeId;

use crate::SnapshotError;

const LATENCY_BUCKETS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsmLatencySample {
    pub faults: u64,
    pub p50_ns: u32,
    pub p99_ns: u32,
}

#[derive(Clone, Copy)]
struct NodeLatency {
    node: NodeId,
    faults: u64,
    buckets: [u64; LATENCY_BUCKETS],
}

impl NodeLatency {
    const fn new(node: NodeId) -> Self {
        Self {
            node,
            faults: 0,
            buckets: [0; LATENCY_BUCKETS],
        }
    }

    fn record(&mut self, latency_ns: u32) {
        let bucket = if latency_ns == 0 {
            0
        } else {
            (u32::BITS - 1 - latency_ns.leading_zeros()) as usize
        };
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.faults = self.faults.saturating_add(1)
    }

    fn sample(&self) -> DsmLatencySample {
        DsmLatencySample {
            faults: self.faults,
            p50_ns: self.percentile(50),
            p99_ns: self.percentile(99),
        }
    }

    fn percentile(&self, percentile: u64) -> u32 {
        if self.faults == 0 {
            return 0
        }
        let target = self
            .faults
            .saturating_mul(percentile)
            .saturating_add(99)
            / 100;
        let mut cumulative = 0u64;
        for (bucket, count) in self.buckets.iter().copied().enumerate() {
            cumulative = cumulative.saturating_add(count);
            if cumulative >= target {
                return bucket_upper_bound(bucket)
            }
        }
        u32::MAX
    }
}

/// Heap-free, logarithmic histogram of remote DSM fault latency per node.
///
/// `take` resets one node after publishing an interval, so dashboard
/// percentiles track current fabric behavior instead of lifetime averages.
pub struct DsmLatencyTracker<const NODES: usize> {
    nodes: [Option<NodeLatency>; NODES],
}

impl<const NODES: usize> DsmLatencyTracker<NODES> {
    pub const fn new() -> Self {
        Self {
            nodes: [None; NODES],
        }
    }

    pub fn record(
        &mut self,
        node: NodeId,
        latency_ns: u32,
    ) -> Result<(), SnapshotError> {
        let index = match self
            .nodes
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.node == node))
        {
            Some(index) => index,
            None => {
                let index = self
                    .nodes
                    .iter()
                    .position(Option::is_none)
                    .ok_or(SnapshotError::Capacity)?;
                self.nodes[index] = Some(NodeLatency::new(node));
                index
            }
        };
        self.nodes[index]
            .as_mut()
            .expect("latency slot initialized")
            .record(latency_ns);
        Ok(())
    }

    pub fn sample(&self, node: NodeId) -> Option<DsmLatencySample> {
        self.nodes
            .iter()
            .flatten()
            .find(|entry| entry.node == node)
            .map(NodeLatency::sample)
    }

    pub fn take(&mut self, node: NodeId) -> Option<DsmLatencySample> {
        let entry = self
            .nodes
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry.node == node))?;
        let sample = entry.as_ref()?.sample();
        *entry = None;
        Some(sample)
    }

    pub fn clear(&mut self) {
        self.nodes.fill(None)
    }
}

impl<const NODES: usize> Default for DsmLatencyTracker<NODES> {
    fn default() -> Self {
        Self::new()
    }
}

const fn bucket_upper_bound(bucket: usize) -> u32 {
    if bucket >= 31 {
        u32::MAX
    } else {
        ((1u64 << (bucket + 1)) - 1) as u32
    }
}
