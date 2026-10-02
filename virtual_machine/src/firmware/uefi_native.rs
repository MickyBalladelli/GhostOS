//! Owned PE/descriptor buffers and borrowed MMU callbacks for native UEFI.
use super::{Mmu, PeImage, PeSection, UefiError, UefiImage, MEMORY_DESCRIPTOR_SIZE};
use std::ffi::c_void;

#[repr(C)]
#[derive(Default)]
struct Pe {
    image_base: u64,
    entry_rva: u32, size_of_image: u32, reloc_rva: u32, reloc_size: u32,
    sections_start: usize, section_count: usize,
}
#[repr(C)]
#[derive(Default)]
struct PeSectionRange { virtual_address: u32, raw_offset: usize, raw_length: usize }
#[repr(C)]
struct Section { virtual_address: u32, bytes: *const u8, length: usize }
#[repr(C)]
struct ImageRange { base: u64, size: u64 }
#[repr(C)]
struct Io {
    read_value: unsafe extern "C-unwind" fn(*mut c_void, u64, u32, *mut u64) -> bool,
    write_value: unsafe extern "C-unwind" fn(*mut c_void, u64, u32, u64) -> bool,
    write_phys: unsafe extern "C-unwind" fn(*mut c_void, u64, *const u8, usize) -> bool,
    context: *mut c_void,
}
impl Io {
    fn new(mmu: &mut Mmu) -> Self {
        Self { read_value, write_value, write_phys, context: mmu as *mut Mmu as *mut c_void }
    }
}
unsafe extern "C-unwind" fn read_value(context: *mut c_void, address: u64, width: u32, output: *mut u64) -> bool {
    let mmu = &*(context as *const Mmu);
    let value = match width {
        2 => mmu.read_u16(address).map(u64::from),
        4 => mmu.read_u32(address).map(u64::from),
        8 => mmu.read_u64(address),
        _ => unreachable!("native UEFI read width"),
    };
    match value { Ok(value) => { output.write(value); true }, Err(_) => false }
}
unsafe extern "C-unwind" fn write_value(context: *mut c_void, address: u64, width: u32, value: u64) -> bool {
    let mmu = &mut *(context as *mut Mmu);
    match width {
        4 => mmu.write_u32(address, value as u32).is_ok(),
        8 => mmu.write_u64(address, value).is_ok(),
        _ => unreachable!("native UEFI write width"),
    }
}
unsafe extern "C-unwind" fn write_phys(context: *mut c_void, address: u64, bytes: *const u8, length: usize) -> bool {
    let mmu = &mut *(context as *mut Mmu);
    mmu.write_phys(address, std::slice::from_raw_parts(bytes, length)).is_ok()
}
fn result(code: u32) -> Result<(), UefiError> {
    match code {
        0 => Ok(()),
        1 => Err(UefiError::InvalidImage),
        2 => Err(UefiError::Unsupported),
        3 => Err(UefiError::OutOfMemory),
        4 => Err(UefiError::LoadFailed),
        5 => panic!("attempt to add with overflow"),
        _ => unreachable!("native UEFI result"),
    }
}

pub(super) fn parse(bytes: &[u8]) -> Result<PeImage, UefiError> {
    let mut pe = Pe::default();
    result(unsafe { ghostos_vm_uefi_pe_parse(bytes.as_ptr(), bytes.len(), &mut pe) })?;
    let mut sections = Vec::new();
    for index in 0..pe.section_count {
        let mut section = PeSectionRange::default();
        // The C parser already validated every section. The same immutable
        // bytes and returned metadata remain borrowed during these lookups.
        assert!(unsafe { ghostos_vm_uefi_pe_section_at(bytes.as_ptr(), bytes.len(), &pe, index, &mut section) }, "validated native PE section");
        let raw_data = if section.raw_length == 0 { Vec::new() }
            else { bytes[section.raw_offset..section.raw_offset + section.raw_length].to_vec() };
        sections.push(PeSection { virtual_address: section.virtual_address, raw_data });
    }
    Ok(PeImage { image_base: pe.image_base, entry_rva: pe.entry_rva,
        size_of_image: pe.size_of_image, reloc_rva: pe.reloc_rva, reloc_size: pe.reloc_size, sections })
}

