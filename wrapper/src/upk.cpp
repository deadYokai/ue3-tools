#define WIN32_LEAN_AND_MEAN
#include "upk.hpp"
#include <algorithm>
#include <cstring>
#include <stdexcept>
#include <unordered_map>
#include <windows.h>

static constexpr uint32_t PACKAGE_TAG = 0x9E2A83C1;

struct UPKCursor {
  const uint8_t *data;
  size_t size, pos = 0;

  uint32_t u32() {
    if (pos + 4 > size)
      throw std::runtime_error("UPK: EOF u32");
    uint32_t v;
    memcpy(&v, data + pos, 4);
    pos += 4;
    return v;
  }
  uint64_t u64() {
    if (pos + 8 > size)
      throw std::runtime_error("UPK: EOF u64");
    uint64_t v;
    memcpy(&v, data + pos, 8);
    pos += 8;
    return v;
  }
  int32_t i32() { return (int32_t)u32(); }
  int16_t i16() {
    if (pos + 2 > size)
      throw std::runtime_error("UPK: EOF i16");
    int16_t v;
    memcpy(&v, data + pos, 2);
    pos += 2;
    return v;
  }
  uint16_t u16() { return (uint16_t)i16(); }
  void seek(size_t p) {
    if (p > size)
      throw std::runtime_error("UPK: seek OOB");
    pos = p;
  }
  void skip(int64_t n) {
    int64_t np = (int64_t)pos + n;
    if (np < 0 || (size_t)np > size)
      throw std::runtime_error("UPK: skip OOB");
    pos = (size_t)np;
  }
};

struct Header {
  int16_t p_ver;
  int32_t name_count, name_offset;
  int32_t export_count, export_offset;
};

static Header read_header(const uint8_t *data, size_t size) {
  UPKCursor c{data, size};
  if (c.u32() != PACKAGE_TAG)
    throw std::runtime_error("UPK: bad magic");
  Header h;
  h.p_ver = c.i16();
  c.i16();
  c.i32();
  int32_t path_len = c.i32();
  c.skip(path_len < 0 ? (int64_t)(-path_len) * 2 : (int64_t)path_len);
  c.u32();
  h.name_count = c.i32();
  h.name_offset = c.i32();
  h.export_count = c.i32();
  h.export_offset = c.i32();
  return h;
}

static std::vector<std::string> read_names(const uint8_t *data, size_t size,
                                           const Header &h) {
  UPKCursor c{data, size};
  c.seek(h.name_offset);
  std::vector<std::string> names(h.name_count);
  for (auto &name : names) {
    int32_t len = c.i32();
    if (len == 0) {
      c.u64();
      continue;
    }
    if (len > 0) {
      if (c.pos + (size_t)len > size)
        throw std::runtime_error("UPK: name OOB");
      std::string s(len, '\0');
      memcpy(s.data(), data + c.pos, len);
      c.pos += len;
      if (!s.empty() && s.back() == '\0')
        s.pop_back();
      name = std::move(s);
    } else {
      int count = -len;
      std::vector<uint16_t> chars(count);
      for (auto &ch : chars)
        ch = c.u16();
      if (!chars.empty() && chars.back() == 0)
        chars.pop_back();
      int nb = WideCharToMultiByte(
          CP_UTF8, 0, reinterpret_cast<LPCWSTR>(chars.data()),
          (int)chars.size(), nullptr, 0, nullptr, nullptr);
      std::string s(nb, '\0');
      WideCharToMultiByte(CP_UTF8, 0, reinterpret_cast<LPCWSTR>(chars.data()),
                          (int)chars.size(), s.data(), nb, nullptr, nullptr);
      name = std::move(s);
    }
    c.u64();
  }
  return names;
}

struct Export {
  int32_t name_idx, outer_index;
  int32_t serial_size, serial_offset;
  size_t serial_size_fpos, serial_offset_fpos;
};

static std::vector<Export> read_exports(const uint8_t *data, size_t size,
                                        const Header &h) {
  UPKCursor c{data, size};
  c.seek(h.export_offset);
  std::vector<Export> out(h.export_count);
  for (auto &e : out) {
    c.i32();
    c.i32();
    e.outer_index = c.i32();
    e.name_idx = c.i32();
    c.i32();
    c.i32();
    c.u64();
    e.serial_size_fpos = c.pos;
    e.serial_size = c.i32();
    e.serial_offset_fpos = c.pos;
    e.serial_offset = c.i32();
    if (h.p_ver < 543) {
      int n = c.i32();
      c.skip((int64_t)n * 12);
    }
    c.u32();
    int64_t gc = c.i32();
    c.skip(gc * 4);
    c.skip(20);
  }
  return out;
}

