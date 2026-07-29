use super::{Severity, Status, facility};

#[test]
fn status_round_trips_all_fields() {
    let status = Status::new(Severity::Information, facility::NETWORK, 0x123, 0xa)
        .expect("valid status");

    assert_eq!(status.severity(), Severity::Information);
    assert_eq!(status.facility(), facility::NETWORK);
    assert_eq!(status.code(), 0x123);
    assert_eq!(status.flags(), 0xa);
    assert_eq!(Status::from_raw(status.raw()), Some(status));
}

#[test]
fn status_rejects_out_of_range_fields() {
    assert_eq!(Status::new(Severity::Error, 0x1000, 1, 0), None);
    assert_eq!(Status::new(Severity::Error, 1, 0x2000, 0), None);
    assert_eq!(Status::new(Severity::Error, 1, 1, 0x10), None);
    assert_eq!(Status::from_raw(0b111), None);
}

#[test]
fn success_uses_openvms_low_bit_convention() {
    assert!(Status::NORMAL.is_success());
    assert!(!Status::INVALID_ARGUMENT.is_success());
    assert!(!Status::BUSY.is_success());
}
