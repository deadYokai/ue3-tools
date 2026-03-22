#pragma once
#define WIN32_LEAN_AND_MEAN
#include <windows.h>

namespace hook {
void install_create_file_hook();
void remove_create_file_hook();

HANDLE WINAPI real_create_file_w(LPCWSTR lpFileName, DWORD dwDesiredAccess,
                                 DWORD dwShareMode,
                                 LPSECURITY_ATTRIBUTES lpSecurityAttributes,
                                 DWORD dwCreationDisposition,
                                 DWORD dwFlagsAndAttributes,
                                 HANDLE hTemplateFile);
} // namespace hook
