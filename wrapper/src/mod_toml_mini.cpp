#include "mod_toml_mini.hpp"
#include <cctype>
#include <filesystem>
#include <fstream>
#include <sstream>

namespace fs = std::filesystem;

namespace mod_toml_mini {

static std::string trim(const std::string &s) {
  size_t a = s.find_first_not_of(" \t\r"), b = s.find_last_not_of(" \t\r");
  return (a == std::string::npos) ? "" : s.substr(a, b - a + 1);
}

static std::string strip_quotes(const std::string &s) {
  if (s.size() >= 2 && s.front() == '"' && s.back() == '"')
    return s.substr(1, s.size() - 2);
  return s;
}

static std::pair<std::string, std::string> split_kv(const std::string &line) {
  auto eq = line.find('=');
  if (eq == std::string::npos)
    return {};
  return {trim(line.substr(0, eq)), strip_quotes(trim(line.substr(eq + 1)))};
}

static fs::path find_blob(const fs::path &mod_dir, const std::string &subdir,
                          const std::string &stem) {
  fs::path dir = mod_dir / subdir;
  try {
    for (const auto &e : fs::directory_iterator(dir))
      if (e.path().stem().string() == stem)
        return e.path();
  } catch (...) {
  }
  return dir / stem;
}

std::unordered_map<std::string, std::vector<CdoPatch>>
parse(const std::string &text, const std::string &mod_dir_str) {
  std::unordered_map<std::string, std::vector<CdoPatch>> out;
  fs::path mod_dir(mod_dir_str);
  std::string cur_pkg, cur_dir, cur_orig;

  std::istringstream ss(text);
  std::string raw;
  while (std::getline(ss, raw)) {
    std::string line = trim(raw);
    if (line.empty() || line[0] == '#')
      continue;

    if (line == "[[patch]]") {
      cur_pkg.clear();
      cur_dir.clear();
      cur_orig.clear();
      continue;
    }
    if (line == "[[patch.replace]]") {
      cur_orig.clear();
      continue;
    }

    auto [k, v] = split_kv(line);
    if (k.empty())
      continue;

    if (k == "package")
      cur_pkg = v;
    else if (k == "dir")
      cur_dir = v;
    else if (k == "original")
      cur_orig = v;
    else if (k == "modfile") {
      if (cur_pkg.empty() || cur_dir.empty() || cur_orig.empty())
        continue;
      auto blob = find_blob(mod_dir, cur_dir, v);
      std::ifstream f(blob, std::ios::binary);
      if (!f)
        continue;
      std::vector<uint8_t> data((std::istreambuf_iterator<char>(f)), {});
      std::string key = cur_pkg;
      for (auto &c : key)
        c = (char)tolower((unsigned char)c);
      out[key].push_back(CdoPatch{cur_orig, std::move(data)});
    }
  }
  return out;
}

} // namespace mod_toml_mini
