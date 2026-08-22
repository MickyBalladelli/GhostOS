use std::time::Instant;

use ghostos_kernel::{
    HotAllocation, HotObjectAllocator, HotObjectKind,
};

const DEFAULT_ITERATIONS: usize = 20_000;

fn percentile(values: &mut [u128], percentile: usize) -> u128 {
    values.sort_unstable();
    let index = ((values.len().saturating_sub(1)) * percentile) / 100;
    values[index]
}

fn print_reports<const CPUS: usize, const NODES: usize>(
    allocator: &HotObjectAllocator<CPUS, NODES>,
    kinds: &[HotObjectKind],
    phase: &str,
) {
    for kind in kinds {
        let report = allocator.report(*kind);
        println!(
            "{{\"record\":\"allocator\",\"phase\":\"{phase}\",\"kind\":\"{kind:?}\",\"capacity\":{},\"in_use\":{},\"free\":{},\"fragmentation_per_mille\":{},\"local_cpu\":{},\"local_node\":{},\"remote_node\":{},\"max_probe_steps\":{},\"reclaims\":{},\"remote_reclaims\":{},\"invalid_reclaims\":{}}}",
            report.capacity,
            report.in_use,
            report.free,
            report.fragmentation_per_mille,
            report.stats.local_cpu_allocations,
            report.stats.local_node_allocations,
            report.stats.remote_node_allocations,
            report.stats.max_probe_steps,
            report.stats.reclaims,
            report.stats.remote_reclaims,
            report.stats.invalid_reclaims,
        );
    }
}

fn main() {
    let iterations = std::env::var("GHOSTOS_HOT_ALLOCATOR_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ITERATIONS);
    let mut allocator = HotObjectAllocator::<4, 2>::new([0, 0, 1, 1], 2, 64, 1)
        .expect("benchmark topology is valid");
    let kinds = [
        HotObjectKind::Ipc,
        HotObjectKind::Packet,
        HotObjectKind::Timer,
        HotObjectKind::Scheduler,
    ];
    let mut live: Vec<HotAllocation> = Vec::new();
    let mut latencies = Vec::with_capacity(iterations);
    let started = Instant::now();

    for index in 0..iterations {
        let cpu = index % 4;
        if index % 7 == 0 {
            if !live.is_empty() {
                let token = live.swap_remove(index % live.len());
                allocator
                    .reclaim_on(cpu, token)
                    .expect("live token reclaims once");
            }
        }
        let operation_started = Instant::now();
        if let Ok(token) = allocator.allocate(kinds[index % kinds.len()], cpu) {
            live.push(token);
        }
        latencies.push(operation_started.elapsed().as_nanos());
    }

    print_reports(&allocator, &kinds, "mixed");
    while let Some(token) = live.pop() {
        allocator
            .reclaim_on(live.len() % 4, token)
            .expect("final live token reclaims once");
    }

    let elapsed_ns = started.elapsed().as_nanos();
    let p50 = percentile(&mut latencies, 50);
    let p95 = percentile(&mut latencies, 95);
    let p99 = percentile(&mut latencies, 99);
    let operations = iterations as u128;
    let rate = operations as f64 / (elapsed_ns as f64 / 1_000_000_000.0);
    println!(
        "{{\"record\":\"metadata\",\"schema\":1,\"workload\":\"mixed-hot-objects\",\"iterations\":{iterations},\"cpus\":4,\"nodes\":2,\"fallback_nodes\":1}}"
    );
    println!(
        "{{\"record\":\"result\",\"benchmark\":\"hot-allocator-mixed\",\"unit\":\"operation\",\"work_units\":{operations},\"elapsed_ns\":{elapsed_ns},\"rate_per_second\":{rate},\"p50_ns\":{p50},\"p95_ns\":{p95},\"p99_ns\":{p99}}}"
    );
    print_reports(&allocator, &kinds, "reclaimed");
}
