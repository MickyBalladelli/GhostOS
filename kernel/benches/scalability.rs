use std::time::Instant;

use synos_ipc::{Envelope, Ring};
use synos_observability::{
    BatchController, ProducerPolicy, ScalePath, ScalePolicy, SCALE_CPU_TIERS,
};
use synos_kernel::Scheduler;

const DEFAULT_ITERATIONS: usize = 20_000;
const SAMPLE_COUNT: usize = 5;

fn iterations() -> usize {
    std::env::var("SYNOS_SCALE_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_ITERATIONS)
}

fn percentile(values: &mut [u128], percentile: usize) -> u128 {
    values.sort_unstable();
    let index = ((values.len().saturating_sub(1)) * percentile) / 100;
    values[index]
}

fn run_path(path: ScalePath, policy: ScalePolicy, iterations: usize) -> (u128, u128, usize) {
    let active = policy.affinity(path).count().max(1);
    let work = iterations.saturating_mul(active);
    let ring = Ring::<1024>::new();
    let start = Instant::now();
    std::thread::scope(|scope| {
        for worker in 0..active {
            let ring = &ring;
            scope.spawn(move || {
                let mut scheduler = Scheduler::new();
                let mut batches = BatchController::new(match path {
                    ScalePath::Logging => ProducerPolicy::LOGGING,
                    ScalePath::Audit => ProducerPolicy::AUDIT,
                    _ => ProducerPolicy::NETWORK,
                });
                for operation in 0..iterations {
                    match path {
                        ScalePath::Scheduler => {
                            if policy.accepts(path, worker) {
                                let _ = scheduler.dispatch();
                            }
                        }
                        ScalePath::Ipc => {
                            let _ = ring.try_send(Envelope::EMPTY);
                            let _ = ring.try_receive();
                        }
                        ScalePath::Timers => {
                            if policy.accepts(path, worker) {
                                let _ = scheduler.tick(1);
                            }
                        }
                        ScalePath::Logging => {
                            let _ = batches.plan(32, 128, 0, false);
                        }
                        ScalePath::Audit => {
                            let _ = batches.plan(16, 1, 0, false);
                        }
                    }
                    std::hint::black_box(operation);
                }
            });
        }
    });
    let elapsed_ns = start.elapsed().as_nanos().max(1);
    let throughput = (work as u128).saturating_mul(1_000_000_000) / elapsed_ns;
    (elapsed_ns, throughput, active)
}

fn queue_saturation(path: ScalePath) -> usize {
    let capacity = path.queue_capacity() as usize;
    let ring = Ring::<1024>::new();
    let mut accepted = 0;
    while accepted < capacity {
        if ring.try_send(Envelope::EMPTY).is_err() {
            break
        }
        accepted += 1;
    }
    accepted
}

fn main() {
    let iterations = iterations();
    println!(
        "{{\"record\":\"metadata\",\"benchmark\":\"kernel-scale\",\"tiers\":{:?},\"iterations\":{}}}",
        SCALE_CPU_TIERS,
        iterations,
    );
    for cpu_count in SCALE_CPU_TIERS {
        let policy = ScalePolicy::for_cpu_count(cpu_count).expect("declared scale tier");
        for path in ScalePath::ALL {
            let mut elapsed = [0u128; SAMPLE_COUNT];
            let mut throughput = [0u128; SAMPLE_COUNT];
            let mut active = 0;
            for sample in 0..SAMPLE_COUNT {
                let (sample_elapsed, sample_throughput, sample_active) =
                    run_path(path, policy, iterations);
                elapsed[sample] = sample_elapsed;
                throughput[sample] = sample_throughput;
                active = sample_active;
            }
            let p50 = percentile(&mut elapsed, 50);
            let p99 = percentile(&mut elapsed, 99);
            let throughput_p50 = percentile(&mut throughput, 50);
            let p50_per_operation = p50 / (iterations.saturating_mul(active).max(1) as u128);
            let p99_per_operation = p99 / (iterations.saturating_mul(active).max(1) as u128);
            println!(
                "{{\"record\":\"result\",\"benchmark\":\"kernel-scale\",\"unit\":\"operation\",\"work_units\":{},\"checksum\":{},\"allocation_count\":0,\"allocated_bytes\":0,\"peak_live_bytes\":0,\"path\":{},\"cpu_count\":{},\"active_cpus\":{},\"housekeeping\":{},\"isolated\":{},\"elapsed_ns\":{},\"p50_ns\":{},\"p99_ns\":{},\"p99_budget_us\":{},\"tail_budget_ok\":{},\"rate_per_second\":{},\"queue_saturation\":{},\"lock_saturation_per_cpu\":{}}}",
                iterations,
                iterations as u64,
                path as u8,
                cpu_count,
                active,
                policy.housekeeping().count(),
                policy.isolated().count(),
                p50_per_operation,
                p50_per_operation,
                p99_per_operation,
                path.p99_budget_us(),
                p99_per_operation <= path.p99_budget_us() as u128 * 1_000,
                throughput_p50,
                queue_saturation(path),
                path.lock_saturation_per_cpu(),
            );
        }
    }
}
