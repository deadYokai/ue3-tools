#define WIN32_LEAN_AND_MEAN
#include "hook.hpp"
#include "log.hpp"
#include "util.hpp"
#include <windows.h>

static volatile FARPROC g_real_di8 = nullptr;

static void load_real_dinput8() {
  log_info("loading real dinput8.dll...");
  for (const char *dep :
       {"ole32.dll", "oleaut32.dll", "user32.dll", "advapi32.dll", "hid.dll"}) {
    HMODULE h = LoadLibraryA(dep);
    log_info("  preload %-16s -> %p", dep, (void *)h);
  }
  HMODULE hmod = LoadLibraryA("C:\\Windows\\System32\\dinput8.dll");
  if (!hmod) {
    log_err("LoadLibraryA(System32 dinput8.dll) failed");
    return;
  }
  log_info("real dinput8.dll at %p", (void *)hmod);

  FARPROC fn = GetProcAddress(hmod, "DirectInput8Create");
  if (fn) {
    g_real_di8 = fn;
    log_info("DirectInput8Create -> %p", (void *)fn);
  } else
    log_err("GetProcAddress(DirectInput8Create) failed");
}

static DWORD WINAPI init_thread(LPVOID) {
  std::wstring dir = get_exe_dir();
  if (!dir.empty())
    log::init(dir);

  log_info("dinput8_wrapper init thread started");
  load_real_dinput8();
  log_info("init complete");
  return 0;
}

extern "C" BOOL WINAPI DllMain(HMODULE hmod, DWORD reason, LPVOID) {
  switch (reason) {
  case DLL_PROCESS_ATTACH:
    DisableThreadLibraryCalls(hmod);
    hook::install_create_file_hook();
    CreateThread(nullptr, 0, init_thread, nullptr, 0, nullptr);
    break;
  case DLL_PROCESS_DETACH:
    log_info("DLL_PROCESS_DETACH");
    hook::remove_create_file_hook();
    break;
  }
  return TRUE;
}

struct GUID_t {
  UINT32 data1;
  UINT16 data2, data3;
  UINT8 data4[8];
};

typedef HRESULT(WINAPI *DI8Fn)(HINSTANCE, DWORD, const GUID_t *, void **,
                               IUnknown *);

extern "C" HRESULT WINAPI DirectInput8Create(HINSTANCE hinst, DWORD ver,
                                             const GUID_t *riid, void **ppv,
                                             IUnknown *outer) {
  for (DWORD retry = 0;; ++retry) {
    if (g_real_di8)
      break;
    if (retry >= 200) {
      log_err("DirectInput8Create: real fn not ready, returning error");
      return (HRESULT)0x8007007Eu;
    }
    Sleep(10);
  }
  log_info("DirectInput8Create forwarded (ver=0x%08lx)", (unsigned long)ver);
  auto fn = reinterpret_cast<DI8Fn>(g_real_di8);
  HRESULT hr = fn(hinst, ver, riid, ppv, outer);
  if (SUCCEEDED(hr))
    log_info("DirectInput8Create OK  hr=0x%08lx", (unsigned long)hr);
  else
    log_err("DirectInput8Create FAIL hr=0x%08lx", (unsigned long)hr);
  return hr;
}
