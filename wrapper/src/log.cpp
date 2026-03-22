#define WIN32_LEAN_AND_MEAN
#include "log.hpp"
#include "util.hpp"
#include <cstdarg>
#include <cstdio>
#include <fstream>
#include <mutex>
#include <windows.h>

namespace log {

struct Logger {
  std::ofstream file;
  std::mutex mtx;
  DWORD start_tick = 0;
};

static Logger *g_logger = nullptr;

void init(const std::wstring &exe_dir) {
  SYSTEMTIME st{};
  GetLocalTime(&st);

  std::string dir = to_narrow(exe_dir);
  char name[MAX_PATH * 4]{};
  snprintf(name, sizeof(name), "%s\\dinput8_%04d%02d%02d_%02d%02d%02d.log",
           dir.c_str(), (int)st.wYear, (int)st.wMonth, (int)st.wDay,
           (int)st.wHour, (int)st.wMinute, (int)st.wSecond);

  auto *lg = new Logger();
  lg->file.open(name, std::ios::trunc | std::ios::out);
  if (!lg->file.is_open()) {
    delete lg;
    return;
  }
  lg->start_tick = GetTickCount();
  g_logger = lg;

  char hdr[512];
  snprintf(hdr, sizeof(hdr), "====\nLog: %s\n====\n", name);
  raw_write(hdr);
}

void raw_write(const char *s) {
  if (!g_logger || !g_logger->file.is_open())
    return;
  std::lock_guard<std::mutex> lk(g_logger->mtx);
  g_logger->file << s;
  g_logger->file.flush();
}

void write_line(const char *level, const char *msg) {
  if (!g_logger || !g_logger->file.is_open())
    return;
  DWORD elapsed = GetTickCount() - g_logger->start_tick;
  DWORD tid = GetCurrentThreadId();
  char line[4096];
  snprintf(line, sizeof(line), "[%08lu][TID:%04lX] %s %s\n",
           (unsigned long)elapsed, (unsigned long)tid, level, msg);
  std::lock_guard<std::mutex> lk(g_logger->mtx);
  g_logger->file << line;
  g_logger->file.flush();
}

std::string vfmt(const char *fmt, ...) {
  char buf[2048];
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(buf, sizeof(buf), fmt, ap);
  va_end(ap);
  return buf;
}

} // namespace log
