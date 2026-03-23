#pragma once
#include "mod_loader.hpp"
#include <string>
#include <vector>

namespace mod_toml_mini {

std::vector<ObjectReplacement> parse(const std::string &text,
                                     const std::string &mod_dir_str);

} // namespace mod_toml_mini
