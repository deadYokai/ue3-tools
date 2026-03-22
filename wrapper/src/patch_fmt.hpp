#pragma once
#include <cstdint>
#include <string>
#include <vector>

struct CdoPatch {
  std::string object_path;
  std::vector<uint8_t> data;
};

std::vector<CdoPatch> load_patch_bin(const uint8_t *data, size_t size);
