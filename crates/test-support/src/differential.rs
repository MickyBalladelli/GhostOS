//! Small, deterministic helpers for differential compatibility tests.

use std::fmt::Debug;

/// Minimize an input while preserving a predicate that reproduces a
/// divergence. The reducer is deterministic and uses chunk deletion followed
/// by single-element deletion.
pub fn minimize<T: Clone>(input: &[T], diverges: impl Fn(&[T]) -> bool) -> Vec<T> {
    let mut reduced = input.to_vec();
    let mut chunk_count = 2usize;

    while reduced.len() >= 2 {
        let chunk_size = reduced.len().div_ceil(chunk_count);
        let mut changed = false;
        let mut start = 0;
        while start < reduced.len() {
            let end = (start + chunk_size).min(reduced.len());
            let mut candidate = reduced.clone();
            candidate.drain(start..end);
            if diverges(&candidate) {
                reduced = candidate;
                chunk_count = chunk_count.saturating_sub(1).max(2);
                changed = true;
                break;
            }
            start = end;
        }
        if !changed {
            if chunk_count >= reduced.len() {
                break;
            }
            chunk_count = (chunk_count * 2).min(reduced.len());
        }
    }

    let mut index = 0;
    while index < reduced.len() {
        let mut candidate = reduced.clone();
        candidate.remove(index);
        if diverges(&candidate) {
            reduced = candidate;
        } else {
            index += 1;
        }
    }
    reduced
}

/// Fail a differential test with the smallest known reproducer.
pub fn assert_no_divergence<T: Clone + Debug>(
    subsystem: &str,
    input: &[T],
    decision: &str,
    diverges: impl Fn(&[T]) -> Option<String>,
) {
    let Some(details) = diverges(input) else {
        return;
    };
    let minimized = minimize(input, |candidate| diverges(candidate).is_some());
    let minimized_details = diverges(&minimized).unwrap_or(details);
    panic!(
        "{subsystem} differential divergence: {minimized_details}; minimized input: {minimized:?}; decision: {decision}"
    )
}
