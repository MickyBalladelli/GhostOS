use core::ffi::c_void;

use crate::{
    EFI_SUCCESS, EfiBootServices, EfiGuid, EfiHandle, EfiStatus,
};

const EFI_NOT_FOUND: EfiStatus = 0x8000_0000_0000_000e;
const BY_PROTOCOL: u32 = 2;
const DEVICE_PATH_BUFFER_SIZE: usize = 1024;
const MEDIA_DEVICE_PATH: u8 = 0x04;
const MEDIA_FILEPATH_DP: u8 = 0x04;
const END_DEVICE_PATH_TYPE: u8 = 0x7f;
const END_ENTIRE_DEVICE_PATH_SUBTYPE: u8 = 0xff;

const LOADED_IMAGE_PROTOCOL: EfiGuid = EfiGuid {
    data1: 0x5b1b_31a1,
    data2: 0x9562,
    data3: 0x11d2,
    data4: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

const DEVICE_PATH_PROTOCOL: EfiGuid = EfiGuid {
    data1: 0x0957_6e91,
    data2: 0x6d3f,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

const SIMPLE_FILE_SYSTEM_PROTOCOL: EfiGuid = EfiGuid {
    data1: 0x964e_5b22,
    data2: 0x6459,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

const WINDOWS_PATH: &[u16] = &[
    '\\' as u16, 'E' as u16, 'F' as u16, 'I' as u16, '\\' as u16,
    'M' as u16, 'i' as u16, 'c' as u16, 'r' as u16, 'o' as u16,
    's' as u16, 'o' as u16, 'f' as u16, 't' as u16, '\\' as u16,
    'B' as u16, 'o' as u16, 'o' as u16, 't' as u16, '\\' as u16,
    'b' as u16, 'o' as u16, 'o' as u16, 't' as u16, 'm' as u16,
    'g' as u16, 'f' as u16, 'w' as u16, '.' as u16, 'e' as u16,
    'f' as u16, 'i' as u16,
];

const GRUB_PATHS: &[&[u16]] = &[
    &[
        '\\' as u16, 'E' as u16, 'F' as u16, 'I' as u16, '\\' as u16,
        'g' as u16, 'r' as u16, 'u' as u16, 'b' as u16, '\\' as u16,
        'g' as u16, 'r' as u16, 'u' as u16, 'b' as u16, 'x' as u16,
        '6' as u16, '4' as u16, '.' as u16, 'e' as u16, 'f' as u16,
        'i' as u16,
    ],
    &[
        '\\' as u16, 'E' as u16, 'F' as u16, 'I' as u16, '\\' as u16,
        'u' as u16, 'b' as u16, 'u' as u16, 'n' as u16, 't' as u16,
        'u' as u16, '\\' as u16, 'g' as u16, 'r' as u16, 'u' as u16,
        'b' as u16, 'x' as u16, '6' as u16, '4' as u16, '.' as u16,
        'e' as u16, 'f' as u16, 'i' as u16,
    ],
    &[
        '\\' as u16, 'E' as u16, 'F' as u16, 'I' as u16, '\\' as u16,
        'f' as u16, 'e' as u16, 'd' as u16, 'o' as u16, 'r' as u16,
        'a' as u16, '\\' as u16, 'g' as u16, 'r' as u16, 'u' as u16,
        'b' as u16, 'x' as u16, '6' as u16, '4' as u16, '.' as u16,
        'e' as u16, 'f' as u16, 'i' as u16,
    ],
    &[
        '\\' as u16, 'E' as u16, 'F' as u16, 'I' as u16, '\\' as u16,
        'd' as u16, 'e' as u16, 'b' as u16, 'i' as u16, 'a' as u16,
        'n' as u16, '\\' as u16, 'g' as u16, 'r' as u16, 'u' as u16,
        'b' as u16, 'x' as u16, '6' as u16, '4' as u16, '.' as u16,
        'e' as u16, 'f' as u16, 'i' as u16,
    ],
];

#[repr(C)]
struct EfiLoadedImage {
    revision: u32,
    parent_handle: EfiHandle,
    system_table: *mut c_void,
    device_handle: EfiHandle,
    file_path: *mut c_void,
    reserved: *mut c_void,
    load_options_size: u32,
    load_options: *mut c_void,
    image_base: *mut c_void,
    image_size: u64,
    image_code_type: u32,
    image_data_type: u32,
    unload: usize,
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct DevicePathHeader {
    device_type: u8,
    sub_type: u8,
    length: [u8; 2],
}

static mut DEVICE_PATH_BUFFER: [u8; DEVICE_PATH_BUFFER_SIZE] =
    [0; DEVICE_PATH_BUFFER_SIZE];

pub(crate) unsafe fn windows(
    image: EfiHandle,
    services: *mut EfiBootServices,
) -> EfiStatus {
    unsafe { start_path(image, services, WINDOWS_PATH) }
}

pub(crate) unsafe fn grub(
    image: EfiHandle,
    services: *mut EfiBootServices,
) -> EfiStatus {
    for path in GRUB_PATHS {
        let status = unsafe { start_path(image, services, path) };
        if status != EFI_NOT_FOUND {
            return status
        }
    }
    EFI_NOT_FOUND
}

unsafe fn start_path(
    parent_image: EfiHandle,
    services: *mut EfiBootServices,
    path: &[u16],
) -> EfiStatus {
    let mut loaded_image_interface = core::ptr::null_mut();
    let status = unsafe {
        ((*services).handle_protocol)(
            parent_image,
            &LOADED_IMAGE_PROTOCOL,
            &mut loaded_image_interface,
        )
    };
    if status != EFI_SUCCESS || loaded_image_interface.is_null() {
        return status
    }

    let loaded_image = loaded_image_interface.cast::<EfiLoadedImage>();
    let device_handle = unsafe { (*loaded_image).device_handle };
    let status = unsafe {
        start_on_device(parent_image, services, device_handle, path)
    };
    if status == EFI_SUCCESS {
        return status
    }

    let mut handle_count = 0;
    let mut handles = core::ptr::null_mut();
    let locate_status = unsafe {
        ((*services).locate_handle_buffer)(
            BY_PROTOCOL,
            &SIMPLE_FILE_SYSTEM_PROTOCOL,
            core::ptr::null_mut(),
            &mut handle_count,
            &mut handles,
        )
    };
    if locate_status != EFI_SUCCESS || handles.is_null() {
        return status
    }

    let mut last_status = status;
    for index in 0..handle_count {
        let candidate = unsafe { *handles.add(index) };
        if candidate == device_handle {
            continue
        }
        last_status = unsafe {
            start_on_device(parent_image, services, candidate, path)
        };
        if last_status == EFI_SUCCESS {
            break
        }
    }
    unsafe {
        ((*services).free_pool)(handles.cast());
    }
    last_status
}

unsafe fn start_on_device(
    parent_image: EfiHandle,
    services: *mut EfiBootServices,
    device_handle: EfiHandle,
    path: &[u16],
) -> EfiStatus {
    let mut device_path_interface = core::ptr::null_mut();
    let status = unsafe {
        ((*services).handle_protocol)(
            device_handle,
            &DEVICE_PATH_PROTOCOL,
            &mut device_path_interface,
        )
    };
    if status != EFI_SUCCESS || device_path_interface.is_null() {
        return status
    }

    let full_path = unsafe { build_device_path(device_path_interface.cast(), path) };
    let Some(full_path) = full_path else {
        return EFI_NOT_FOUND
    };

    let mut child_image = core::ptr::null_mut();
    let status = unsafe {
        ((*services).load_image)(
            false,
            parent_image,
            full_path.cast::<c_void>(),
            core::ptr::null(),
            0,
            &mut child_image,
        )
    };
    if status != EFI_SUCCESS {
        return status
    }

    let mut exit_data_size = 0;
    let mut exit_data = core::ptr::null_mut();
    let status = unsafe {
        ((*services).start_image)(child_image, &mut exit_data_size, &mut exit_data)
    };
    unsafe {
        ((*services).unload_image)(child_image);
    }
    status
}

unsafe fn build_device_path(
    device_path: *const DevicePathHeader,
    file_path: &[u16],
) -> Option<*const DevicePathHeader> {
    let buffer = (&raw mut DEVICE_PATH_BUFFER).cast::<u8>();
    let mut source = device_path.cast::<u8>();
    let mut used = 0usize;

    loop {
        let header = unsafe { &*source.cast::<DevicePathHeader>() };
        let length = u16::from_le_bytes(header.length) as usize;
        if length < size_of::<DevicePathHeader>() {
            return None
        }
        if header.device_type == END_DEVICE_PATH_TYPE {
            break
        }
        if used.checked_add(length)? > DEVICE_PATH_BUFFER_SIZE {
            return None
        }
        unsafe {
            core::ptr::copy_nonoverlapping(source, buffer.add(used), length);
            source = source.add(length);
        }
        used += length;
    }

    let path_bytes = file_path.len().checked_add(1)?.checked_mul(2)?;
    let node_length = size_of::<DevicePathHeader>().checked_add(path_bytes)?;
    let total_length = used
        .checked_add(node_length)?
        .checked_add(size_of::<DevicePathHeader>())?;
    if node_length > u16::MAX as usize || total_length > DEVICE_PATH_BUFFER_SIZE {
        return None
    }

    let path_header = DevicePathHeader {
        device_type: MEDIA_DEVICE_PATH,
        sub_type: MEDIA_FILEPATH_DP,
        length: (node_length as u16).to_le_bytes(),
    };
    unsafe {
        buffer.add(used).cast::<DevicePathHeader>().write_unaligned(path_header);
    }
    used += size_of::<DevicePathHeader>();

    for code_unit in file_path {
        unsafe {
            buffer.cast::<u8>().add(used).cast::<u16>().write_unaligned(*code_unit);
        }
        used += 2;
    }
    unsafe {
        buffer.add(used).cast::<u16>().write_unaligned(0);
    }
    used += 2;

    let end = DevicePathHeader {
        device_type: END_DEVICE_PATH_TYPE,
        sub_type: END_ENTIRE_DEVICE_PATH_SUBTYPE,
        length: (size_of::<DevicePathHeader>() as u16).to_le_bytes(),
    };
    unsafe {
        buffer.add(used).cast::<DevicePathHeader>().write_unaligned(end);
    }

    Some(buffer.cast::<DevicePathHeader>())
}
