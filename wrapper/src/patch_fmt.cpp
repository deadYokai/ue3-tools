#define WIN32_LEAN_AND_MEAN
#include "patch_fmt.hpp"
#include <cstring>
#include <stdexcept>
#include <windows.h>
#include <zlib.h>

static constexpr size_t BLOCK_SIZE = 0x20000;

struct PReader {
  const uint8_t *data;
  size_t pos, size;

  PReader(const uint8_t *d, size_t s) : data(d), pos(0), size(s) {}

  int32_t read_i32() {
    if (pos + 4 > size)
      throw std::runtime_error("patch_fmt: EOF reading i32");
    int32_t v;
    memcpy(&v, data + pos, 4);
    pos += 4;
    return v;
  }
  uint16_t read_u16() {
    if (pos + 2 > size)
      throw std::runtime_error("patch_fmt: EOF reading u16");
    uint16_t v;
    memcpy(&v, data + pos, 2);
    pos += 2;
    return v;
  }
  void read_exact(uint8_t *dst, size_t n) {
    if (pos + n > size)
      throw std::runtime_error("patch_fmt: EOF reading bytes");
    memcpy(dst, data + pos, n);
    pos += n;
  }

  std::string read_ue3_str() {
    int32_t len = read_i32();
    if (len == 0)
      return {};
    if (len > 0) {
      std::string s(len, '\0');
      read_exact(reinterpret_cast<uint8_t *>(s.data()), len);
      if (!s.empty() && s.back() == '\0')
        s.pop_back();
      return s;
    }
    int count = -len;
    std::vector<uint16_t> chars(count);
    for (auto &c : chars)
      c = read_u16();
    if (!chars.empty() && chars.back() == 0)
      chars.pop_back();
    int nb =
        WideCharToMultiByte(CP_UTF8, 0, reinterpret_cast<LPCWSTR>(chars.data()),
                            (int)chars.size(), nullptr, 0, nullptr, nullptr);
    std::string s(nb, '\0');
    WideCharToMultiByte(CP_UTF8, 0, reinterpret_cast<LPCWSTR>(chars.data()),
                        (int)chars.size(), s.data(), nb, nullptr, nullptr);
    return s;
  }

  std::vector<uint8_t> read_byte_array() {
    int32_t n = read_i32();
    if (n <= 0)
      return {};
    std::vector<uint8_t> b(n);
    read_exact(b.data(), n);
    return b;
  }

  void skip_str_array() {
    int32_t n = read_i32();
    for (int32_t i = 0; i < n; ++i)
      read_ue3_str();
  }
  void require_empty_array(const char *name) {
    int32_t n = read_i32();
    if (n != 0)
      throw std::runtime_error(std::string(name) + " array must be empty");
  }
  void skip_patch_array() {
    int32_t n = read_i32();
    for (int32_t i = 0; i < n; ++i) {
      read_ue3_str();
      read_byte_array();
    }
  }
};

static std::vector<uint8_t> inflate_block(const uint8_t *src, uInt src_sz,
                                          size_t expected) {
  std::vector<uint8_t> out(expected);
  z_stream strm{};
  if (inflateInit(&strm) != Z_OK)
    throw std::runtime_error("inflateInit failed");
  strm.next_in = const_cast<Bytef *>(src);
  strm.avail_in = src_sz;
  strm.next_out = out.data();
  strm.avail_out = static_cast<uInt>(expected);
  int ret = inflate(&strm, Z_FINISH);
  inflateEnd(&strm);
  if (ret != Z_STREAM_END)
    throw std::runtime_error("inflate failed: " + std::to_string(ret));
  return out;
}

static std::vector<CdoPatch> extract_cdo(const std::vector<uint8_t> &unc) {
  PReader r{unc.data(), unc.size()};
  r.read_ue3_str();
  r.skip_str_array();
  r.require_empty_array("Exports");
  r.require_empty_array("Imports");
  r.skip_patch_array();

  int32_t n = r.read_i32();
  std::vector<CdoPatch> out;
  out.reserve(n);
  for (int32_t i = 0; i < n; ++i) {
    std::string name = r.read_ue3_str();
    auto data = r.read_byte_array();
    out.push_back(CdoPatch{std::move(name), std::move(data)});
  }
  return out;
}

std::vector<CdoPatch> load_patch_bin(const uint8_t *bin, size_t bin_size) {
  if (bin_size < 8)
    throw std::runtime_error("patch bin too short");

  uint32_t unc_total;
  memcpy(&unc_total, bin, 4);

  size_t n_blocks = (unc_total + BLOCK_SIZE - 1) / BLOCK_SIZE;
  size_t hdr_end = 8 + n_blocks * 8;
  if (hdr_end > bin_size)
    throw std::runtime_error("patch bin header out of bounds");

  std::vector<uint8_t> unc;
  unc.reserve(unc_total);

  size_t pos = hdr_end;
  for (size_t i = 0; i < n_blocks; ++i) {
    size_t h = 8 + i * 8;
    uint32_t cs;
    memcpy(&cs, bin + h, 4);
    size_t block_unc =
        (i + 1 < n_blocks) ? BLOCK_SIZE : (unc_total - i * BLOCK_SIZE);
    if (pos + cs > bin_size)
      throw std::runtime_error("block out of bounds");
    auto blk = inflate_block(bin + pos, static_cast<uInt>(cs), block_unc);
    unc.insert(unc.end(), blk.begin(), blk.end());
    pos += cs;
  }

  return extract_cdo(unc);
}
