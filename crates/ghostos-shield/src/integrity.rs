//! Runtime measurements for executable pages.
//!
//! The package daemon supplies a page manifest only after a signed,
//! content-addressed bundle has passed its trust policy. This module keeps
//! the hot path bounded: a process is bound to one immutable image, and each
//! executable page can be checked directly against its storage measurement.

use ghostos_fabric::PAGE_SIZE;
use ghostos_observability::{EventField, Level, audit_event, field};
use ghostos_status::{IntoStatus, Status};
use ghostos_system_model::ContentId;

pub const DEFAULT_PAGE_HASH_CAPACITY: usize = 256;
pub const DEFAULT_IMAGE_CAPACITY: usize = 64;
pub const DEFAULT_PROCESS_CAPACITY: usize = 128;
const PAGE_BYTES: usize = PAGE_SIZE as usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegrityError {
    BufferTooSmall { required: usize },
    Capacity,
    Duplicate,
    InvalidImage,
    InvalidInput,
    ImageNotFound,
    PageHashMismatch {
        subject: u64,
        page_index: usize,
        expected: ContentId,
        observed: ContentId,
    },
    StorageDigestMismatch {
        expected: ContentId,
        observed: ContentId,
    },
    PageNotFound,
    ProcessNotFound,
}

impl IntoStatus for IntegrityError {
    fn status(self) -> Status {
        match self {
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::Duplicate | Self::InvalidImage | Self::InvalidInput => Status::INVALID_ARGUMENT,
            Self::ImageNotFound | Self::PageNotFound | Self::ProcessNotFound => {
                Status::NOT_FOUND
            }
            Self::PageHashMismatch { .. } | Self::StorageDigestMismatch { .. } => {
                Status::ACCESS_DENIED
            }
        }
    }
}

/// Signed-storage page measurements for one content-addressed executable.
/// The package signature authenticates the package identity; this manifest is
/// derived from that immutable payload before it is handed to the verifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageHashManifest<const PAGES: usize = DEFAULT_PAGE_HASH_CAPACITY> {
    package: ContentId,
    byte_length: u64,
    page_count: usize,
    hashes: [Option<ContentId>; PAGES],
}

impl<const PAGES: usize> PageHashManifest<PAGES> {
    pub fn new(
        package: ContentId,
        byte_length: u64,
        hashes: &[ContentId],
    ) -> Result<Self, IntegrityError> {
        if package.is_zero() || byte_length == 0 || hashes.is_empty() {
            return Err(IntegrityError::InvalidImage)
        }
        let expected_pages = page_count(byte_length)?;
        if hashes.len() != expected_pages || hashes.len() > PAGES {
            return Err(IntegrityError::BufferTooSmall {
                required: expected_pages,
            })
        }
        let mut stored = [None; PAGES];
        for (slot, hash) in stored.iter_mut().zip(hashes) {
            if hash.is_zero() {
                return Err(IntegrityError::InvalidImage)
            }
            *slot = Some(*hash)
        }
        Ok(Self {
            package,
            byte_length,
            page_count: expected_pages,
            hashes: stored,
        })
    }

    pub fn from_bytes(
        package: ContentId,
        expected_payload: ContentId,
        bytes: &[u8],
    ) -> Result<Self, IntegrityError> {
        if bytes.is_empty() {
            return Err(IntegrityError::InvalidImage)
        }
        let observed_payload = ContentId::hash(bytes);
        if observed_payload != expected_payload {
            return Err(IntegrityError::StorageDigestMismatch {
                expected: expected_payload,
                observed: observed_payload,
            })
        }
        let expected_pages = bytes
            .len()
            .checked_add(PAGE_BYTES - 1)
            .map(|length| length / PAGE_BYTES)
            .ok_or(IntegrityError::BufferTooSmall { required: usize::MAX })?;
        if expected_pages > PAGES {
            return Err(IntegrityError::BufferTooSmall {
                required: expected_pages,
            })
        }
        let mut hashes = [ContentId::from_bytes([0; 32]); PAGES];
        for (index, chunk) in bytes.chunks(PAGE_BYTES).enumerate() {
            hashes[index] = ContentId::hash(chunk);
        }
        Self::new(package, bytes.len() as u64, &hashes[..expected_pages])
    }

