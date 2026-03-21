#![cfg(target_os = "windows")]
#![allow(non_snake_case, clippy::missing_safety_doc)]

pub(crate) mod log;
mod hook;
mod mod_toml_mini;
mod overlay;
mod patch_fmt;
mod upk;

use std::{ffi::OsString, os::windows::ffi::OsStringExt, sync::OnceLock};

use windows_sys::Win32::{
    Foundation::{FALSE, HANDLE, HMODULE, TRUE},
    System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW, GetProcAddress, LoadLibraryExW},
};

static REAL_DI8_CREATE:   OnceLock<usize> = OnceLock::new();
pub(crate) static ORIG_CREATE_FILE_W: OnceLock<usize> = OnceLock::new();

const DLL_PROCESS_ATTACH: u32 = 1;
const DLL_PROCESS_DETACH: u32 = 0;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(
    _hinstance: HMODULE,
    reason:     u32,
    _reserved:  *mut core::ffi::c_void,
) -> i32 {
    match reason {
        DLL_PROCESS_ATTACH => {
            hook::install_create_file_hook();
        }
        DLL_PROCESS_DETACH => {
            hook::remove_create_file_hook();
        }
        _ => {}
    }
    TRUE
}

unsafe fn lazy_init() {
    if let Some(dir) = exe_dir() {
        log::init(&dir);
    }
    log_info!("lazy_init: starting");

    let real_path = wide("C:\\Windows\\System32\\dinput8.dll\0");
    let hmod = LoadLibraryExW(real_path.as_ptr(), core::ptr::null_mut(), 0);
    if hmod == core::ptr::null_mut() {
        log_err!("LoadLibraryExW(System32\\dinput8.dll) failed — DirectInput8Create will fail");
        return;
    }
    log_info!("real dinput8.dll loaded at {:p}", hmod);

    let name = b"DirectInput8Create\0";
    match GetProcAddress(hmod, name.as_ptr() as _) {
        Some(f) => {
            let _ = REAL_DI8_CREATE.set(f as usize);
            log_info!("DirectInput8Create resolved at {:#x}", f as usize);
        }
        None => log_err!("GetProcAddress(DirectInput8Create) returned null"),
    }
}

#[repr(C)]
pub struct GUID {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DirectInput8Create(
    hinst:      HMODULE,
    dw_version: u32,
    riid:       *const GUID,
    ppv_out:    *mut *mut core::ffi::c_void,
    punk_outer: *mut core::ffi::c_void,
) -> i32 {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| { unsafe { lazy_init(); } });

    let fn_ptr = match REAL_DI8_CREATE.get() {
        Some(p) => *p,
        None => {
            log_err!("DirectInput8Create: real fn not resolved, returning error");
            return 0x8007_007Eu32 as i32; // HRESULT: ERROR_MOD_NOT_FOUND
        }
    };

    log_info!("DirectInput8Create forwarded (version={:#x})", dw_version);
    type Fn = unsafe extern "system" fn(
        HMODULE, u32, *const GUID, *mut *mut core::ffi::c_void, *mut core::ffi::c_void,
    ) -> i32;
    let f: Fn = core::mem::transmute(fn_ptr);
    f(hinst, dw_version, riid, ppv_out, punk_outer)
}

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub(crate) unsafe fn pcwstr_to_string(ptr: *const u16) -> Option<String> {
    if ptr.is_null() { return None; }
    let mut len = 0usize;
    while *ptr.add(len) != 0 { len += 1; }
    let slice = core::slice::from_raw_parts(ptr, len);
    Some(OsString::from_wide(slice).to_string_lossy().into_owned())
}

pub(crate) unsafe fn exe_dir() -> Option<String> {
    let mut buf = vec![0u16; 32768];
    let len = GetModuleFileNameW(core::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32);
    if len == 0 { return None; }
    let path = OsString::from_wide(&buf[..len as usize])
        .to_string_lossy()
        .into_owned();
    path.rfind(['/', '\\']).map(|i| path[..i].to_owned())
}
