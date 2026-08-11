use super::*;

fn envelope(label: u64) -> Envelope {
    Envelope {
        correlation: 0x1234,
        label,
        buffer: Some(SharedBuffer {
            region: SharedRegionId::new(4).expect("valid region"),
            offset: 8,
            length: 16,
            writable: false,
        }),
        words: [1, 2, 3, 4],
    }
}

#[test]
fn bounded_ring_enforces_capacity_and_reuses_consumed_slots() {
    let ring = Ring::<2>::new();
    assert_eq!(ring.try_receive(), Err(RingError::Empty));
    assert_eq!(ring.pending(), 0);
    assert!(ring.try_send(envelope(1)).is_ok());
    assert!(ring.try_send(envelope(2)).is_ok());
    assert_eq!(ring.pending(), 2);
    assert_eq!(ring.try_send(envelope(3)), Err(RingError::Full));
    assert_eq!(ring.try_receive().unwrap().label, 1);
    assert!(ring.try_send(envelope(3)).is_ok());
    assert_eq!(ring.try_receive().unwrap().label, 2);
    assert_eq!(ring.try_receive().unwrap().label, 3);
    assert_eq!(ring.try_receive(), Err(RingError::Empty));
}

#[test]
fn structured_envelopes_validate_schema_version_and_checksum() {
    #[repr(align(8))]
    struct AlignedBytes([u8; 32]);

    let region = SharedRegionId::new(5).expect("valid region");
    let mut storage = AlignedBytes([0; 32]);
    let value = 0xfeed_beefu32;
    let mut arena = SharedArena::new(region, &mut storage.0);
    let envelope = arena
        .archive_structured(0x55, 7, &value, Some(9))
        .expect("archive value");
    drop(arena);
    let view = SharedView::new(region, &storage.0);
    assert_eq!(view.resolve_envelope::<u32>(envelope, 0x55), Ok(&value));
    assert_eq!(validate_structured(envelope, 0x56), Err(ArchiveError::SchemaMismatch));

    let mut corrupt = envelope;
    corrupt.words[1] ^= 1;
    assert_eq!(
        view.resolve_envelope::<u32>(corrupt, 0x55),
        Err(ArchiveError::InvalidDescriptor)
    );
}

#[test]
fn descriptor_inheritance_rejects_duplicates_and_capacity_overflow() {
    let channel = InheritableDescriptor::channel(3).expect("valid channel");
    assert!(validate_inheritable_descriptors(&[channel]).is_ok());
    assert_eq!(
        validate_inheritable_descriptors(&[channel, channel]),
        Err(DescriptorInheritanceError::Duplicate)
    );

    let descriptors = [channel; MAX_INHERITABLE_DESCRIPTORS + 1];
    assert_eq!(
        validate_inheritable_descriptors(&descriptors),
        Err(DescriptorInheritanceError::Capacity)
    );
}

#[test]
fn guarded_buffers_bind_capability_and_owner_to_the_wire_descriptor() {
    let region = SharedRegionId::new(6).expect("valid region");
    let descriptor = SharedBuffer {
        region,
        offset: 0,
        length: 4,
        writable: true,
    };
    let capability = BufferCapability::new(region, 3, BufferRights::ALL, 0x44)
        .expect("valid buffer capability");
    let read_only = BufferCapability::new(region, 3, BufferRights::READ, 0x45)
        .expect("valid read capability");
    let mut bytes = [1, 2, 3, 4];
    assert!(matches!(
        BufferLease::new(descriptor, read_only, BufferOwner::IpcProducer, &mut bytes),
        Err(BufferError::CapabilityDenied)
    ));
    let lease = BufferLease::new(descriptor, capability, BufferOwner::IpcProducer, &mut bytes)
        .expect("capability grants the initial owner");
    let lease = lease
        .transfer(BufferOwner::IpcConsumer)
        .expect("transfer is explicit");
    let envelope = guarded_envelope(0x99, 7, &lease).expect("read capability");
    assert_eq!(
        validate_guarded(envelope, 0x99, capability),
        Ok(descriptor)
    );
}
