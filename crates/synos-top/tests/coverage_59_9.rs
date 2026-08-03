use synos_fabric::NodeId;
use synos_top::{
    CapabilityKind, CapabilitySample, DashboardRenderer, NodeHealth, NodeSample, Poll,
    RealtimeMonitor, TopologySnapshot, TopologySource, Viewport,
};

#[test]
fn dashboard_rendering_has_a_stable_terminal_snapshot() {
    let mut snapshot = TopologySnapshot::<2, 2>::new();
    snapshot.push_node(NodeSample {
        node: NodeId::LOCAL,
        health: NodeHealth::Healthy,
        ram_used_bytes: 512 * 1024 * 1024,
        ram_total_bytes: 1024 * 1024 * 1024,
        vram_used_bytes: 256 * 1024 * 1024,
        vram_total_bytes: 512 * 1024 * 1024,
        remote_faults: 3,
        dsm_latency_p50_ns: 100,
        dsm_latency_p99_ns: 200,
    }).unwrap();
    snapshot.push_capability(CapabilitySample { handle: 1, parent: None, owner: 7, rights: 3, kind: CapabilityKind::Memory }).unwrap();
    snapshot.finish(99).unwrap();

    let mut renderer = DashboardRenderer::new(Viewport::new(64, 20).unwrap());
    let mut output = String::new();
    renderer.render(&snapshot, &mut output).unwrap();
    assert!(output.starts_with("\x1b[?25l\x1b[2J\x1b[H\x1b[1;96mSYNOS-TOP\x1b[0m  generation=1  sample=99us"));
    assert!(output.contains("CLUSTER RAM / VRAM HEATMAP"));
    assert!(output.contains("0000000000000001"));
    assert!(output.ends_with("\x1b[0m\x1b[J"));
}

struct Source;

impl TopologySource<1, 0> for Source {
    type Error = ();

    fn sample(&mut self, now_us: u64, snapshot: &mut TopologySnapshot<1, 0>) -> Result<(), Self::Error> {
        snapshot.push_node(NodeSample {
            node: NodeId::LOCAL,
            health: NodeHealth::Degraded,
            ram_used_bytes: 1,
            ram_total_bytes: 2,
            vram_used_bytes: 0,
            vram_total_bytes: 1,
            remote_faults: now_us,
            dsm_latency_p50_ns: 1,
            dsm_latency_p99_ns: 2,
        }).unwrap();
        Ok(())
    }
}

#[test]
fn monitor_polls_at_a_fixed_rate_and_rejects_zero_interval() {
    assert!(RealtimeMonitor::<Source, 1, 0>::new(Source, 0, 0).is_none());
    let mut monitor = RealtimeMonitor::<Source, 1, 0>::new(Source, 10, 5).unwrap();
    assert_eq!(monitor.poll(4), Poll::Waiting);
    assert_eq!(monitor.poll(5), Poll::Updated { generation: 1 });
    assert_eq!(monitor.poll(9), Poll::Waiting);
    assert_eq!(monitor.poll(15), Poll::Updated { generation: 2 });
    assert_eq!(monitor.snapshot().sampled_at_us(), 15);
}
