#define WIN32_LEAN_AND_MEAN
#include "linker_hook.hpp"
#include "disasm.hpp"
#include "logs.hpp"
#include "mod_loader.hpp"
#include <cstring>
#include <psapi.h>
#include <string>
#include <windows.h>

TrampolineHook Hook_StaticFindObjectFast{};

void *__fastcall Hooked_StaticFindObjectFast(void *cls, void *outer, FName name,
                                             int bExact, int bAny,
                                             uint64_t excl) {
	log_info(
	    "SFOF: Index=%d Number=%d cls=%p outer=%p bExact=%d bAny=%d excl=%llu",
	    name.Index, name.Number, cls, outer, bExact, bAny,
	    static_cast<unsigned long long>(excl));

	void *repl = mod_loader::find_replacement(name);
	if (repl) {
		log_info("SFOF: -> mod replacement %p", repl);
		return repl;
	}

	void *result = reinterpret_cast<StaticFindObjectFast_fn>(
	    Hook_StaticFindObjectFast.trampoline)(cls, outer, name, bExact, bAny,
	                                          excl);

	log_info("SFOF: -> original result %p", result);
	return result;
}

namespace linker_hook {
namespace {

static FindPackageFile_fn g_orig_fpf = nullptr;
static void **g_fpf_slot = nullptr;

static constexpr int kFPFSlot = 3;

static int __fastcall hook_fpf(void *self, const wchar_t *name, void *guid,
                               FStringLayout *out_fstr,
                               const wchar_t *language) {
	std::wstring mod_path = mod_loader::find_mod_pkg_path(name);
	if (!mod_path.empty()) {
		static wchar_t s_buf[512];
		wcsncpy_s(s_buf, mod_path.c_str(), 511);
		out_fstr->Data = s_buf;
		out_fstr->Num = static_cast<int32_t>(wcslen(s_buf) + 1);
		out_fstr->Max = 0;
		log_info("linker_hook: FindPackageFile '%ls' -> mod path", name);
		return 1;
	}
	return g_orig_fpf(self, name, guid, out_fstr, language);
}

static bool vtable_write(void **slot, void *fn) {
	DWORD old{};
	if (VirtualProtect(slot, sizeof(void *), PAGE_READWRITE, &old)) {
		*slot = fn;
		VirtualProtect(slot, sizeof(void *), old, &old);
		return true;
	}
	SIZE_T written = 0;
	WriteProcessMemory(GetCurrentProcess(), slot, &fn, sizeof(fn), &written);
	return written == sizeof(fn);
}

} // namespace

void install_vtable() {
	void **cache = ue3().GPackageFileCache;
	if (!cache || !*cache) {
		log_warn(
		    "linker_hook: GPackageFileCache not ready — vtable hook skipped");
		return;
	}

	auto **vtbl = *reinterpret_cast<void ***>(*cache);
	if (!vtbl) {
		log_err("linker_hook: null vtable on GPackageFileCache object");
		return;
	}

	HMODULE exe = GetModuleHandleW(nullptr);
	MODULEINFO mi{};
	GetModuleInformation(GetCurrentProcess(), exe, &mi, sizeof(mi));

	constexpr int kFallback = kFPFSlot; // = 3
	const int slot = find_fpf_vtable_slot(
	    vtbl, 16, reinterpret_cast<uintptr_t>(mi.lpBaseOfDll),
	    static_cast<size_t>(mi.SizeOfImage), kFallback);

	g_fpf_slot = &vtbl[slot];
	g_orig_fpf = reinterpret_cast<FindPackageFile_fn>(*g_fpf_slot);

	if (!vtable_write(g_fpf_slot, reinterpret_cast<void *>(&hook_fpf))) {
		log_err("linker_hook: vtable write failed for slot %d", slot);
		g_fpf_slot = nullptr;
		g_orig_fpf = nullptr;
		return;
	}

	log_info("linker_hook: FindPackageFile vtable hooked (slot=%d orig=%p)",
	         slot, reinterpret_cast<void *>(g_orig_fpf));
}

void remove_vtable() {
	if (!g_fpf_slot || !g_orig_fpf)
		return;
	vtable_write(g_fpf_slot, reinterpret_cast<void *>(g_orig_fpf));
	g_fpf_slot = nullptr;
	g_orig_fpf = nullptr;
	log_info("linker_hook: FindPackageFile vtable hook removed");
}

} // namespace linker_hook
