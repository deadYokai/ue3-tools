#define WIN32_LEAN_AND_MEAN
#include "mod_loader.hpp"
#include "logs.hpp"
#include "mod_toml_mini.hpp"
#include "util.hpp"
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <windows.h>

namespace fs = std::filesystem;
namespace {

std::once_flag g_once;
std::vector<ObjectReplacement> g_replacements;
std::mutex g_load_mtx;

static void *resolve_ue3_class(const std::string &class_name) {
	if (class_name.empty() || !ue3().StaticFindObjectFast)
		return nullptr;

	std::wstring wname = to_wide(class_name);
	FName cls_name = fname_find(wname.c_str());
	if (cls_name.Index == 0) {
		log_warn("mod_loader: FName not found for class '%s'",
		         class_name.c_str());
		return nullptr;
	}

	void *cls = ue3().StaticFindObjectFast(nullptr, nullptr, cls_name, 0, 0, 0);
	if (!cls)
		log_warn("mod_loader: UClass not found for '%s'", class_name.c_str());
	return cls;
}

static void load_mod_dir(const fs::path &dir) {
	auto toml = dir / "mod.toml";
	if (!fs::exists(toml))
		return;

	std::ifstream f(toml);
	if (!f) {
		log_warn("mod_loader: cannot open %s", toml.string().c_str());
		return;
	}
	std::string text((std::istreambuf_iterator<char>(f)), {});

	auto entries = mod_toml_mini::parse(text, dir.string());
	for (auto &r : entries) {
		log_info("mod_loader: entry orig='%s' repl='%ls' file='%ls' class='%s'",
		         r.orig_obj.c_str(), r.repl_path_w.c_str(), r.mod_upk_w.c_str(),
		         r.ue3_class.c_str());
		g_replacements.push_back(std::move(r));
	}

	log_info("mod_loader: loaded mod dir '%s' (%zu replacements)",
	         dir.filename().string().c_str(), entries.size());
}

static void do_load() {
	std::wstring exe_dir = get_exe_dir();
	if (exe_dir.empty()) {
		log_err("mod_loader: get_exe_dir failed");
		return;
	}

	fs::path mods = fs::path(exe_dir) / ".." / ".." / L"Mods";
	if (!fs::exists(mods)) {
		log_info("mod_loader: no Mods directory at '%s'",
		         mods.string().c_str());
		return;
	}
	log_info("mod_loader: scanning '%s'", mods.string().c_str());

	try {
		for (const auto &e : fs::directory_iterator(mods))
			if (e.is_directory())
				load_mod_dir(e.path());
	} catch (const std::exception &ex) {
		log_err("mod_loader: scan error: %s", ex.what());
	}

	log_info("mod_loader: ready — %zu object replacements",
	         g_replacements.size());
}

static void *load_one(ObjectReplacement &r) {
	auto *slo = ue3().StaticLoadObject;
	if (!slo)
		return nullptr;

	void *cls = resolve_ue3_class(r.ue3_class);
	const wchar_t *fp = r.mod_upk_w.empty() ? nullptr : r.mod_upk_w.c_str();

	log_info("mod_loader: StaticLoadObject cls=%p path='%ls' file='%ls'", cls,
	         r.repl_path_w.c_str(), fp ? fp : L"(null)");

	return slo(cls, nullptr, r.repl_path_w.c_str(), fp, 0, nullptr, 1);
}

} // namespace

namespace mod_loader {

void ensure_loaded() { std::call_once(g_once, do_load); }

void preload_objects() {
	if (!ue3().StaticLoadObject) {
		log_err("mod_loader: StaticLoadObject not resolved");
		return;
	}

	for (auto &r : g_replacements) {
		if (r.cached_obj)
			continue;
		r.cached_obj = load_one(r);
		if (r.cached_obj)
			log_info("mod_loader: preloaded '%ls' -> %p", r.repl_path_w.c_str(),
			         r.cached_obj);
		else
			log_err("mod_loader: StaticLoadObject failed for '%ls'",
			        r.repl_path_w.c_str());
	}
}

void *find_replacement(FName name) {
	if (name.Index == 0 || g_replacements.empty())
		return nullptr;

	for (auto &r : g_replacements) {
		if (r.orig_name.Index == 0) {
			std::wstring wn = to_wide(r.orig_obj);
			r.orig_name = fname_find(wn.c_str());
			if (r.orig_name.Index == 0)
				continue;
		}
		if (r.orig_name.Index != name.Index)
			continue;

		if (!r.cached_obj) {
			std::lock_guard<std::mutex> lk(g_load_mtx);
			if (r.cached_obj)
				return r.cached_obj;
			r.cached_obj = load_one(r);
			if (r.cached_obj)
				log_info("mod_loader: lazy-loaded '%ls' -> %p",
				         r.repl_path_w.c_str(), r.cached_obj);
			else
				log_err("mod_loader: lazy StaticLoadObject failed for '%ls'",
				        r.repl_path_w.c_str());
		}
		return r.cached_obj;
	}
	return nullptr;
}

std::wstring find_mod_pkg_path(const wchar_t *pkg_name) {
	std::string stem;
	for (const wchar_t *p = pkg_name; *p; ++p)
		stem += static_cast<char>(towlower(*p));

	for (const auto &r : g_replacements) {
		if (r.mod_upk_w.empty())
			continue;
		std::string s = fs::path(r.mod_upk_w).stem().string();
		for (auto &c : s)
			c = static_cast<char>(tolower(static_cast<unsigned char>(c)));
		if (s == stem)
			return r.mod_upk_w;
	}
	return {};
}

} // namespace mod_loader
