use core::cell::Cell;

use ghostos_ipc::{SharedBuffer, SharedRegionId};
use ghostos_status::Status;

use super::*;

struct MockSystemCall {
    request: Cell<Request>,
    response: Response,
}

impl SystemCall for MockSystemCall {
    fn call(&self, request: Request) -> Response {
        self.request.set(request);
        self.response
    }
}

fn buffer(writable: bool) -> SharedBuffer {
    SharedBuffer {
        region: SharedRegionId::new(7).expect("valid region"),
        offset: 32,
        length: 128,
        writable,
    }
}

#[test]
fn abi_operations_and_capabilities_reject_unknown_or_stale_values() {
    assert_eq!(Operation::from_raw(Operation::SynFsDelete as u16), Some(Operation::SynFsDelete));
    assert_eq!(Operation::from_raw(Operation::SynFsRmdir as u16), Some(Operation::SynFsRmdir));
    assert_eq!(Operation::from_raw(Operation::SynFsLink as u16), Some(Operation::SynFsLink));
    assert_eq!(Operation::from_raw(Operation::SynFsLinks as u16), Some(Operation::SynFsLinks));
    assert_eq!(Operation::from_raw(0), None);
    assert_eq!(Operation::from_raw(u16::MAX), None);
    assert_eq!(Capability::from_raw(0), None);
    assert_eq!(Capability::from_raw(1_u64 << 32).unwrap().raw(), 1_u64 << 32);
}

#[test]
fn filesystem_operation_numbers_are_stable_and_contiguous() {
    let operations = [
        Operation::SynFsOpen,
        Operation::SynFsClose,
        Operation::SynFsRead,
        Operation::SynFsWrite,
        Operation::SynFsMetadata,
        Operation::SynFsMkdir,
        Operation::SynFsRmdir,
        Operation::SynFsLink,
        Operation::SynFsList,
        Operation::SynFsLinks,
        Operation::SynFsDelete,
    ];
    for pair in operations.windows(2) {
        assert_eq!(pair[1] as u16, pair[0] as u16 + 1);
    }
}

#[test]
fn request_encoding_preserves_capability_buffer_and_direction() {
    let capability = Capability::from_raw((9_u64 << 32) | 3).expect("valid capability");
    let request = Request::new(Operation::SynFsWrite)
        .with_capability(capability)
        .with_buffer(buffer(true));

    assert_eq!(request.operation, Operation::SynFsWrite as u16);
    assert_eq!(request.capability, capability.raw());
    assert_eq!(request.arguments, [7, 32, 128, 1, 0, 0]);
}

#[test]
fn runtime_maps_statuses_and_rejects_malformed_responses() {
    let system = MockSystemCall {
        request: Cell::new(Request::new(Operation::Yield)),
        response: Response {
            status: Status::ACCESS_DENIED.raw(),
            flags: 0,
            values: [0; 4],
        },
    };
    let runtime = Runtime::new(system);
    assert_eq!(runtime.clock_now(), Err(Error::Status(Status::ACCESS_DENIED)));

    let system = MockSystemCall {
        request: Cell::new(Request::new(Operation::Yield)),
        response: Response {
            status: 7,
            flags: 0,
            values: [0; 4],
        },
    };
    let runtime = Runtime::new(system);
    assert_eq!(runtime.clock_now(), Err(Error::InvalidResponse));
}

#[test]
fn runtime_marshals_mapping_and_wait_arguments() {
    let capability = Capability::from_raw((4_u64 << 32) | 1).expect("valid capability");
    let system = MockSystemCall {
        request: Cell::new(Request::new(Operation::Yield)),
        response: Response {
            status: Status::NORMAL.raw(),
            flags: 0,
            values: [0x8000, 0, 0, 0],
        },
    };
    let runtime = Runtime::new(system);
    assert_eq!(runtime.map(capability, 0x1000, 0x2000, true), Ok(0x8000));
    let request = runtime.system().request.get();
    assert_eq!(request.operation, Operation::MemoryMap as u16);
    assert_eq!(request.arguments, [0x1000, 0x2000, 1, 0, 0, 0]);
}