pub(super) fn map(mmu: &mut Mmu, image: &PeImage, requested: u64, memory_size: u64, images: &[UefiImage]) -> Result<u64, UefiError> {
    let pe = Pe { image_base: image.image_base, entry_rva: image.entry_rva,
        size_of_image: image.size_of_image, reloc_rva: image.reloc_rva, reloc_size: image.reloc_size,
        sections_start: 0, section_count: image.sections.len() };
    let sections: Vec<_> = image.sections.iter().map(|section| Section {
        virtual_address: section.virtual_address, bytes: section.raw_data.as_ptr(), length: section.raw_data.len(),
    }).collect();
    let ranges: Vec<_> = images.iter().map(|image| ImageRange { base: image.base, size: image.size }).collect();
    // Callbacks exclusively borrow this MMU for the synchronous C invocation.
    // Section/range buffers and their byte backing storage remain alive.
    let io = Io::new(mmu);
    result(unsafe { ghostos_vm_uefi_map_pe(&pe, sections.as_ptr(), sections.len(), requested, memory_size,
        ranges.as_ptr(), ranges.len(), cfg!(debug_assertions), &io) })?;
    Ok(requested)
}
#[cfg(test)]
pub(super) fn relocate(mmu: &mut Mmu, base: u64, delta: i128, rva: u32, size: u32) -> Result<(), UefiError> {
    let io = Io::new(mmu);
    // Both supported relocation widths use only the low 64 bits of the delta.
    result(unsafe { ghostos_vm_uefi_relocate(base, delta as u64, rva, size, cfg!(debug_assertions), &io) })
}
pub(super) fn memory_map(memory_size: u64, images: &[UefiImage]) -> Vec<[u8; MEMORY_DESCRIPTOR_SIZE]> {
    let ranges: Vec<_> = images.iter().map(|image| ImageRange { base: image.base, size: image.size }).collect();
    let needed = unsafe { ghostos_vm_uefi_memory_map(memory_size, ranges.as_ptr(), ranges.len(), std::ptr::null_mut(), 0) };
    let mut output = vec![[0; MEMORY_DESCRIPTOR_SIZE]; needed];
    let written = unsafe { ghostos_vm_uefi_memory_map(memory_size, ranges.as_ptr(), ranges.len(), output.as_mut_ptr().cast(), output.len()) };
    assert_eq!(written, needed, "native UEFI memory-map size");
    output
}
pub(super) fn service_stub(id: u64) -> [u8; 8] {
    let mut output = [0; 8];
    unsafe { ghostos_vm_uefi_service_stub(id, output.as_mut_ptr()); }
    output
}
pub(super) fn table_header(signature: u64) -> [u8; 24] {
    let mut output = [0; 24];
    unsafe { ghostos_vm_uefi_table_header(signature, output.as_mut_ptr()); }
    output
}
pub(super) fn rsdp() -> [u8; 36] {
    let mut output = [0; 36];
    unsafe { ghostos_vm_uefi_rsdp(output.as_mut_ptr()); }
    output
}

extern "C" {
    fn ghostos_vm_uefi_pe_parse(bytes: *const u8, length: usize, output: *mut Pe) -> u32;
    fn ghostos_vm_uefi_pe_section_at(bytes: *const u8, length: usize, pe: *const Pe, index: usize, output: *mut PeSectionRange) -> bool;
    fn ghostos_vm_uefi_memory_map(memory_size: u64, images: *const ImageRange, count: usize, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_vm_uefi_service_stub(id: u64, output: *mut u8);
    fn ghostos_vm_uefi_table_header(signature: u64, output: *mut u8);
    fn ghostos_vm_uefi_rsdp(output: *mut u8);
}
extern "C-unwind" {
    fn ghostos_vm_uefi_map_pe(image: *const Pe, sections: *const Section, count: usize, requested: u64,
        memory_size: u64, images: *const ImageRange, image_count: usize, checked: bool, io: *const Io) -> u32;
    #[cfg(test)]
    fn ghostos_vm_uefi_relocate(base: u64, delta: u64, rva: u32, size: u32, checked: bool, io: *const Io) -> u32;
}