    pub const fn package(&self) -> ContentId {
        self.package
    }

    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }

    pub const fn page_count(&self) -> usize {
        self.page_count
    }

    pub fn hash_at(&self, page_index: usize) -> Result<ContentId, IntegrityError> {
        self.hashes
            .get(page_index)
            .and_then(|hash| *hash)
            .ok_or(IntegrityError::PageNotFound)
    }

    fn page_length(&self, page_index: usize) -> Result<usize, IntegrityError> {
        if page_index >= self.page_count {
            return Err(IntegrityError::PageNotFound)
        }
        let offset = page_index
            .checked_mul(PAGE_BYTES)
            .ok_or(IntegrityError::PageNotFound)?;
        let remaining = usize::try_from(self.byte_length)
            .map_err(|_| IntegrityError::PageNotFound)?
            .checked_sub(offset)
            .ok_or(IntegrityError::PageNotFound)?;
        Ok(remaining.min(PAGE_BYTES))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageSample<'a> {
    pub address: u64,
    pub bytes: &'a [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedPage {
    pub subject: u64,
    pub package: ContentId,
    pub page_index: usize,
    pub digest: ContentId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegrityReport {
    pub pages_checked: usize,
    pub mismatches: usize,
    pub first_mismatch_address: Option<u64>,
}

#[derive(Clone, Copy)]
struct ImageRecord<const PAGES: usize> {
    manifest: PageHashManifest<PAGES>,
}

#[derive(Clone, Copy)]
struct ProcessBinding {
    subject: u64,
    package: ContentId,
    base: u64,
}

/// Bounded runtime page verifier. The caller supplies bytes read through the
/// kernel's executable mapping, so no unsafe memory access is needed here.
pub struct RuntimePageVerifier<
    const IMAGES: usize = DEFAULT_IMAGE_CAPACITY,
    const PROCESSES: usize = DEFAULT_PROCESS_CAPACITY,
    const PAGES: usize = DEFAULT_PAGE_HASH_CAPACITY,
> {
    images: [Option<ImageRecord<PAGES>>; IMAGES],
    processes: [Option<ProcessBinding>; PROCESSES],
}

impl<const IMAGES: usize, const PROCESSES: usize, const PAGES: usize>
    RuntimePageVerifier<IMAGES, PROCESSES, PAGES>
{
    pub const fn new() -> Self {
        Self {
            images: [None; IMAGES],
            processes: [None; PROCESSES],
        }
    }

    pub fn register_image(
        &mut self,
        manifest: PageHashManifest<PAGES>,
    ) -> Result<(), IntegrityError> {
        if self
            .images
            .iter()
            .flatten()
            .any(|image| image.manifest.package() == manifest.package())
        {
            return Err(IntegrityError::Duplicate)
        }
        let slot = self
            .images
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(IntegrityError::Capacity)?;
        *slot = Some(ImageRecord { manifest });
        Ok(())
    }

    pub fn unregister_image(&mut self, package: ContentId) -> Result<(), IntegrityError> {
        let slot = self
            .images
            .iter()
            .position(|entry| entry.is_some_and(|image| image.manifest.package() == package))
            .ok_or(IntegrityError::ImageNotFound)?;
        if self
            .processes
            .iter()
            .flatten()
            .any(|process| process.package == package)
        {
            return Err(IntegrityError::InvalidInput)
        }
        self.images[slot] = None;
        Ok(())
    }

    pub fn bind_process(
        &mut self,
        subject: u64,
        package: ContentId,
        base: u64,
    ) -> Result<(), IntegrityError> {
        if subject == 0 || base % PAGE_SIZE != 0 {
            return Err(IntegrityError::InvalidInput)
        }
        if !self.has_image(package) {
            return Err(IntegrityError::ImageNotFound)
        }
        if self
            .processes
            .iter()
            .flatten()
            .any(|process| process.subject == subject)
        {
            return Err(IntegrityError::Duplicate)
        }
        let slot = self
            .processes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(IntegrityError::Capacity)?;
        *slot = Some(ProcessBinding {
            subject,
            package,
            base,
        });
        Ok(())
    }

    pub fn unbind_process(&mut self, subject: u64) -> Result<(), IntegrityError> {
        let slot = self
            .processes
            .iter()
            .position(|entry| entry.is_some_and(|process| process.subject == subject))
            .ok_or(IntegrityError::ProcessNotFound)?;
        self.processes[slot] = None;
        Ok(())
    }

    pub fn verify_page(
        &self,
        subject: u64,
        address: u64,
        bytes: &[u8],
    ) -> Result<VerifiedPage, IntegrityError> {
        let process = self
            .processes
            .iter()
            .flatten()
            .find(|process| process.subject == subject)
            .copied()
            .ok_or(IntegrityError::ProcessNotFound)?;
        let offset = address
            .checked_sub(process.base)
            .ok_or(IntegrityError::PageNotFound)?;
        if offset % PAGE_SIZE != 0 {
            return Err(IntegrityError::InvalidInput)
        }
        let page_index = usize::try_from(offset / PAGE_SIZE)
            .map_err(|_| IntegrityError::PageNotFound)?;
        let image = self
            .images
            .iter()
            .flatten()
            .find(|image| image.manifest.package() == process.package)
            .ok_or(IntegrityError::ImageNotFound)?;
        let expected_length = image.manifest.page_length(page_index)?;
        if bytes.len() != expected_length {
            return Err(IntegrityError::InvalidInput)
        }
        let expected = image.manifest.hash_at(page_index)?;
        let observed = ContentId::hash(bytes);
        if expected != observed {
            audit_event!(
                Level::Critical,
                EventField::unsigned(field::CALLER, subject),
                EventField::unsigned(field::ADDRESS, address),
                EventField::unsigned(field::OPERATION, page_index as u64),
                EventField::status(Status::ACCESS_DENIED),
            );
            return Err(IntegrityError::PageHashMismatch {
                subject,
                page_index,
                expected,
                observed,
            })
        }
        Ok(VerifiedPage {
            subject,
            package: process.package,
            page_index,
            digest: observed,
        })
    }

    /// Audit every sampled executable page and keep scanning after a mismatch
    /// so one sweep reports the full damage set.
    pub fn audit_process(
        &self,
        subject: u64,
        samples: &[PageSample<'_>],
    ) -> Result<IntegrityReport, IntegrityError> {
        let mut report = IntegrityReport {
            pages_checked: 0,
            mismatches: 0,
            first_mismatch_address: None,
        };
        for sample in samples {
            report.pages_checked += 1;
            match self.verify_page(subject, sample.address, sample.bytes) {
                Ok(_) => {}
                Err(IntegrityError::PageHashMismatch { .. }) => {
                    report.mismatches += 1;
                    if report.first_mismatch_address.is_none() {
                        report.first_mismatch_address = Some(sample.address);
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(report)
    }

    pub fn has_image(&self, package: ContentId) -> bool {
        self.images
            .iter()
            .flatten()
            .any(|image| image.manifest.package() == package)
    }
}

impl<const IMAGES: usize, const PROCESSES: usize, const PAGES: usize> Default
    for RuntimePageVerifier<IMAGES, PROCESSES, PAGES>
{
    fn default() -> Self {
        Self::new()
    }
}

fn page_count(byte_length: u64) -> Result<usize, IntegrityError> {
    let length = usize::try_from(byte_length)
        .map_err(|_| IntegrityError::BufferTooSmall { required: usize::MAX })?;
    length
        .checked_add(PAGE_BYTES - 1)
        .map(|value| value / PAGE_BYTES)
        .ok_or(IntegrityError::BufferTooSmall { required: usize::MAX })
}
