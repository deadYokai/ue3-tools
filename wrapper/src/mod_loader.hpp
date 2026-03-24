#pragma once
#include "ue3_types.hpp"
#include <string>
#include <vector>

struct ObjectReplacement {
	std::string orig_obj;
	std::string orig_full_path;
	std::wstring repl_path_w;
	std::wstring mod_upk_w;
	std::string ue3_class;

	FName orig_name{};
	void *cached_cls{};
	void *cached_obj{};

	bool cls_resolved{};
	bool orig_found{};
	bool orig_warned{};
	bool slo_failed{};
};

namespace mod_loader {
void ensure_loaded();
void *find_replacement(FName name, void *outer, void *hook_cls);
std::wstring find_mod_pkg_path(const wchar_t *pkg_name);
} // namespace mod_loader