static std::string export_path(const std::vector<std::string> &names,
                               const std::vector<Export> &exps, size_t idx) {
  std::vector<std::string> parts;
  size_t cur = idx;
  for (;;) {
    size_t ni = (size_t)exps[cur].name_idx;
    parts.push_back(ni < names.size() ? names[ni] : "<bad>");
    int32_t outer = exps[cur].outer_index;
    if (outer <= 0)
      break;
    cur = (size_t)(outer - 1);
    if (cur >= exps.size())
      break;
  }
  std::reverse(parts.begin(), parts.end());
  std::string r;
  for (size_t i = 0; i < parts.size(); ++i) {
    if (i)
      r += '.';
    r += parts[i];
  }
  return r;
}

static std::vector<uint8_t>
rebuild(const uint8_t *raw, size_t raw_size, const std::vector<Export> &exps,
        const std::unordered_map<size_t, std::vector<uint8_t>> &reps) {
  std::vector<size_t> order;
  for (size_t i = 0; i < exps.size(); ++i)
    if (exps[i].serial_size > 0)
      order.push_back(i);
  std::sort(order.begin(), order.end(), [&](size_t a, size_t b) {
    return exps[a].serial_offset < exps[b].serial_offset;
  });

  size_t data_start =
      order.empty() ? raw_size : (size_t)exps[order.front()].serial_offset;
  size_t data_end = order.empty() ? raw_size
                                  : (size_t)(exps[order.back()].serial_offset +
                                             exps[order.back()].serial_size);

  std::vector<uint8_t> out(raw, raw + data_start);

  std::vector<std::pair<int32_t, int32_t>> new_off(exps.size());
  for (size_t i = 0; i < exps.size(); ++i)
    new_off[i] = {exps[i].serial_offset, exps[i].serial_size};

  size_t cur_off = data_start;
  for (size_t ei : order) {
    auto it = reps.find(ei);
    const uint8_t *blob;
    size_t sz;
    if (it != reps.end()) {
      blob = it->second.data();
      sz = it->second.size();
    } else {
      size_t s = (size_t)exps[ei].serial_offset;
      sz = (size_t)exps[ei].serial_size;
      blob = raw + s;
    }
    new_off[ei] = {(int32_t)cur_off, (int32_t)sz};
    out.insert(out.end(), blob, blob + sz);
    cur_off += sz;
  }
  if (data_end < raw_size)
    out.insert(out.end(), raw + data_end, raw + raw_size);

  for (size_t ei = 0; ei < exps.size(); ++ei) {
    auto [no, ns] = new_off[ei];
    size_t sp = exps[ei].serial_size_fpos, op = exps[ei].serial_offset_fpos;
    if (sp + 4 <= out.size())
      memcpy(&out[sp], &ns, 4);
    if (op + 4 <= out.size())
      memcpy(&out[op], &no, 4);
  }
  return out;
}

std::vector<uint8_t> apply_cdo_patches(const uint8_t *raw, size_t size,
                                       const std::vector<CdoPatch> &patches) {
  auto h = read_header(raw, size);
  auto names = read_names(raw, size, h);
  auto exps = read_exports(raw, size, h);

  std::unordered_map<std::string, size_t> path_map;
  path_map.reserve(exps.size());
  for (size_t i = 0; i < exps.size(); ++i) {
    std::string k = export_path(names, exps, i);
    for (auto &c : k)
      c = (char)tolower((unsigned char)c);
    path_map[k] = i;
  }

  std::unordered_map<size_t, std::vector<uint8_t>> reps;
  for (const auto &p : patches) {
    std::string key = p.object_path;
    for (auto &c : key)
      c = (char)tolower((unsigned char)c);
    auto it = path_map.find(key);
    if (it == path_map.end()) {
      for (auto &[k, v] : path_map) {
        if ((k.size() >= key.size() &&
             k.compare(k.size() - key.size(), key.size(), key) == 0) ||
            (key.size() >= k.size() &&
             key.compare(key.size() - k.size(), k.size(), k) == 0)) {
          it = path_map.find(k);
          break;
        }
      }
    }
    if (it != path_map.end())
      reps[it->second] = p.data;
  }

  if (reps.empty())
    return std::vector<uint8_t>(raw, raw + size);
  return rebuild(raw, size, exps, reps);
}
