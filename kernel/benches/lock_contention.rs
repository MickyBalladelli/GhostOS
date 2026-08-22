use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use ghostos_kernel::{CapabilityQuota, QuotaContentionReport, QuotaResource};

const BATCHES: usize = 32;
const DEFAULT_ITERATIONS: usize = 20_000;

fn percentile(values: &mut [u128], percentile: usize) -> u128 {
    values.sort_unstable();
    let index = ((values.len().saturating_sub(1)) * percentile) / 100;
    values[index]
}

fn run(workers: usize, iterations: usize) {
    let quota = Arc::new(CapabilityQuota::new());
    let barrier = Arc::new(Barrier::new(workers));
    let started = Instant::now();
    let mut handles = Vec::with_capacity(workers);
    for worker in 0..workers {
        let quota = Arc::clone(&quota);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            let resource = match worker % 3 {
                0 => QuotaResource::IpcMessages,
                1 => QuotaResource::PageFaults,
                _ => QuotaResource::MemoryBytes,
            };
            let batch_size = (iterations / BATCHES).max(1);
            let mut batches = Vec::with_capacity(BATCHES);
            for batch in 0..BATCHES {
                let batch_started = Instant::now();
                for index in 0..batch_size {
                    let now = (batch * batch_size + index) as u64;
                    let decision = quota.consume(resource, now, 1);
                    assert_eq!(decision, ghostos_kernel::QuotaDecision::Allowed);
                    quota.refund(resource, 1);
                }
                batches.push(batch_started.elapsed().as_nanos() / batch_size as u128);
            }
            batches
        }));
    }
    let mut latencies = Vec::with_capacity(workers * BATCHES);
    for handle in handles {
        latencies.extend(handle.join().expect("worker completed"));
    }
    let elapsed_ns = started.elapsed().as_nanos();
    let p50 = percentile(&mut latencies, 50);
    let p95 = percentile(&mut latencies, 95);
    let p99 = percentile(&mut latencies, 99);
    let operations = workers.saturating_mul(iterations);
    let rate = operations as f64 / (elapsed_ns as f64 / 1_000_000_000.0);
    let report: QuotaContentionReport = quota.contention_report();
    let contended: u64 = report
        .resources
        .iter()
        .map(|resource| resource.contended_acquisitions)
        .sum();
    let spins: u64 = report.resources.iter().map(|resource| resource.spins).sum();
    let active_owners: u64 = report
        .resources
        .iter()
        .filter(|resource| resource.active_owner != 0)
        .count() as u64;
    println!(
        "{{\"record\":\"metadata\",\"schema\":1,\"workload\":\"quota-locks\",\"workers\":{workers},\"iterations\":{iterations},\"cpus\":{}}}",
        thread::available_parallelism().map_or(1, |value| value.get())
    );
    println!(
        "{{\"record\":\"result\",\"benchmark\":\"quota-locks-{workers}core\",\"unit\":\"operation\",\"work_units\":{operations},\"elapsed_ns\":{elapsed_ns},\"rate_per_second\":{rate},\"allocation_count\":0,\"allocated_bytes\":0,\"peak_live_bytes\":0,\"checksum\":{operations},\"p50_ns\":{p50},\"p95_ns\":{p95},\"p99_ns\":{p99},\"contended_acquisitions\":{contended},\"spins\":{spins},\"active_owners\":{active_owners}}}"
    );
}

fn main() {
    let iterations = std::env::var("GHOSTOS_LOCK_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS);
    let cpus = thread::available_parallelism().map_or(1, |value| value.get());
    let many_workers = std::env::var("GHOSTOS_LOCK_WORKERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(cpus.max(2));
    run(1, iterations);
    run(many_workers, iterations);
}
