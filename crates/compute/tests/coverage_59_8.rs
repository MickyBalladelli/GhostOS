use synos_compute::{
    Error, accelerator::{AcceleratorCapability, AcceleratorDevice, AcceleratorId, AcceleratorKind, AcceleratorOperation, AcceleratorQueue, AcceleratorRequest, ComputeApi, ComputeDispatch, DeviceLimits, KernelDescriptor, KernelFormat, KernelHandle},
    tensor::{DType, SharedTensor, TensorLayout, TensorRegion, TensorShape},
};
use synos_ipc::{SharedBuffer, SharedRegionId};
use synos_legacy_pc_drivers::{Bar, PciAddress, PciDevice};

fn buffer(region: u32, offset: u32, length: u32, writable: bool) -> SharedBuffer {
    SharedBuffer {
        region: SharedRegionId::new(region).unwrap(),
        offset,
        length,
        writable,
    }
}

fn device() -> AcceleratorDevice {
    AcceleratorDevice::from_pci(
        AcceleratorId::new(1).unwrap(),
        PciDevice {
            address: PciAddress::new(0, 2, 0).unwrap(),
            vendor_id: 1,
            device_id: 2,
            revision: 1,
            programming_interface: 0,
            subclass: 0,
            class: 0x03,
            header_type: 0,
            bars: [Bar::Memory64 { address: 0x1000, prefetchable: true }, Bar::Unused, Bar::Unused, Bar::Unused, Bar::Unused, Bar::Unused],
            interrupt_line: 0,
            interrupt_pin: 1,
        },
        ComputeApi::Native,
        DeviceLimits {
            max_workgroups: [4, 4, 4],
            max_bindings: 2,
            shared_memory_bytes: 1024,
            coherent_host_memory: true,
        },
    )
    .unwrap()
}

#[test]
fn tensor_layout_and_shared_mapping_validate_bounds() {
    let shape = TensorShape::new(&[2, 2]).unwrap();
    let layout = TensorLayout::contiguous(DType::F32, shape).unwrap();
    assert_eq!(shape.element_count().unwrap(), 4);
    assert_eq!(layout.required_bytes().unwrap(), 16);
    assert!(layout.is_contiguous());
    assert!(TensorShape::new(&[0]).is_err());
    assert!(TensorShape::new(&[u32::MAX, u32::MAX]).is_err());
    assert!(TensorLayout::strided(DType::F32, shape, &[2, 4]).is_err());

    let tensor = SharedTensor::new(buffer(1, 0, 16, true), layout).unwrap();
    let bytes = [7; 16];
    let region = TensorRegion::new(SharedRegionId::new(1).unwrap(), &bytes);
    let view = region
        .map(tensor)
        .unwrap();
    assert_eq!(view.bytes(), &bytes);
    assert!(matches!(
        TensorRegion::new(SharedRegionId::new(2).unwrap(), &bytes).map(tensor),
        Err(Error::RegionMismatch)
    ));
    assert!(SharedTensor::new(buffer(1, 4, 16, true), layout).is_err());
}

#[test]
fn accelerator_queue_checks_capability_and_completes_dispatch() {
    let device = device();
    let capability = AcceleratorCapability::new(9).unwrap();
    let tensor = SharedTensor::new(
        buffer(1, 0, 16, true),
        TensorLayout::contiguous(DType::F32, TensorShape::new(&[2, 2]).unwrap()).unwrap(),
    )
    .unwrap();
    let dispatch = ComputeDispatch::new(
        KernelHandle::new(1).unwrap(),
        [2, 1, 1],
        &[synos_compute::accelerator::DispatchBinding {
            slot: 0,
            tensor,
            access: synos_compute::framework::BindingAccess::ReadWrite,
        }],
    )
    .unwrap();
    let request = AcceleratorRequest {
        device: device.id,
        capability,
        operation: AcceleratorOperation::Dispatch(dispatch),
    };
    let mut queue = AcceleratorQueue::<1>::new(device, capability);
    let _token = queue.submit(request).unwrap();
    assert!(matches!(queue.submit(request), Err(Error::QueueFull)));
    let submission = queue.dispatch().unwrap();
    queue
        .complete(
            submission.token,
            synos_compute::accelerator::AcceleratorResult {
                status: 1,
                device_timestamp_ns: 77,
            },
        )
        .unwrap();
    assert_eq!(queue.poll().unwrap().result.device_timestamp_ns, 77);
}

#[test]
fn kernel_validation_rejects_writable_and_bad_spirv_buffers() {
    let good = KernelDescriptor {
        handle: KernelHandle::new(1).unwrap(),
        format: KernelFormat::SpirV,
        code: buffer(1, 0, 4, false),
    };
    assert!(good.validate().is_ok());
    assert!(KernelDescriptor { code: buffer(1, 0, 3, false), ..good }.validate().is_err());
    assert!(KernelDescriptor { code: buffer(1, 0, 4, true), ..good }.validate().is_err());
    assert_eq!(AcceleratorKind::Gpu, device().kind);
}
