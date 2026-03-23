#include "mod_toml_mini.hpp"
#include "util.hpp"
#include <cctype>
#include <filesystem>
#include <sstream>
#include <vector>

namespace fs = std::filesystem;
namespace mod_toml_mini {

namespace {

std::string trim(const std::string &s) {
	size_t a = s.find_first_not_of(" \t\r");
	size_t b = s.find_last_not_of(" \t\r");
	return (a == std::string::npos) ? "" : s.substr(a, b - a + 1);
}

std::string strip_quotes(const std::string &s) {
	if (s.size() >= 2 && ((s.front() == '"' && s.back() == '"') ||
	                      (s.front() == '\'' && s.back() == '\'')))
		return s.substr(1, s.size() - 2);
	return s;
}

std::pair<std::string, std::string> split_kv(const std::string &line) {
	auto eq = line.find('=');
	if (eq == std::string::npos)
		return {};
	return {trim(line.substr(0, eq)), strip_quotes(trim(line.substr(eq + 1)))};
}

} // namespace

std::vector<ObjectReplacement> parse(const std::string &text,
                                     const std::string &mod_dir_str) {
	std::vector<ObjectReplacement> out;
	fs::path mod_dir(mod_dir_str);

	enum class Section { None, Replace } sec = Section::None;
	std::string cur_orig;
	std::string cur_repl;

	auto flush = [&]() {
		if (cur_orig.empty() || cur_repl.empty()) {
			cur_orig.clear();
			cur_repl.clear();
			return;
		}

		auto dot = cur_orig.rfind('.');
		std::string obj_name =
		    (dot != std::string::npos) ? cur_orig.substr(dot + 1) : cur_orig;

		auto rdot = cur_repl.rfind('.');
		std::string repl_pkg =
		    (rdot != std::string::npos) ? cur_repl.substr(0, rdot) : cur_repl;
		std::string repl_pkg_lo = repl_pkg;
		for (auto &c : repl_pkg_lo)
			c = static_cast<char>(tolower(static_cast<unsigned char>(c)));

		std::wstring upk_path;
		try {
			for (const auto &e : fs::directory_iterator(mod_dir)) {
				if (!e.is_regular_file())
					continue;
				std::string ext = e.path().extension().string();
				std::string stem = e.path().stem().string();
				for (auto &c : ext)
					c = static_cast<char>(
					    tolower(static_cast<unsigned char>(c)));
				for (auto &c : stem)
					c = static_cast<char>(
					    tolower(static_cast<unsigned char>(c)));
				if (ext == ".upk" && stem == repl_pkg_lo) {
					upk_path = e.path().wstring();
					break;
				}
			}
		} catch (...) {
		}

		ObjectReplacement r;
		r.orig_obj = obj_name;
		r.repl_path_w = to_wide(cur_repl);
		r.mod_upk_w = upk_path;
		out.push_back(std::move(r));
		cur_orig.clear();
		cur_repl.clear();
	};

	std::istringstream ss(text);
	std::string raw;
	while (std::getline(ss, raw)) {
		std::string line = trim(raw);
		auto hash = line.find('#');
		if (hash != std::string::npos)
			line = trim(line.substr(0, hash));
		if (line.empty())
			continue;

		if (line == "[[patch]]" || line == "[patch.name_remap]") {
			sec = Section::None;
			continue;
		}
		if (line == "[[patch.replace]]") {
			flush();
			sec = Section::Replace;
			continue;
		}

		auto [k, v] = split_kv(line);
		if (k.empty())
			continue;

		if (sec == Section::Replace) {
			if (k == "original")
				cur_orig = v;
			else if (k == "replacement")
				cur_repl = v;
		}
	}
	flush();
	return out;
}

} // namespace mod_toml_mini
