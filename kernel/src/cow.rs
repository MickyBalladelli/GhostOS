use crate::allocator::EarlyFrameAllocator;
use crate::capability::PhysicalRange;
use crate::task::AddressSpaceId;

pub const MAX_COW_PAGES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowError {
    InvalidRange,
    NotTracked,
    AlreadyTracked,
    Capacity,
    InvalidOwner,
    Allocation,
    CopyFailed,
    ReclaimFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CowPageInfo {
    pub frame: u64,
    pub owner: AddressSpaceId,
    pub references: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CowWriteResult {
    Exclusive { frame: u64 },
    Copied { old_frame: u64, new_frame: u64 },
}

pub trait CowPageCopier {
    fn copy_page(&mut self, source: u64, destination: u64) -> bool;
}

#[derive(Clone, Copy)]
struct CowPage {
    info: CowPageInfo,
}

pub struct CowManager<const CAPACITY: usize = MAX_COW_PAGES> {
    pages: [Option<CowPage>; CAPACITY],
}

impl<const CAPACITY: usize> CowManager<CAPACITY> {
    pub const fn new() -> Self {
        Self { pages: [None; CAPACITY] }
    }

    pub fn register(
        &mut self,
        owner: AddressSpaceId,
        backing: PhysicalRange,
    ) -> Result<(), CowError> {
        validate_range(backing)?;
        let count = page_count(backing)?;
        if self.pages.iter().filter(|page| page.is_none()).count() < count {
            return Err(CowError::Capacity)
        }
        for frame in frames(backing) {
            if self.find(frame).is_some() {
                return Err(CowError::AlreadyTracked)
            }
        }
        for frame in frames(backing) {
            let slot = self
                .pages
                .iter()
                .position(Option::is_none)
                .ok_or(CowError::Capacity)?;
            self.pages[slot] = Some(CowPage {
                info: CowPageInfo {
                    frame,
                    owner,
                    references: 1,
                },
            });
        }
        Ok(())
    }

    pub fn share(
        &mut self,
        owner: AddressSpaceId,
        backing: PhysicalRange,
    ) -> Result<(), CowError> {
        validate_range(backing)?;
        for frame in frames(backing) {
            let slot = self.find(frame).ok_or(CowError::NotTracked)?;
            let page = self.pages[slot].ok_or(CowError::NotTracked)?;
            if page.info.owner != owner {
                return Err(CowError::InvalidOwner)
            }
            if page.info.references == u32::MAX {
                return Err(CowError::Capacity)
            }
        }
        for frame in frames(backing) {
            let slot = self.find(frame).ok_or(CowError::NotTracked)?;
            let page = self.pages[slot].as_mut().ok_or(CowError::NotTracked)?;
            page.info.references += 1;
        }
        Ok(())
    }

    pub fn write_fault(
        &mut self,
        address_space: AddressSpaceId,
        frame: u64,
        allocator: &mut EarlyFrameAllocator,
        copier: &mut impl CowPageCopier,
    ) -> Result<CowWriteResult, CowError> {
        if frame % crate::FRAME_SIZE != 0 {
            return Err(CowError::InvalidRange)
        }
        let slot = self.find(frame).ok_or(CowError::NotTracked)?;
        let page = self.pages[slot].ok_or(CowError::NotTracked)?;
        if page.info.references == 1 {
            return Ok(CowWriteResult::Exclusive { frame })
        }
        let new_slot = self
            .pages
            .iter()
            .position(Option::is_none)
            .ok_or(CowError::Capacity)?;
        let new_frame = allocator
            .allocate_for_owner(address_space)
            .map_err(|_| CowError::Allocation)?;
        if !copier.copy_page(frame, new_frame) {
            allocator
                .reclaim(address_space, new_frame)
                .map_err(|_| CowError::ReclaimFailed)?;
            return Err(CowError::CopyFailed)
        }
        let old_page = self.pages[slot].as_mut().ok_or(CowError::NotTracked)?;
        old_page.info.references -= 1;
        self.pages[new_slot] = Some(CowPage {
            info: CowPageInfo {
                frame: new_frame,
                owner: address_space,
                references: 1,
            },
        });
        Ok(CowWriteResult::Copied {
            old_frame: frame,
            new_frame,
        })
    }

    pub fn release(
        &mut self,
        backing: PhysicalRange,
        allocator: &mut EarlyFrameAllocator,
    ) -> Result<u64, CowError> {
        validate_range(backing)?;
        let mut reclaimed = 0;
        for frame in frames(backing) {
            let slot = self.find(frame).ok_or(CowError::NotTracked)?;
            let page = self.pages[slot].ok_or(CowError::NotTracked)?;
            if page.info.references == 1 {
                allocator
                    .reclaim(page.info.owner, page.info.frame)
                    .map_err(|_| CowError::ReclaimFailed)?;
            }
            let page = self.pages[slot].as_mut().ok_or(CowError::NotTracked)?;
            page.info.references -= 1;
            if page.info.references == 0 {
                self.pages[slot] = None;
                reclaimed += 1;
            }
        }
        Ok(reclaimed)
    }

    pub fn info(&self, frame: u64) -> Option<CowPageInfo> {
        self.find(frame)
            .and_then(|slot| self.pages[slot].map(|page| page.info))
    }

    fn find(&self, frame: u64) -> Option<usize> {
        self.pages
            .iter()
            .position(|page| page.is_some_and(|page| page.info.frame == frame))
    }

}

impl<const CAPACITY: usize> Default for CowManager<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_range(backing: PhysicalRange) -> Result<(), CowError> {
    if backing.start % crate::FRAME_SIZE != 0
        || backing.length == 0
        || backing.length % crate::FRAME_SIZE != 0
    {
        Err(CowError::InvalidRange)
    } else {
        Ok(())
    }
}

fn page_count(backing: PhysicalRange) -> Result<usize, CowError> {
    usize::try_from(backing.length / crate::FRAME_SIZE).map_err(|_| CowError::Capacity)
}

fn frames(backing: PhysicalRange) -> impl Iterator<Item = u64> {
    (0..backing.length / crate::FRAME_SIZE)
        .map(move |index| backing.start + index * crate::FRAME_SIZE)
}
