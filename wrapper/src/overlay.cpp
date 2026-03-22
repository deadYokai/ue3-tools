#define WIN32_LEAN_AND_MEAN
#include "overlay.hpp"
#include "log.hpp"
#include "mod_toml_mini.hpp"
#include "patch_fmt.hpp"
#include "upk.hpp"
#include "util.hpp"
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <unordered_map>
#include <windows.h>

namespace fs = std::filesystem;
using PatchMap = std::unordered_map<std::string, std::vector<CdoPatch>>;

static uint64_t fnv1a(const uint8_t *d, size_t n) {
  uint64_t h = 0xcbf29ce484222325ULL;
  for (size_t i = 0; i < n; ++i) {
    h ^= d[i];
    h *= 0x100000001B3ULL;
  }
  return h;
}
static std::string hex64(uint64_t v) {
  char buf[17];
  snprintf(buf, sizeof(buf), "%016llx", (unsigned long long)v);
  return buf;
}

static std::once_flag s_once;
static PatchMap s_patch_map;
static std::mutex s_cache_mtx;
static std::unordered_map<std::string, std::string> s_cache;

static PatchMap load_mods_dir() {
  PatchMap map;
  std::wstring wdir = get_exe_dir();
  if (wdir.empty())
    return map;
  fs::path mods_dir = fs::path(wdir) / L"Mods";
  try {
    for (const auto &entry : fs::directory_iterator(mods_dir)) {
      const auto &p = entry.path();
      if (entry.is_directory()) {
        auto toml = p / "mod.toml";
        if (!fs::exists(toml))
          continue;
        std::ifstream f(toml);
        if (!f)
          continue;
        std::string text((std::istreambuf_iterator<char>(f)), {});
        auto patches = mod_toml_mini::parse(text, p.string());
        for (auto &[pkg, cdo] : patches) {
          auto &dst = map[pkg];
          dst.insert(dst.end(), cdo.begin(), cdo.end());
        }
      } else {
        std::string fname = p.filename().string();
        const std::string pfx = "ScriptPatch_";
        if (fname.compare(0, pfx.size(), pfx) != 0)
          continue;
        std::string rest = fname.substr(pfx.size());
        if (rest.size() < 4 || rest.substr(rest.size() - 4) != ".bin")
          continue;
        std::string pkg = rest.substr(0, rest.size() - 4);
        if (pkg.empty())
          continue;
        std::ifstream f(p, std::ios::binary);
        if (!f)
          continue;
        std::vector<uint8_t> data((std::istreambuf_iterator<char>(f)), {});
        try {
          auto ps = load_patch_bin(data.data(), data.size());
          std::string key = pkg;
          for (auto &c : key)
            c = (char)tolower((unsigned char)c);
          auto &dst = map[key];
          dst.insert(dst.end(), ps.begin(), ps.end());
        } catch (...) {
        }
      }
    }
  } catch (...) {
  }
  return map;
}

static void ensure_loaded() {
  std::call_once(s_once, []() { s_patch_map = load_mods_dir(); });
}

namespace overlay {

std::string get_patched_path(const std::string &original_path) {
  fs::path p(original_path);
  std::string stem = p.stem().string();
  for (auto &c : stem)
    c = (char)tolower((unsigned char)c);

  ensure_loaded();

  auto it = s_patch_map.find(stem);
  if (it == s_patch_map.end() || it->second.empty())
    return {};

  std::string key = original_path;
  for (auto &c : key)
    c = (char)tolower((unsigned char)c);

  {
    std::lock_guard<std::mutex> lk(s_cache_mtx);
    auto cit = s_cache.find(key);
    if (cit != s_cache.end() && fs::exists(cit->second))
      return cit->second;
  }

  std::ifstream f(original_path, std::ios::binary);
  if (!f)
    return {};
  std::vector<uint8_t> raw((std::istreambuf_iterator<char>(f)), {});

  std::vector<uint8_t> patched;
  try {
    patched = apply_cdo_patches(raw.data(), raw.size(), it->second);
  } catch (const std::exception &ex) {
    log_err("apply_cdo_patches: %s", ex.what());
    return {};
  }

  auto tmp = fs::temp_directory_path() / "ue3mods";
  try {
    fs::create_directories(tmp);
  } catch (...) {
    return {};
  }

  auto out_path =
      tmp / (stem + "_" + hex64(fnv1a(raw.data(), raw.size())) + ".upk");
  {
    std::ofstream of(out_path, std::ios::binary);
    if (!of)
      return {};
    of.write(reinterpret_cast<const char *>(patched.data()), patched.size());
  }

  std::string result = out_path.string();
  {
    std::lock_guard<std::mutex> lk(s_cache_mtx);
    s_cache[key] = result;
  }
  return result;
}

} // namespace overlay
