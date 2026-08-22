//! Negative-input matrix for guest-facing boundaries.

use ghostos_boot_protocol::{BootInfo, BootMethod, MAX_MEMORY_REGIONS};
use ghostos_ipc::{SharedBuffer, SharedRegionId};
use ghostos_runtime::{Error as RuntimeError, Operation, Response, Runtime, SystemCall};
use ghostos_status::Status;
use ghostos_ghostfs::{Error as SynFsError, SynFs, VersionedPath};

struct InvalidAbi;

impl SystemCall for InvalidAbi {
    fn call(&self, _request: ghostos_runtime::Request) -> Response {
        Response {
            status: u32::MAX,
            flags: 0,
            values: [0; 4],
        }
    }
}

#[test]
fn malformed_input_matrix_returns_stable_errors() {
    // Boot info.
    let mut boot_info = BootInfo::empty(BootMethod::Bios);
    boot_info.magic = 0;
    assert_eq!(
        ghostos_kernel::validate_boot_info(&boot_info),
        Err(Status::INVALID_ARGUMENT)
    );
    boot_info.magic = ghostos_boot_protocol::BOOT_INFO_MAGIC;
    boot_info.memory_region_count = MAX_MEMORY_REGIONS + 1;
    assert_eq!(
        ghostos_kernel::validate_boot_info(&boot_info),
        Err(Status::INVALID_ARGUMENT)
    );

    // ABI frames.
    assert_eq!(Operation::from_raw(u16::MAX), None);
    assert_eq!(
        Runtime::new(InvalidAbi).clock_now(),
        Err(RuntimeError::InvalidResponse)
    );

    // Shared buffers.
    let region = SharedRegionId::new(7).expect("non-zero test region");
    assert!(SharedRegionId::new(0).is_none());
    let mut bytes = [0u8; 8];
    let mut mapped = ghostos_netd::MappedRegion::new(region, &mut bytes);
    let out_of_range = SharedBuffer {
        region,
        offset: 7,
        length: 2,
        writable: false,
    };
    assert_eq!(
        ghostos_netd::SharedMemory::read(&mapped, out_of_range, |_| ()),
        Err(ghostos_netd::MemoryError::InvalidRange)
    );
    let read_only = SharedBuffer {
        region,
        offset: 0,
        length: 1,
        writable: false,
    };
    assert_eq!(
        ghostos_netd::SharedMemory::write(&mut mapped, read_only, |_| ()),
        Err(ghostos_netd::MemoryError::ReadOnly)
    );

    // Capabilities.
    assert!(ghostos_runtime::Capability::from_raw(0).is_none());
    assert!(ghostos_kernel::CapabilityHandle::from_raw(0).is_none());
    assert!(ghostos_kernel::Rights::from_bits(u16::MAX).is_none());

    // Application and package manifests.
    assert_eq!(
        ghostos_app::AppManifest::parse("schema = 2"),
        Err(ghostos_app::ManifestError::MissingField)
    );
    assert!(matches!(
        ghostos_pkg::PackageBundle::decode(&[]),
        Err(ghostos_pkg::PackageError::BundleTooSmall)
    ));

    // Filesystem metadata.
    assert_eq!(
        VersionedPath::parse("/tmp/../file"),
        Err(SynFsError::InvalidPath)
    );
    let corrupt_volume = vec![0u8; SynFs::<16>::volume_bytes()];
    assert!(matches!(
        SynFs::<16>::load(&corrupt_volume),
        Err(SynFsError::Corrupt)
    ));

    // Packets.
    assert_eq!(
        ghostos_netd::PacketView::parse(&[0u8; 20]),
        Err(ghostos_netd::FirewallError::InvalidFrame)
    );

    // Terminal input.
    let mut terminal = ghostos_webterm::Terminal::<80, 25>::new().expect("valid test terminal");
    terminal.write(&[
        0xff, 0xfe, 0x1b, b'[', b'9', b'9', b'9', b'm', 0x1b, b'[', b'2', b'J',
    ]);
    let cursor = terminal.cursor();
    assert!(usize::from(cursor.column) < terminal.columns());
    assert!(usize::from(cursor.row) < terminal.rows());

    // Snapshot data.
    assert!(matches!(
        ghostos_vm::VmSnapshot::from_bytes(&[]),
        Err(ghostos_vm::SnapshotError::InvalidFormat)
    ));
}

#[test]
fn locked_disk_error_is_readable() {
    let error = ghostos_vm::devices::StorageError::Locked {
        path: "/tmp/data.raw.ghostos.lock".to_string(),
        owner: "version=2\nimage_identity=/tmp/data.raw\nowner_identity=micky\npid=25765\nstart_time=Sun Aug 9 19:11:20 2026\nhost_identity=hostname=unknown;machine=unknown\nformat=raw\nlock_token=25765-0".to_string(),
    };

    assert_eq!(
        error.to_string(),
        "disk is already locked\n  lock: /tmp/data.raw.ghostos.lock\n  image: /tmp/data.raw\n  owner: micky\n  pid: 25765\n  started: Sun Aug 9 19:11:20 2026\n  host: hostname=unknown;machine=unknown\n  format: raw\n\n  action: ./target/release/ghostos-vm disk lock /tmp/data.raw"
    );
}
