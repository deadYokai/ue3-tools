#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <unknwn.h>
#include "hook.hpp"
#include "log.hpp"
#include "util.hpp"

static HMODULE  g_sys_dinput8 = nullptr;
static FARPROC  g_real_di8    = nullptr;

static HMODULE load_system_dinput8() {
    char sys_dir[MAX_PATH]{};

#ifdef _WIN64
    GetSystemDirectoryA(sys_dir, MAX_PATH);
#else
    BOOL wow64 = FALSE;
    {
        using Fn = BOOL(WINAPI*)(HANDLE, PBOOL);
        auto fn = reinterpret_cast<Fn>(
            GetProcAddress(GetModuleHandleA("kernel32"), "IsWow64Process"));
        if (fn) fn(GetCurrentProcess(), &wow64);
    }
    if (wow64) {
        GetSystemWow64DirectoryA(sys_dir, MAX_PATH);
        if (sys_dir[0] == '\0') {
            GetWindowsDirectoryA(sys_dir, MAX_PATH);
            strcat_s(sys_dir, "\\SysWOW64");
        }
        log_info("arch: 32-bit on 64-bit Windows, using SysWOW64");
    } else {
        GetSystemDirectoryA(sys_dir, MAX_PATH);
        log_info("arch: native 32-bit Windows");
    }
#endif

    char dll_path[MAX_PATH]{};
    snprintf(dll_path, MAX_PATH, "%s\\dinput8.dll", sys_dir);
    log_info("loading real dinput8.dll from: %s", dll_path);

    HMODULE h = LoadLibraryA(dll_path);
    if (!h)
        log_err("LoadLibraryA failed (err=%lu)", GetLastError());
    else
        log_info("real dinput8.dll loaded at %p", (void*)h);
    return h;
}

static void init_dinput8() {
    log_info("pre-loading dinput8 dependencies...");
    for (const char* dep : {
            "ole32.dll","oleaut32.dll","user32.dll","advapi32.dll","hid.dll" }) {
        HMODULE h = LoadLibraryA(dep);
        log_info("  %-20s -> %p", dep, (void*)h);
    }

    g_sys_dinput8 = load_system_dinput8();
    if (!g_sys_dinput8) {
        log_err("FATAL: could not load system dinput8.dll");
        return;
    }

    g_real_di8 = GetProcAddress(g_sys_dinput8, "DirectInput8Create");
    if (g_real_di8)
        log_info("DirectInput8Create resolved at %p", (void*)g_real_di8);
    else {
        log_err("GetProcAddress(DirectInput8Create) failed (err=%lu)", GetLastError());
        FreeLibrary(g_sys_dinput8);
        g_sys_dinput8 = nullptr;
    }
}

static DWORD WINAPI init_thread(LPVOID) {
    log_info("init_thread: installing hooks");
    hook::install_create_file_hook();
    log_info("init_thread: done");
    return 0;
}

extern "C" BOOL WINAPI DllMain(HMODULE hmod, DWORD reason, LPVOID) {
    switch (reason) {
    case DLL_PROCESS_ATTACH:
        DisableThreadLibraryCalls(hmod);

        log::init(get_exe_dir());
        log_info("DLL_PROCESS_ATTACH");
#ifdef _WIN64
        log_info("build: 64-bit");
#else
        log_info("build: 32-bit");
#endif
        log_info("module handle: %p", (void*)hmod);

        init_dinput8();

        CreateThread(nullptr, 0, init_thread, nullptr, 0, nullptr);
        break;

    case DLL_PROCESS_DETACH:
        log_info("DLL_PROCESS_DETACH");
        hook::remove_create_file_hook();
        if (g_sys_dinput8) {
            FreeLibrary(g_sys_dinput8);
            g_sys_dinput8 = nullptr;
            g_real_di8    = nullptr;
        }
        break;
    }
    return TRUE;
}

struct GUID_t { UINT32 data1; UINT16 data2, data3; UINT8 data4[8]; };
typedef HRESULT (WINAPI *DI8Fn)(HINSTANCE, DWORD, const GUID_t*, void**, IUnknown*);

extern "C" HRESULT WINAPI DirectInput8Create(
    HINSTANCE hinst, DWORD ver, const GUID_t* riid, void** ppv, IUnknown* outer)
{
    for (DWORD retry = 0; !g_real_di8; ++retry) {
        if (retry >= 200) {
            log_err("DirectInput8Create: real fn not ready, giving up");
            return (HRESULT)0x8007007Eu;
        }
        Sleep(10);
    }
    log_info("DirectInput8Create forwarded (ver=0x%08lx)", (unsigned long)ver);
    HRESULT hr = reinterpret_cast<DI8Fn>(g_real_di8)(hinst, ver, riid, ppv, outer);
    if (SUCCEEDED(hr)) log_info("DirectInput8Create OK   hr=0x%08lx", (unsigned long)hr);
    else               log_err ("DirectInput8Create FAIL hr=0x%08lx", (unsigned long)hr);
    return hr;
}
