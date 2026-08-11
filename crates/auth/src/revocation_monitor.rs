//! Bounded revocation propagation and cache-fencing monitor.
//!
//! A cache may keep bytes from an old authorization decision, but it must not
//! keep authority from an old epoch. The monitor records one propagation event
//! per revocation, measures each cache's acknowledgement latency, and fences
//! refresh, reconnect, and authorization until the cache has observed the
//! current epoch.

use crate::federation::MAX_REVOCATION_LATENCY_US;

pub const REVOCATION_CACHE_COUNT: usize = 7;
pub const DEFAULT_REVOCATION_EVENT_CAPACITY: usize = 64;

const ALL_CACHE_BITS: u8 = (1 << REVOCATION_CACHE_COUNT) - 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RevocationCache {
    Kernel = 0,
    Ipc = 1,
    Storage = 2,
    Network = 3,
    Package = 4,
    Cluster = 5,
    Client = 6,
}

impl RevocationCache {
    pub const ALL: [Self; REVOCATION_CACHE_COUNT] = [
        Self::Kernel,
        Self::Ipc,
        Self::Storage,
        Self::Network,
        Self::Package,
        Self::Cluster,
        Self::Client,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Ipc => "ipc",
            Self::Storage => "storage",
            Self::Network => "network",
            Self::Package => "package",
            Self::Cluster => "cluster",
            Self::Client => "client",
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevocationKey {
    pub subject: u64,
    pub object: u64,
}

impl RevocationKey {
    pub const fn new(subject: u64, object: u64) -> Option<Self> {
        if subject == 0 || object == 0 {
            None
        } else {
            Some(Self { subject, object })
        }
    }

    const fn is_valid(self) -> bool {
        self.subject != 0 && self.object != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevocationNotice {
    pub key: RevocationKey,
    pub epoch: u64,
    pub revoked_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PropagationObservation {
    pub cache: RevocationCache,
    pub key: RevocationKey,
    pub epoch: u64,
    pub latency_us: u64,
    pub late: bool,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheReport {
    pub cache: RevocationCache,
    pub observed_events: u64,
    pub pending_events: u64,
    pub stale_rejections: u64,
    pub maximum_latency_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PropagationReport {
    pub measured_at_us: u64,
    pub total_events: u64,
    pub completed_events: u64,
    pub pending_events: u64,
    pub maximum_latency_us: u64,
    pub observed_maximum_latency_us: u64,
    pub pending_maximum_age_us: u64,
    pub late_observations: u64,
    pub stale_rejections: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevocationMonitorError {
    Invalid,
    Capacity,
    EpochRegression,
    UnknownRevocation,
    StaleEpoch,
    FutureEpoch,
    CacheNotSynchronized,
}

#[derive(Clone, Copy)]
struct RevocationEvent {
    notice: RevocationNotice,
    observed: u8,
    maximum_latency_us: u64,
    maximum_latency_by_cache_us: [u64; REVOCATION_CACHE_COUNT],
    late_observations: u64,
}

/// Fixed-capacity monitor shared by the authorization and cache paths.
///
/// `refresh`, `reconnect`, and `authorize` all reject an epoch older than the
/// latest observed revocation. A cache therefore cannot turn a stale local
/// entry into live authority by refreshing or reconnecting.
pub struct RevocationMonitor<const CAPACITY: usize = DEFAULT_REVOCATION_EVENT_CAPACITY> {
    events: [Option<RevocationEvent>; CAPACITY],
    maximum_allowed_latency_us: u64,
    total_events: u64,
    late_observations: u64,
    stale_rejections: u64,
    cache_stale_rejections: [u64; REVOCATION_CACHE_COUNT],
}

impl<const CAPACITY: usize> RevocationMonitor<CAPACITY> {
    pub const fn new(maximum_allowed_latency_us: u64) -> Self {
        assert!(CAPACITY >= 1);
        assert!(maximum_allowed_latency_us > 0);
        Self {
            events: [None; CAPACITY],
            maximum_allowed_latency_us,
            total_events: 0,
            late_observations: 0,
            stale_rejections: 0,
            cache_stale_rejections: [0; REVOCATION_CACHE_COUNT],
        }
    }

    pub const fn maximum_allowed_latency_us(&self) -> u64 {
        self.maximum_allowed_latency_us
    }

    pub fn revoke(
        &mut self,
        key: RevocationKey,
        epoch: u64,
        revoked_at_us: u64,
    ) -> Result<RevocationNotice, RevocationMonitorError> {
        if !key.is_valid() || epoch == 0 || revoked_at_us == 0 {
            return Err(RevocationMonitorError::Invalid)
        }
        if self.latest_epoch(key).is_some_and(|current| epoch <= current) {
            return Err(RevocationMonitorError::EpochRegression)
        }
        let slot = self
            .events
            .iter_mut()
            .find(|event| event.is_none())
            .ok_or(RevocationMonitorError::Capacity)?;
        let notice = RevocationNotice {
            key,
            epoch,
            revoked_at_us,
        };
        *slot = Some(RevocationEvent {
            notice,
            observed: 0,
            maximum_latency_us: 0,
            maximum_latency_by_cache_us: [0; REVOCATION_CACHE_COUNT],
            late_observations: 0,
        });
        self.total_events = self.total_events.saturating_add(1);
        Ok(notice)
    }

    /// Record a cache's first observation of a revocation. Replays are
    /// idempotent so retrying a delivered signal cannot inflate latency.
    pub fn observe(
        &mut self,
        cache: RevocationCache,
        notice: RevocationNotice,
        now_us: u64,
    ) -> Result<PropagationObservation, RevocationMonitorError> {
        if now_us < notice.revoked_at_us {
            return Err(RevocationMonitorError::Invalid)
        }
        if self
            .latest_epoch(notice.key)
            .is_some_and(|epoch| notice.epoch < epoch)
        {
            return Err(RevocationMonitorError::StaleEpoch)
        }
        let index = self
            .event_index(notice)
            .ok_or(RevocationMonitorError::UnknownRevocation)?;
        let event = self.events[index].as_mut().expect("validated event index");
        let bit = cache.bit();
        let latency_us = now_us.saturating_sub(notice.revoked_at_us);
        if event.observed & bit == 0 {
            event.observed |= bit;
            event.maximum_latency_us = event.maximum_latency_us.max(latency_us);
            event.maximum_latency_by_cache_us[cache.index()] = latency_us;
            if latency_us > self.maximum_allowed_latency_us {
                event.late_observations = event.late_observations.saturating_add(1);
                self.late_observations = self.late_observations.saturating_add(1);
            }
        }
        Ok(PropagationObservation {
            cache,
            key: notice.key,
            epoch: notice.epoch,
            latency_us,
            late: latency_us > self.maximum_allowed_latency_us,
            complete: event.observed == ALL_CACHE_BITS,
        })
    }

    /// Refresh a cache from its local epoch. Old entries are rejected before
    /// they can be used, and a current entry is recorded as propagated.
    pub fn refresh(
        &mut self,
        cache: RevocationCache,
        key: RevocationKey,
        cached_epoch: u64,
        now_us: u64,
    ) -> Result<(), RevocationMonitorError> {
        self.sync(cache, key, cached_epoch, now_us)
    }

    /// Reconnect has the same fence as refresh; transport recovery cannot
    /// restore a stale authorization cache.
    pub fn reconnect(
        &mut self,
        cache: RevocationCache,
        key: RevocationKey,
        cached_epoch: u64,
        now_us: u64,
    ) -> Result<(), RevocationMonitorError> {
        self.sync(cache, key, cached_epoch, now_us)
    }

    pub fn authorize(
        &mut self,
        cache: RevocationCache,
        key: RevocationKey,
        lease_epoch: u64,
    ) -> Result<(), RevocationMonitorError> {
        let Some(index) = self.latest_event_index(key) else {
            if lease_epoch == 0 {
                return Err(RevocationMonitorError::Invalid)
            }
            return Ok(())
        };
        let event = self.events[index].expect("validated event index");
        if lease_epoch < event.notice.epoch {
            self.stale_rejections = self.stale_rejections.saturating_add(1);
            self.cache_stale_rejections[cache.index()] =
                self.cache_stale_rejections[cache.index()].saturating_add(1);
            return Err(RevocationMonitorError::StaleEpoch)
        }
        if lease_epoch > event.notice.epoch {
            return Err(RevocationMonitorError::FutureEpoch)
        }
        if event.observed & cache.bit() == 0 {
            self.stale_rejections = self.stale_rejections.saturating_add(1);
            self.cache_stale_rejections[cache.index()] =
                self.cache_stale_rejections[cache.index()].saturating_add(1);
            return Err(RevocationMonitorError::CacheNotSynchronized)
        }
        Ok(())
    }

    pub fn report(&self, measured_at_us: u64) -> PropagationReport {
        let mut completed_events = 0;
        let mut pending_events = 0;
        let mut observed_maximum_latency_us = 0;
        let mut pending_maximum_age_us = 0;
        for event in self.events.iter().flatten() {
            observed_maximum_latency_us = observed_maximum_latency_us.max(event.maximum_latency_us);
            if event.observed == ALL_CACHE_BITS {
                completed_events += 1;
            } else {
                pending_events += 1;
                pending_maximum_age_us = pending_maximum_age_us.max(
                    measured_at_us.saturating_sub(event.notice.revoked_at_us),
                );
            }
        }
        PropagationReport {
            measured_at_us,
            total_events: self.total_events,
            completed_events,
            pending_events,
            maximum_latency_us: observed_maximum_latency_us.max(pending_maximum_age_us),
            observed_maximum_latency_us,
            pending_maximum_age_us,
            late_observations: self.late_observations,
            stale_rejections: self.stale_rejections,
        }
    }

    pub fn cache_report(&self, cache: RevocationCache) -> CacheReport {
        let bit = cache.bit();
        let mut observed_events = 0;
        let mut pending_events = 0;
        let mut maximum_latency_us = 0;
        for event in self.events.iter().flatten() {
            if event.observed & bit != 0 {
                observed_events += 1;
                maximum_latency_us = maximum_latency_us
                    .max(event.maximum_latency_by_cache_us[cache.index()]);
            } else {
                pending_events += 1;
            }
        }
        CacheReport {
            cache,
            observed_events,
            pending_events,
            stale_rejections: self.cache_stale_rejections[cache.index()],
            maximum_latency_us,
        }
    }

    fn sync(
        &mut self,
        cache: RevocationCache,
        key: RevocationKey,
        cached_epoch: u64,
        now_us: u64,
    ) -> Result<(), RevocationMonitorError> {
        if cached_epoch == 0 {
            return Err(RevocationMonitorError::Invalid)
        }
        let Some(index) = self.latest_event_index(key) else {
            return Ok(())
        };
        let notice = self.events[index].expect("validated event index").notice;
        if cached_epoch < notice.epoch {
            self.stale_rejections = self.stale_rejections.saturating_add(1);
            self.cache_stale_rejections[cache.index()] =
                self.cache_stale_rejections[cache.index()].saturating_add(1);
            return Err(RevocationMonitorError::StaleEpoch)
        }
        if cached_epoch > notice.epoch {
            return Err(RevocationMonitorError::FutureEpoch)
        }
        self.observe(cache, notice, now_us).map(|_| ())
    }

    fn latest_epoch(&self, key: RevocationKey) -> Option<u64> {
        self.events
            .iter()
            .flatten()
            .filter(|event| event.notice.key == key)
            .map(|event| event.notice.epoch)
            .max()
    }

    fn latest_event_index(&self, key: RevocationKey) -> Option<usize> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| event.map(|event| (index, event)))
            .filter(|(_, event)| event.notice.key == key)
            .max_by_key(|(_, event)| event.notice.epoch)
            .map(|(index, _)| index)
    }

    fn event_index(&self, notice: RevocationNotice) -> Option<usize> {
        self.events.iter().position(|event| {
            event.is_some_and(|event| event.notice == notice)
        })
    }
}

impl<const CAPACITY: usize> Default for RevocationMonitor<CAPACITY> {
    fn default() -> Self {
        Self::new(MAX_REVOCATION_LATENCY_US)
    }
}
