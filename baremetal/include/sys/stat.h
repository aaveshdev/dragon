#ifndef DGN_BM_STAT_H
#define DGN_BM_STAT_H
#include <stddef.h>
struct stat { long st_size; unsigned st_mode; long st_mtime; };
#define S_ISDIR(m) (((m) & 0170000) == 0040000)
#define S_ISREG(m) (((m) & 0170000) == 0100000)
int stat(const char*, struct stat*);
int mkdir(const char*, unsigned);
#endif
