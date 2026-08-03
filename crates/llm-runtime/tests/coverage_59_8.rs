use synos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use synos_fabric::memory::{GlobalAddressSpace, LeaseTable, MemoryKind, MemoryPool, PoolId, Transport};
use synos_llm::{Error, RequestId};
use synos_llm::allocator::{AllocationPolicy, UnifiedAllocator};
use synos_llm::inference::{InferenceLedger, InferenceState, RecoveryRecord};
use synos_llm::kv_cache::KvCachePool;

fn space() -> GlobalAddressSpace<2, 2> {
    let mut space = GlobalAddressSpace::new();
    space
        .add_pool(MemoryPool {
            id: PoolId::new(1).unwrap(),
            node: NodeId::LOCAL,
            mirror: NodeId::new(2),
            kind: MemoryKind::Ram,
            transport: Transport::Local,
            global: AddressRange::new(0x1000_0000, 0x20_000).unwrap(),
            backing_start: 0x2000_0000,
            latency_ns: 100,
        })
        .unwrap();
    space
}

#[test]
fn allocator_aligns_ranges_and_releases_leases() {
    let space = space();
    let mut leases = LeaseTable::<8>::new();
    let mut allocator = UnifiedAllocator::<2, 4>::new(AddressRange::new(0x1_0000_0000, 0x20_000).unwrap());
    let allocation = allocator
        .allocate(
            &space,
            &mut leases,
            5_000,
            PAGE_SIZE,
            AllocationPolicy::single_node(NodeId::LOCAL, MemoryKind::Ram),
            10,
            100,
        )
        .unwrap();
    assert_eq!(allocation.virtual_range.length, PAGE_SIZE * 2);
    let resolved = allocator
        .resolve(allocation.handle, 0, synos_fabric::Access::Read, 20, &space, &leases)
        .unwrap();
    assert_eq!(resolved.resolved.transport, Transport::Local);
    assert!(matches!(allocator.resolve(allocation.handle, allocation.virtual_range.length, synos_fabric::Access::Read, 20, &space, &leases), Err(Error::InvalidRange)));
    allocator.release(allocation.handle, &mut leases).unwrap();
    assert!(matches!(allocator.info(allocation.handle), Err(Error::AllocationNotFound)));
}

#[test]
fn kv_cache_grows_commits_tokens_and_rejects_bad_offsets() {
    let space = space();
    let mut leases = LeaseTable::<16>::new();
    let mut allocator = UnifiedAllocator::<4, 8>::new(AddressRange::new(0x1_0000_0000, 0x100_000).unwrap());
    let mut caches = KvCachePool::<2, 4>::new();
    let cache = caches
        .open_in_memory(
            RequestId::new(3).unwrap(),
            NodeId::LOCAL,
            MemoryKind::Ram,
            4096,
            2,
            &mut allocator,
            &space,
            &mut leases,
            0,
            1_000,
        )
        .unwrap();
    assert_eq!(cache.reserved_tokens, 2);
    caches.commit_tokens(cache.handle, 1).unwrap();
    assert_eq!(caches.info(cache.handle).unwrap().committed_tokens, 1);
    assert!(caches.resolve(cache.handle, 2, 0, synos_fabric::Access::Read, 1, &allocator, &space, &leases).is_err());
}

#[test]
fn inference_checkpoint_wire_and_failover_are_deterministic() {
    let request = RequestId::new(9).unwrap();
    let model = synos_llm::allocator::AllocationHandle::from_raw((1u64 << 32) | 1).unwrap();
    let cache = synos_llm::kv_cache::KvCacheHandle::from_raw((1u64 << 32) | 2).unwrap();
    let mut ledger = InferenceLedger::<2>::new();
    let (handle, initial) = ledger.begin(request, model, cache, NodeId::new(2).unwrap(), NodeId::new(3).unwrap(), 44).unwrap();
    assert_eq!(RecoveryRecord::decode(initial.bytes).unwrap().rng_state, 44);
    assert!(matches!(ledger.acknowledge_checkpoint(handle, initial.epoch, true, false), Err(Error::NoFailoverReplica)));
    ledger.acknowledge_checkpoint(handle, initial.epoch, true, true).unwrap();
    let checkpoint = ledger.prepare_checkpoint(handle, 5, 55).unwrap();
    let info = ledger.acknowledge_checkpoint(handle, checkpoint.epoch, true, true).unwrap();
    assert_eq!(info.state, InferenceState::Running);
    let degradation = ledger.fail_node(handle, NodeId::new(2).unwrap()).unwrap();
    assert_eq!(degradation.journal_node, NodeId::new(3).unwrap());
    let repair = ledger.prepare_replica_repair(handle, NodeId::new(4).unwrap()).unwrap();
    assert_eq!(repair.primary, NodeId::new(3).unwrap());
    assert_eq!(repair.replica, NodeId::new(4).unwrap());
    let mut corrupt = checkpoint.bytes;
    corrupt[64] ^= 1;
    assert!(matches!(RecoveryRecord::decode(corrupt), Err(Error::CorruptRecoveryRecord)));
}
