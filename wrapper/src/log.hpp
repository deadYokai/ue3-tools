#pragma once
#define WIN32_LEAN_AND_MEAN
#include <string>
#include <windows.h>

namespace log {
void init(const std::wstring &exe_dir);
void raw_write(const char *s);
void write_line(const char *level, const char *msg);
std::string vfmt(const char *fmt, ...);
} // namespace log

#define log_info(...) log::write_line("INFO ", log::vfmt(__VA_ARGS__).c_str())
#define log_warn(...) log::write_line("WARN ", log::vfmt(__VA_ARGS__).c_str())
#define log_err(...) log::write_line("ERROR", log::vfmt(__VA_ARGS__).c_str())
