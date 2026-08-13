#![no_std]
#![forbid(unsafe_code)]

//! Version policy shared by every public SynOS boundary.
//!
//! A decoder remains responsible for validating its bytes. This crate gives
//! callers one stable decision before decoding: accept, accept with a
//! deprecation warning, or reject with a machine-readable compatibility code.

#[cfg(test)]
extern crate alloc;

use core::fmt;

pub const COMPATIBILITY_ERROR_TOO_OLD: &str = "SYNOS-COMPAT-001";
pub const COMPATIBILITY_ERROR_TOO_NEW: &str = "SYNOS-COMPAT-002";
pub const COMPATIBILITY_ERROR_INVALID_RANGE: &str = "SYNOS-COMPAT-003";
pub const COMPATIBILITY_ERROR_MIGRATION_REQUIRED: &str = "SYNOS-COMPAT-004";
pub const DEPRECATION_WARNING_LEGACY: &str = "SYNOS-COMPAT-DEP-001";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ApiVersion {
    pub major: u16,
    pub minor: u16,
}

impl ApiVersion {
    pub const V1: Self = Self { major: 1, minor: 0 };
    pub const V2: Self = Self { major: 2, minor: 0 };

    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub const fn is_zero(self) -> bool {
        self.major == 0 && self.minor == 0
    }

    pub const fn is_before(self, other: Self) -> bool {
        self.major < other.major || (self.major == other.major && self.minor < other.minor)
    }

