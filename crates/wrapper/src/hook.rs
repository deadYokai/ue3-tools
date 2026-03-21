use windows_sys::Win32::{
    Foundation::HANDLE,
    Security::SECURITY_ATTRIBUTES,              // correct module
    Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_READ, FILE_SHARE_READ,
        OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL,
    },
    System::Memory::{VirtualProtect, PAGE_READWRITE},
    System::LibraryLoader::GetModuleHandleW,
};

use crate::{ORIG_CREATE_FILE_W, overlay, pcwstr_to_string};

pub(crate) fn install_create_file_hook() -> Result<(), &'static str> {
    unsafe {
        let base = GetModuleHandleW(core::ptr::null()) as *const u8;
        if base.is_null() { return Err("GetModuleHandleW failed"); }
        patch_iat(base, b"CreateFileW\0").ok_or("CreateFileW not found in IAT")
    }
}

pub(crate) fn remove_create_file_hook() {}

unsafe fn patch_iat(base: *const u8, target_fn: &[u8]) -> Option<()> {
    let e_lfanew = read_u32(base, 0x3C) as usize;
    if read_u32(base, e_lfanew) != 0x00004550 { return None; } // "PE\0\0"

    let opt_off = e_lfanew + 24;
    let dir_off = match read_u16(base, opt_off) {
        0x10B => opt_off + 96,
        0x20B => opt_off + 112,
        _     => return None,
    };

    let import_rva = read_u32(base, dir_off) as usize;
    if import_rva == 0 { return None; }

    let mut desc = import_rva;
    loop {
        let orig_first  = read_u32(base, desc)      as usize;
        let name_rva    = read_u32(base, desc + 12) as usize;
        let first_thunk = read_u32(base, desc + 16) as usize;
        if name_rva == 0 && first_thunk == 0 { break; }

        let thunk_rva = if orig_first != 0 { orig_first } else { first_thunk };
        let mut slot = 0usize;
        loop {
            let thunk_ptr = base.add(thunk_rva + slot * 8) as *const u64;
            let thunk_val = thunk_ptr.read_unaligned();
            if thunk_val == 0 { break; }

            if thunk_val & 0x8000_0000_0000_0000 == 0 {
                let ibn_rva = (thunk_val & 0x7FFF_FFFF_FFFF_FFFF) as usize;
                let fn_name = base.add(ibn_rva + 2) as *const u8;
                if cstr_eq(fn_name, target_fn) {
                    let iat_entry = base.add(first_thunk + slot * 8) as *mut u64;
                    let _ = ORIG_CREATE_FILE_W.set(iat_entry.read_unaligned() as usize);
                    let mut old = 0u32;
                    VirtualProtect(iat_entry as _, 8, PAGE_READWRITE, &mut old);
                    iat_entry.write_unaligned(hooked_create_file_w as u64);
                    VirtualProtect(iat_entry as _, 8, old, &mut old);
                    return Some(());
                }
            }
            slot += 1;
        }
        desc += 20;
    }
    None
}

unsafe extern "system" fn hooked_create_file_w(
    lp_file_name:            *const u16,
    dw_desired_access:       u32,
    dw_share_mode:           u32,
    lp_security_attributes:  *const SECURITY_ATTRIBUTES,
    dw_creation_disposition: u32,
    dw_flags_and_attributes: u32,
    h_template_file:         HANDLE,
) -> HANDLE {
    'patch: {
        let Some(path) = pcwstr_to_string(lp_file_name) else { break 'patch };
        if !path.to_ascii_lowercase().ends_with(".upk") { break 'patch }
        if dw_desired_access & FILE_GENERIC_READ == 0   { break 'patch }
        let Some(patched) = overlay::get_patched_path(&path) else { break 'patch };
        let wide: Vec<u16> = patched.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        return real_create_file_w(
            wide.as_ptr(),
            dw_desired_access, dw_share_mode,
            lp_security_attributes, dw_creation_disposition,
            dw_flags_and_attributes, h_template_file,
        );
    }
    real_create_file_w(
        lp_file_name, dw_desired_access, dw_share_mode,
        lp_security_attributes, dw_creation_disposition,
        dw_flags_and_attributes, h_template_file,
    )
}

pub(crate) unsafe fn real_create_file_w(
    lp_file_name:            *const u16,
    dw_desired_access:       u32,
    dw_share_mode:           u32,
    lp_security_attributes:  *const SECURITY_ATTRIBUTES,
    dw_creation_disposition: u32,
    dw_flags_and_attributes: u32,
    h_template_file:         HANDLE,
) -> HANDLE {
    let fn_ptr = ORIG_CREATE_FILE_W.get().copied().unwrap_or(CreateFileW as usize);
    type Fn = unsafe extern "system" fn(
        *const u16, u32, u32, *const SECURITY_ATTRIBUTES, u32, u32, HANDLE,
    ) -> HANDLE;
    let f: Fn = core::mem::transmute(fn_ptr);
    f(lp_file_name, dw_desired_access, dw_share_mode,
      lp_security_attributes, dw_creation_disposition,
      dw_flags_and_attributes, h_template_file)
}

#[inline(always)] unsafe fn read_u32(base: *const u8, off: usize) -> u32 {
    (base.add(off) as *const u32).read_unaligned()
}
#[inline(always)] unsafe fn read_u16(base: *const u8, off: usize) -> u16 {
    (base.add(off) as *const u16).read_unaligned()
}
#[inline(always)] unsafe fn cstr_eq(ptr: *const u8, needle: &[u8]) -> bool {
    core::slice::from_raw_parts(ptr, needle.len()) == needle
}
