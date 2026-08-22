//! Deterministic property-test helpers.
//!
//! The runner deliberately does not use wall-clock time or process randomness.
//! A failed case prints the exact seed and case number needed to replay it.

use super::{DEFAULT_SEED, DeterministicEntropy, stable_hash};
use std::collections::VecDeque;
use std::env;
use std::fmt;

pub const DEFAULT_CASES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Config {
    pub seed: u64,
    pub cases: usize,
    pub replay_case: Option<usize>,
}

impl Config {
    pub const fn new(seed: u64, cases: usize) -> Self {
        Self {
            seed,
            cases,
            replay_case: None,
        }
    }

    pub const fn replay(seed: u64, case: usize) -> Self {
        Self {
            seed,
            cases: 1,
            replay_case: Some(case),
        }
    }

    /// Read optional replay controls. Unset variables use stable defaults.
    pub fn from_env() -> Self {
        let seed = env::var("GHOSTOS_PROPERTY_SEED")
            .ok()
            .and_then(|value| parse_integer(&value))
            .unwrap_or(DEFAULT_SEED);
        let cases = env::var("GHOSTOS_PROPERTY_CASES")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CASES);
        let replay_case = env::var("GHOSTOS_PROPERTY_CASE")
            .ok()
            .and_then(|value| value.parse().ok());
        Self {
            seed,
            cases,
            replay_case,
        }
    }

    pub fn case_seed(self, case: usize) -> u64 {
        let mut input = self.seed.to_le_bytes().to_vec();
        input.extend_from_slice(&(case as u64).to_le_bytes());
        stable_hash(&input)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::new(DEFAULT_SEED, DEFAULT_CASES)
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct Failure {
    pub property: String,
    pub seed: u64,
    pub case: usize,
    pub case_seed: u64,
    pub message: String,
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "property {} failed at case {} (seed 0x{:016x}, case seed 0x{:016x}): {}\nreplay with GHOSTOS_PROPERTY_SEED=0x{:016x} GHOSTOS_PROPERTY_CASE={}",
            self.property,
            self.case,
            self.seed,
            self.case_seed,
            self.message,
            self.seed,
            self.case
        )
    }
}

impl std::error::Error for Failure {}

/// Run a deterministic property for every configured case.
pub fn run<F>(property: &str, config: Config, mut check: F) -> Result<(), Failure>
where
    F: FnMut(usize, u64, &mut DeterministicEntropy) -> Result<(), String>,
{
    let cases = config
        .replay_case
        .map(|case| case..case + 1)
        .unwrap_or(0..config.cases);
    for case in cases {
        let case_seed = config.case_seed(case);
        let mut entropy = DeterministicEntropy::new(case_seed);
        if let Err(message) = check(case, case_seed, &mut entropy) {
            return Err(Failure {
                property: property.into(),
                seed: config.seed,
                case,
                case_seed,
                message,
            });
        }
    }
    Ok(())
}

/// Convenience form for `no_std` test modules that only need a pass/fail
/// predicate and should not allocate an error message.
pub fn run_assert<F>(property: &str, config: Config, mut check: F) -> Result<(), Failure>
where
    F: FnMut(usize, u64, &mut DeterministicEntropy) -> bool,
{
    run(property, config, |case, seed, entropy| {
        if check(case, seed, entropy) {
            Ok(())
        } else {
            Err("property assertion failed".into())
        }
    })
}

pub fn bytes(entropy: &mut DeterministicEntropy, maximum_length: usize) -> Vec<u8> {
    let length = bounded_usize(entropy, maximum_length.saturating_add(1));
    let mut result = vec![0; length];
    entropy.fill_bytes(&mut result);
    result
}

pub fn ascii(entropy: &mut DeterministicEntropy, maximum_length: usize) -> String {
    bytes(entropy, maximum_length)
        .into_iter()
        .map(|byte| b'a' + byte % 26)
        .map(char::from)
        .collect()
}

