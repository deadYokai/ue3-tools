#pragma once
#include "patch_fmt.hpp"
#include <cstdint>
#include <vector>

std::vector<uint8_t> apply_cdo_patches(const uint8_t *raw, size_t size,
                                       const std::vector<CdoPatch> &patches);
