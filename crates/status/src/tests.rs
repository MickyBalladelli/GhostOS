use super::{AuditContext, IntoPublicError, IntoStatus, PublicError, RetryHint, Severity, Status, facility};

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

#[test]
fn public_error_keeps_only_stable_safe_context() {
    let error = PublicError::new(
        Status::ACCESS_DENIED,
        7,
        RetryHint::Never,
        AuditContext::new(0xfeed, 3),
    );

    assert_eq!(error.code.raw(), Status::ACCESS_DENIED.raw());
    assert_eq!(error.operation, 7);
    assert!(!error.retry.is_retryable());
    assert_eq!(error.audit, AuditContext::new(0xfeed, 3));
}

#[test]
fn every_status_error_can_be_wrapped_at_a_boundary() {
    struct ExampleError;

    impl IntoStatus for ExampleError {
        fn status(self) -> Status {
            Status::BUSY
        }
    }

    let error = ExampleError.public_error(12, AuditContext::new(8, 1));
    assert_eq!(error.code, Status::BUSY);
    assert_eq!(error.operation, 12);
    assert_eq!(error.retry, RetryHint::AfterUs(1_000_000));
    assert_eq!(error.audit.correlation, 8);
}

#[test]
fn stable_messages_cover_public_status_constants() {
    assert_eq!(Status::NORMAL.message(), "normal");
    assert_eq!(Status::ACCESS_DENIED.message(), "access denied");
    assert_eq!(Status::METHOD_NOT_ALLOWED.message(), "method not allowed");
    assert_eq!(Status::DIRECTORY_NOT_EMPTY.message(), "directory not empty");
    assert_eq!(Status::INVALID_PATH.message(), "invalid path");
    assert_eq!(Status::NOT_DIRECTORY.message(), "not a directory");
    assert_eq!(Status::READ_ONLY.message(), "read-only mount");
    assert_eq!(
        Status::new(Severity::Error, facility::KERNEL, 0x1f, 0).unwrap().message(),
        "unknown status"
    );
}

#[test]
fn property_valid_statuses_round_trip_their_raw_value() {
    use synos_test_support::property::{run_assert, Config};

    run_assert("status.raw-round-trip", Config::new(0x59_3, 256), |_, _, entropy| {
        let severity = match (entropy.next_u64() % 5) as u8 {
            0 => Severity::Warning,
            1 => Severity::Success,
            2 => Severity::Error,
            3 => Severity::Information,
            _ => Severity::Fatal,
        };
        let facility = (entropy.next_u64() % 0x1000) as u16;
        let code = (entropy.next_u64() % 0x2000) as u16;
        let flags = (entropy.next_u64() % 16) as u8;
        let Some(status) = Status::new(severity, facility, code, flags) else { return false };
        Status::from_raw(status.raw()) == Some(status)
    })
    .expect("generated status fields round-trip");
}
