#pragma once
#include "patch_fmt.hpp"
#include <string>
#include <unordered_map>
#include <vector>

namespace mod_toml_mini {
std::unordered_map<std::string, std::vector<CdoPatch>>
parse(const std::string &text, const std::string &mod_dir);
}