pub fn bounded_usize(entropy: &mut DeterministicEntropy, exclusive_upper_bound: usize) -> usize {
    if exclusive_upper_bound == 0 {
        0
    } else {
        (entropy.next_u64() % exclusive_upper_bound as u64) as usize
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedQueue<T> {
    capacity: usize,
    values: VecDeque<T>,
}

impl<T> BoundedQueue<T> {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            values: VecDeque::new(),
        }
    }

    pub fn push(&mut self, value: T) -> Result<(), T> {
        if self.values.len() == self.capacity {
            Err(value)
        } else {
            self.values.push_back(value);
            Ok(())
        }
    }

    pub fn pop(&mut self) -> Option<T> {
        self.values.pop_front()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn is_full(&self) -> bool {
        self.values.len() == self.capacity
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityModel {
    rights: std::collections::BTreeMap<u64, u32>,
    generations: std::collections::BTreeMap<u64, u32>,
}

impl CapabilityModel {
    pub fn new() -> Self {
        Self {
            rights: std::collections::BTreeMap::new(),
            generations: std::collections::BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, slot: u64, rights: u32) -> u32 {
        let generation = self.generations.entry(slot).or_insert(1);
        self.rights.insert(slot, rights);
        *generation
    }

    pub fn attenuate(&mut self, slot: u64, rights: u32) -> bool {
        let Some(current) = self.rights.get_mut(&slot) else {
            return false
        };
        if rights & !*current != 0 {
            return false
        }
        *current = rights;
        true
    }

    pub fn revoke(&mut self, slot: u64) -> bool {
        let removed = self.rights.remove(&slot).is_some();
        if removed {
            let generation = self.generations.entry(slot).or_insert(1);
            *generation = generation.wrapping_add(1).max(1);
        }
        removed
    }

    pub fn allows(&self, slot: u64, rights: u32) -> bool {
        self.rights
            .get(&slot)
            .is_some_and(|current| rights & !*current == 0)
    }

    pub fn generation(&self, slot: u64) -> u32 {
        self.generations.get(&slot).copied().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseModel {
    expiry: Option<u64>,
    revoked: bool,
}

impl LeaseModel {
    pub const fn new(expiry: u64) -> Self {
        Self {
            expiry: Some(expiry),
            revoked: false,
        }
    }

    pub const fn renew(&mut self, expiry: u64) {
        if !self.revoked {
            self.expiry = Some(expiry);
        }
    }

    pub const fn revoke(&mut self) {
        self.revoked = true;
        self.expiry = None;
    }

    pub const fn active_at(self, now: u64) -> bool {
        !self.revoked && matches!(self.expiry, Some(expiry) if now < expiry)
    }
}

fn parse_integer(value: &str) -> Option<u64> {
    value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .map_or_else(|| value.parse().ok(), |hex| u64::from_str_radix(hex, 16).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_runs_the_same_case_seed() {
        let config = Config::new(7, 4);
        let mut first = None;
        run("seed", config, |case, seed, _| {
            if case == 2 {
                first = Some(seed);
            }
            Ok(())
        })
        .expect("property succeeds");

        let mut replay = None;
        run("seed", Config::replay(7, 2), |_, seed, _| {
            replay = Some(seed);
            Ok(())
        })
        .expect("replay succeeds");
        assert_eq!(first, replay);
    }

    #[test]
    fn failure_contains_replay_command() {
        let failure = run("failure", Config::new(9, 1), |_, _, _| {
            Err("bad input".into())
        })
        .expect_err("property must fail");
        let message = failure.to_string();
        assert!(message.contains("GHOSTOS_PROPERTY_SEED=0x0000000000000009"));
        assert!(message.contains("GHOSTOS_PROPERTY_CASE=0"));
    }

    #[test]
    fn bounded_queue_matches_fifo_contract() {
        let mut queue = BoundedQueue::new(2);
        assert!(queue.push(1).is_ok());
        assert!(queue.push(2).is_ok());
        assert_eq!(queue.push(3), Err(3));
        assert_eq!(queue.pop(), Some(1));
        assert_eq!(queue.pop(), Some(2));
        assert!(queue.is_empty());
    }

    #[test]
    fn capability_model_only_allows_attenuation() {
        let mut capabilities = CapabilityModel::new();
        assert_eq!(capabilities.insert(4, 0b111), 1);
        assert!(capabilities.attenuate(4, 0b011));
        assert!(!capabilities.attenuate(4, 0b1000));
        assert!(capabilities.allows(4, 0b001));
        assert!(!capabilities.allows(4, 0b100));
        assert!(capabilities.revoke(4));
        assert_eq!(capabilities.generation(4), 2);
        assert!(!capabilities.allows(4, 1));
    }

    #[test]
    fn lease_model_expires_and_revokes() {
        let mut lease = LeaseModel::new(10);
        assert!(lease.active_at(9));
        assert!(!lease.active_at(10));
        lease.renew(20);
        assert!(lease.active_at(19));
        lease.revoke();
        assert!(!lease.active_at(0));
    }
}
