#pragma once
#include "ue3_types.hpp"
#include <string>
#include <vector>

struct ObjectReplacement {
	std::string orig_obj;
	std::wstring repl_path_w;
	std::wstring mod_upk_w;
	FName orig_name{};
	void *cached_obj{};
};

struct ModData {
	std::vector<ObjectReplacement> replacements;
	bool empty() const { return replacements.empty(); }
};

namespace mod_loader {
void ensure_loaded();
void preload_objects();
void *find_replacement(FName name);
std::wstring find_mod_pkg_path(const wchar_t *pkg_name);
} // namespace mod_loader
