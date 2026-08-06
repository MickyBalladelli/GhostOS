use synos_status::Status;
use synos_system_model::ContentId;

use crate::{BuildRequest, Error, Profile, Target, Text, MAX_PATH_BYTES};

pub const COMPILER_IDENTITY: u64 = 0x5359_4e4f_5255_5354;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CompilerCapabilities(u32);

impl CompilerCapabilities {
    pub const READ_SOURCE: Self = Self(1 << 0);
    pub const WRITE_BUILD: Self = Self(1 << 1);
    pub const READ_TOOLCHAIN: Self = Self(1 << 2);
    pub const READ_REGISTRY: Self = Self(1 << 3);
    pub const WRITE_OUTPUT: Self = Self(1 << 4);
    pub const IPC: Self = Self(1 << 5);
    pub const EXECUTE_TOOLS: Self = Self(1 << 6);
    pub const NETWORK: Self = Self(1 << 7);
    pub const DEVICES: Self = Self(1 << 8);
    pub const SECRETS: Self = Self(1 << 9);
    pub const PROCESS_CONTROL: Self = Self(1 << 10);

    pub const MINIMUM: Self = Self(
        Self::READ_SOURCE.0
            | Self::WRITE_BUILD.0
            | Self::READ_TOOLCHAIN.0
            | Self::READ_REGISTRY.0
            | Self::WRITE_OUTPUT.0
            | Self::IPC.0
            | Self::EXECUTE_TOOLS.0,
    );

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompilerSecurityPolicy {
    identity: u64,
    capabilities: CompilerCapabilities,
    pub source_root: Text<MAX_PATH_BYTES>,
    pub toolchain_root: Text<MAX_PATH_BYTES>,
    pub registry_root: Text<MAX_PATH_BYTES>,
    pub build_root: Text<MAX_PATH_BYTES>,
    pub cache_root: Text<MAX_PATH_BYTES>,
    pub output_root: Text<MAX_PATH_BYTES>,
    pub active_package_root: Text<MAX_PATH_BYTES>,
    pub trusted_key_root: Text<MAX_PATH_BYTES>,
}

impl CompilerSecurityPolicy {
    pub fn minimum(identity: u64) -> Result<Self, Error> {
        if identity == 0 {
            return Err(Error::IdentityMismatch);
        }
        Ok(Self {
            identity,
            capabilities: CompilerCapabilities::MINIMUM,
            source_root: Text::new("/system/sources")?,
            toolchain_root: Text::new("/system/toolchains")?,
            registry_root: Text::new("/system/registries")?,
            build_root: Text::new("/system/builds")?,
            cache_root: Text::new("/system/cache/compiler")?,
            output_root: Text::new("/system/bundles")?,
            active_package_root: Text::new("/system/compiler")?,
            trusted_key_root: Text::new("/system/trusted-keys")?,
        })
    }

    pub const fn identity(self) -> u64 {
        self.identity
    }

    pub const fn capabilities(self) -> CompilerCapabilities {
        self.capabilities
    }

    pub const fn with_capabilities(mut self, capabilities: CompilerCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn validate(&self) -> Result<(), Error> {
        if self.identity == 0 || !self.capabilities.contains(CompilerCapabilities::MINIMUM) {
            return Err(Error::IdentityMismatch);
        }
        let roots = [
            self.source_root.as_str(),
            self.toolchain_root.as_str(),
            self.registry_root.as_str(),
            self.build_root.as_str(),
            self.cache_root.as_str(),
            self.output_root.as_str(),
            self.active_package_root.as_str(),
            self.trusted_key_root.as_str(),
        ];
        for (index, root) in roots.iter().enumerate() {
            if !root.starts_with('/')
                || root.len() <= 1
                || roots[..index]
                    .iter()
                    .any(|existing| *existing == *root || is_member(existing, root) || is_member(root, existing))
            {
                return Err(Error::StorageSeparation);
            }
        }
        Ok(())
    }

    pub fn authorize_write_path(&self, path: &str) -> Result<(), Error> {
        self.validate()?;
        if is_member(self.active_package_root.as_str(), path)
            || is_member(self.trusted_key_root.as_str(), path)
        {
            return Err(Error::CapabilityDenied);
        }
        if is_member(self.build_root.as_str(), path)
            || is_member(self.cache_root.as_str(), path)
            || is_member(self.output_root.as_str(), path)
        {
            Ok(())
        } else {
            Err(Error::StorageSeparation)
        }
    }

    pub fn authorize(&self, request: &BuildRequest) -> Result<(), Error> {
        self.validate()?;
        if !self.capabilities.contains(CompilerCapabilities::MINIMUM) {
            return Err(Error::CapabilityDenied);
        }
        if !is_member(self.source_root.as_str(), request.source_root.as_str()) {
            return Err(Error::StorageSeparation);
        }
        if request.network == crate::NetworkPolicy::Allowed
            && !self.capabilities.contains(CompilerCapabilities::NETWORK)
        {
            return Err(Error::CapabilityDenied);
        }
        Ok(())
    }

    pub fn authorize_dangerous(
        &self,
        network: bool,
        devices: bool,
        secrets: bool,
        process_control: bool,
    ) -> Result<(), Error> {
        let requested = [
            (network, CompilerCapabilities::NETWORK),
            (devices, CompilerCapabilities::DEVICES),
            (secrets, CompilerCapabilities::SECRETS),
            (process_control, CompilerCapabilities::PROCESS_CONTROL),
        ];
        if requested
            .iter()
            .any(|(requested, capability)| *requested && !self.capabilities.contains(*capability))
        {
            return Err(Error::CapabilityDenied);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildAuditRecord {
    pub identity: u64,
    pub job: u64,
    pub source: ContentId,
    pub dependencies: ContentId,
    pub toolchain: ContentId,
    pub package: ContentId,
    pub payload: ContentId,
    pub target: Target,
    pub profile: Profile,
    pub capability_bits: u32,
    pub status: Status,
}

impl BuildAuditRecord {
    pub fn content_id(self) -> ContentId {
        let mut bytes = [0; 32 * 5 + 8 + 8 + 4 + 1 + 1 + 4];
        let mut cursor = 0;
        for id in [
            self.source,
            self.dependencies,
            self.toolchain,
            self.package,
            self.payload,
        ] {
            bytes[cursor..cursor + 32].copy_from_slice(id.as_bytes());
            cursor += 32;
        }
        bytes[cursor..cursor + 8].copy_from_slice(&self.identity.to_be_bytes());
        cursor += 8;
        bytes[cursor..cursor + 8].copy_from_slice(&self.job.to_be_bytes());
        cursor += 8;
        bytes[cursor] = self.target as u8;
        cursor += 1;
        bytes[cursor] = self.profile as u8;
        cursor += 1;
        bytes[cursor..cursor + 4].copy_from_slice(&self.capability_bits.to_be_bytes());
        cursor += 4;
        bytes[cursor..cursor + 4].copy_from_slice(&self.status.raw().to_be_bytes());
        ContentId::hash(&bytes)
    }
}

fn is_member(root: &str, path: &str) -> bool {
    path == root || path.strip_prefix(root).is_some_and(|suffix| suffix.starts_with('/'))
}
