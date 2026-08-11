use synos_durability::{
    DurabilityEvent, DurabilityTrace, DURABILITY_CONTRACT, DURABILITY_CONTRACT_VERSION,
};

fn main() {
    let transaction = 42;
    let mut trace = DurabilityTrace::<16>::new();
    for event in [
        DurabilityEvent::ApplicationWrite { transaction },
        DurabilityEvent::SynFsWrite { transaction },
        DurabilityEvent::Rename { transaction },
        DurabilityEvent::Commit { transaction },
        DurabilityEvent::StorageDaemonWrite { transaction },
        DurabilityEvent::BlockDataWrite { transaction },
        DurabilityEvent::CacheFlush { transaction },
        DurabilityEvent::BlockCommitRecord { transaction },
        DurabilityEvent::BlockFlush { transaction },
        DurabilityEvent::SyncAcknowledged { transaction },
        DurabilityEvent::PowerLoss,
        DurabilityEvent::Recovered { transaction },
    ] {
        trace.record(event).expect("trace has capacity");
    }
    trace.verify().expect("durability order is valid");
    println!(
        "durability contract v{}: {} layers",
        DURABILITY_CONTRACT_VERSION,
        DURABILITY_CONTRACT.len()
    );
}
