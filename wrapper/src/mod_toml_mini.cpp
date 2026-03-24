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

std::string to_lower(std::string s) {
	for (auto &c : s)
		c = static_cast<char>(tolower(static_cast<unsigned char>(c)));
	return s;
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

		auto orig_dot = cur_orig.rfind('.');
		std::string obj_name = (orig_dot != std::string::npos)
		                           ? cur_orig.substr(orig_dot + 1)
		                           : cur_orig;

		std::string repl_full = cur_repl;
		auto repl_dot = cur_repl.rfind('.');
		if (repl_dot == std::string::npos) {
			repl_full = cur_repl + "." + cur_repl;
		}

		auto pkg_dot = repl_full.rfind('.');
		std::string repl_pkg = repl_full.substr(0, pkg_dot);
		std::string repl_pkg_lo = to_lower(repl_pkg);

		std::wstring file_path;
		std::string ue3_class;
		try {
			for (const auto &e : fs::directory_iterator(mod_dir)) {
				if (!e.is_regular_file())
					continue;
				std::string stem = to_lower(e.path().stem().string());
				if (stem != repl_pkg_lo)
					continue;

				std::string ext = e.path().extension().string();
				file_path = e.path().wstring();
				if (ext.size() > 1 && ext[0] == '.' && ext != ".upk")
					ue3_class = ext.substr(1);
				break;
			}
		} catch (...) {
		}

		ObjectReplacement r;
		r.orig_obj = obj_name;
		r.orig_full_path = cur_orig;
		r.repl_path_w = to_wide(repl_full);
		r.mod_upk_w = file_path;
		r.ue3_class = ue3_class;
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
