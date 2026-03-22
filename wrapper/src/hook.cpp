#define WIN32_LEAN_AND_MEAN
#include "hook.hpp"
#include "log.hpp"
#include "overlay.hpp"
#include "util.hpp"
#include <cstring>
#include <string>
#include <windows.h>

static volatile FARPROC g_orig_cfW = nullptr;

static HANDLE WINAPI hooked_create_file_w(LPCWSTR, DWORD, DWORD,
                                          LPSECURITY_ATTRIBUTES, DWORD, DWORD,
                                          HANDLE);

static inline uint32_t ru32(const uint8_t *b, size_t o) {
  uint32_t v;
  memcpy(&v, b + o, 4);
  return v;
}
static inline uint16_t ru16(const uint8_t *b, size_t o) {
  uint16_t v;
  memcpy(&v, b + o, 2);
  return v;
}

static bool patch_iat(const uint8_t *base) {
  size_t e_lfanew = ru32(base, 0x3C);
  if (ru32(base, e_lfanew) != 0x00004550)
    return false; // "PE\0\0"

  size_t opt_off = e_lfanew + 24;
  size_t dir_off;
  switch (ru16(base, opt_off)) {
  case 0x10B:
    dir_off = opt_off + 96;
    break;
  case 0x20B:
    dir_off = opt_off + 112;
    break;
  default:
    return false;
  }

  size_t import_rva = ru32(base, dir_off);
  if (!import_rva)
    return false;

  static const char target[] = "CreateFileW";
  static const size_t tlen = sizeof(target);

  for (size_t desc = import_rva;; desc += 20) {
    uint32_t orig_first = ru32(base, desc);
    uint32_t name_rva = ru32(base, desc + 12);
    uint32_t first_thunk = ru32(base, desc + 16);
    if (!name_rva && !first_thunk)
      break;

    size_t thunk_rva = orig_first ? orig_first : first_thunk;
    for (size_t slot = 0;; ++slot) {
      uint64_t tv;
      memcpy(&tv, base + thunk_rva + slot * 8, 8);
      if (!tv)
        break;
      if (!(tv & 0x8000000000000000ULL)) {
        size_t ibn_rva = (size_t)(tv & 0x7FFFFFFFFFFFFFFFULL);
        const char *fn = reinterpret_cast<const char *>(base + ibn_rva + 2);
        if (memcmp(fn, target, tlen) == 0) {
          auto *iat = reinterpret_cast<uint64_t *>(const_cast<uint8_t *>(base) +
                                                   first_thunk + slot * 8);
          g_orig_cfW = reinterpret_cast<FARPROC>((uintptr_t)*iat);
          DWORD old;
          VirtualProtect(iat, 8, PAGE_READWRITE, &old);
          *iat = (uint64_t)(uintptr_t)&hooked_create_file_w;
          VirtualProtect(iat, 8, old, &old);
          return true;
        }
      }
    }
  }
  return false;
}

static HANDLE WINAPI hooked_create_file_w(LPCWSTR lp, DWORD access, DWORD share,
                                          LPSECURITY_ATTRIBUTES sa, DWORD cd,
                                          DWORD fa, HANDLE htf) {
  if (lp && (access & GENERIC_READ)) {
    std::wstring wpath(lp);
    if (wpath.size() >= 4) {
      std::wstring ext = wpath.substr(wpath.size() - 4);
      for (auto &c : ext)
        c = (wchar_t)towlower(c);
      if (ext == L".upk") {
        std::string narrow = to_narrow(wpath);
        std::string patched = overlay::get_patched_path(narrow);
        if (!patched.empty())
          return hook::real_create_file_w(to_wide(patched).c_str(), access,
                                          share, sa, cd, fa, htf);
      }
    }
  }
  return hook::real_create_file_w(lp, access, share, sa, cd, fa, htf);
}

namespace hook {

void install_create_file_hook() {
  const uint8_t *base =
      reinterpret_cast<const uint8_t *>(GetModuleHandleW(nullptr));
  if (!base)
    return;
  if (!patch_iat(base))
    log_warn("install_create_file_hook: CreateFileW not found in IAT");
}

void remove_create_file_hook() {}

HANDLE WINAPI real_create_file_w(LPCWSTR lp, DWORD access, DWORD share,
                                 LPSECURITY_ATTRIBUTES sa, DWORD cd, DWORD fa,
                                 HANDLE htf) {
  typedef HANDLE(WINAPI * Fn)(LPCWSTR, DWORD, DWORD, LPSECURITY_ATTRIBUTES,
                              DWORD, DWORD, HANDLE);
  Fn fn = g_orig_cfW ? reinterpret_cast<Fn>(g_orig_cfW) : CreateFileW;
  return fn(lp, access, share, sa, cd, fa, htf);
}

} // namespace hook