    pub const fn is_after(self, other: Self) -> bool {
        self.major > other.major || (self.major == other.major && self.minor > other.minor)
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ApiKind {
    Rust = 1,
    Swift = 2,
    Wire = 3,
    Shell = 4,
    Package = 5,
    Snapshot = 6,
    Configuration = 7,
    Sdk = 8,
}

impl ApiKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Swift => "swift",
            Self::Wire => "wire",
            Self::Shell => "shell",
            Self::Package => "package",
            Self::Snapshot => "snapshot",
            Self::Configuration => "configuration",
            Self::Sdk => "user-space SDK",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionRange {
    pub minimum: ApiVersion,
    pub maximum: ApiVersion,
}

impl VersionRange {
    pub const fn new(minimum: ApiVersion, maximum: ApiVersion) -> Self {
        Self { minimum, maximum }
    }

    pub const fn contains(self, version: ApiVersion) -> bool {
        !version.is_before(self.minimum) && !version.is_after(self.maximum)
    }

    pub const fn is_valid(self) -> bool {
        !self.minimum.is_zero() && !self.minimum.is_after(self.maximum)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Migration {
    pub id: &'static str,
    pub from: ApiVersion,
    pub to: ApiVersion,
    pub example: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApiContract {
    pub kind: ApiKind,
    pub name: &'static str,
    pub current: ApiVersion,
    pub supported: VersionRange,
    pub migration: Option<Migration>,
}

impl ApiContract {
    pub const fn accepts(self, offered: ApiVersion) -> bool {
        self.supported.contains(offered)
    }

    pub fn check(self, offered: ApiVersion) -> Result<Compatibility, CompatibilityError> {
        if !self.supported.is_valid() {
            return Err(CompatibilityError::new(
                self.kind,
                COMPATIBILITY_ERROR_INVALID_RANGE,
                offered,
                self.supported,
            ));
        }
        if offered.is_before(self.supported.minimum) {
            return Err(CompatibilityError::new(
                self.kind,
                COMPATIBILITY_ERROR_TOO_OLD,
                offered,
                self.supported,
            ));
        }
        if offered.is_after(self.supported.maximum) {
            return Err(CompatibilityError::new(
                self.kind,
                COMPATIBILITY_ERROR_TOO_NEW,
                offered,
                self.supported,
            ));
        }
        if offered.is_before(self.current) {
            return Ok(Compatibility::Deprecated(DeprecationWarning {
                code: DEPRECATION_WARNING_LEGACY,
                kind: self.kind,
                version: offered,
                replacement: self.current,
            }));
        }
        Ok(Compatibility::Accepted)
    }

    pub fn migrate_to_current(
        self,
        offered: ApiVersion,
    ) -> Result<Migration, CompatibilityError> {
        match self.migration {
            Some(migration)
                if !migration.from.is_before(offered)
                    && !migration.from.is_after(offered)
                    && !migration.to.is_before(self.current)
                    && !migration.to.is_after(self.current) =>
            {
                Ok(migration)
            }
            _ => Err(CompatibilityError::new(
                self.kind,
                COMPATIBILITY_ERROR_MIGRATION_REQUIRED,
                offered,
                self.supported,
            )),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeprecationWarning {
    pub code: &'static str,
    pub kind: ApiKind,
    pub version: ApiVersion,
    pub replacement: ApiVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Compatibility {
    Accepted,
    Deprecated(DeprecationWarning),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityError {
    pub kind: ApiKind,
    pub code: &'static str,
    pub offered: ApiVersion,
    pub supported: VersionRange,
}

impl CompatibilityError {
    pub const fn new(
        kind: ApiKind,
        code: &'static str,
        offered: ApiVersion,
        supported: VersionRange,
    ) -> Self {
        Self {
            kind,
            code,
            offered,
            supported,
        }
    }
}

impl fmt::Display for CompatibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {} API version {} is outside supported range {}..={}",
            self.code,
            self.kind.name(),
            self.offered,
            self.supported.minimum,
            self.supported.maximum
        )
    }
}

pub const RUST_API: ApiContract = ApiContract {
    kind: ApiKind::Rust,
    name: "public Rust API",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const SWIFT_API: ApiContract = ApiContract {
    kind: ApiKind::Swift,
    name: "public Swift API",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const WIRE_API: ApiContract = ApiContract {
    kind: ApiKind::Wire,
    name: "wire API",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const SHELL_API: ApiContract = ApiContract {
    kind: ApiKind::Shell,
    name: "shell API",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const PACKAGE_API: ApiContract = ApiContract {
    kind: ApiKind::Package,
    name: "package API",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const SNAPSHOT_API: ApiContract = ApiContract {
    kind: ApiKind::Snapshot,
    name: "snapshot API",
    current: ApiVersion::V2,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V2),
    migration: Some(Migration {
        id: "snapshot-v1-to-v2",
        from: ApiVersion::V1,
        to: ApiVersion::V2,
        example: "synos-vm snapshot convert --from 1 --to 2 input.vm output.vm",
    }),
};

pub const CONFIGURATION_API: ApiContract = ApiContract {
    kind: ApiKind::Configuration,
    name: "configuration API",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const SDK_API: ApiContract = ApiContract {
    kind: ApiKind::Sdk,
    name: "user-space SDK",
    current: ApiVersion::V1,
    supported: VersionRange::new(ApiVersion::V1, ApiVersion::V1),
    migration: None,
};

pub const ALL_CONTRACTS: [ApiContract; 8] = [
    RUST_API,
    SWIFT_API,
    WIRE_API,
    SHELL_API,
    PACKAGE_API,
    SNAPSHOT_API,
    CONFIGURATION_API,
    SDK_API,
];

pub const fn contract(kind: ApiKind) -> ApiContract {
    match kind {
        ApiKind::Rust => RUST_API,
        ApiKind::Swift => SWIFT_API,
        ApiKind::Wire => WIRE_API,
        ApiKind::Shell => SHELL_API,
        ApiKind::Package => PACKAGE_API,
        ApiKind::Snapshot => SNAPSHOT_API,
        ApiKind::Configuration => CONFIGURATION_API,
        ApiKind::Sdk => SDK_API,
    }
}

/// Compatibility entry point for clients that still send the v1 snapshot.
///
/// New code should negotiate `SNAPSHOT_API` and use the migration plan.
#[deprecated(note = "use SNAPSHOT_API.check() and migrate_to_current()")]
pub const LEGACY_SNAPSHOT_V1: ApiVersion = ApiVersion::V1;

#[cfg(test)]
mod tests {
    use alloc::string::ToString;
    use super::*;

    #[test]
    fn every_public_boundary_has_a_valid_contract() {
        assert_eq!(ALL_CONTRACTS.len(), 8);
        for contract in ALL_CONTRACTS {
            assert!(contract.supported.is_valid());
            assert!(contract.accepts(contract.current));
        }
    }

    #[test]
    fn old_clients_get_stable_errors() {
        let error = WIRE_API.check(ApiVersion::new(0, 9)).unwrap_err();
        assert_eq!(error.code, COMPATIBILITY_ERROR_TOO_OLD);
        assert_eq!(error.to_string(), "SYNOS-COMPAT-001: wire API version 0.9 is outside supported range 1.0..=1.0");

        let error = WIRE_API.check(ApiVersion::new(2, 0)).unwrap_err();
        assert_eq!(error.code, COMPATIBILITY_ERROR_TOO_NEW);
    }

    #[test]
    fn supported_legacy_versions_warn_and_migrate() {
        let compatibility = SNAPSHOT_API.check(ApiVersion::V1).unwrap();
        assert!(matches!(compatibility, Compatibility::Deprecated(_)));
        let migration = SNAPSHOT_API.migrate_to_current(ApiVersion::V1).unwrap();
        assert_eq!(migration.id, "snapshot-v1-to-v2");
    }

    #[test]
    fn current_versions_are_accepted_without_warning() {
        for contract in ALL_CONTRACTS {
            assert_eq!(contract.check(contract.current), Ok(Compatibility::Accepted));
        }
    }

    #[test]
    fn sdk_contract_rejects_unknown_major_versions() {
        let error = SDK_API.check(ApiVersion::new(2, 0)).unwrap_err();
        assert_eq!(error.kind, ApiKind::Sdk);
        assert_eq!(error.code, COMPATIBILITY_ERROR_TOO_NEW);
    }
}
