#ifndef DGN_BM_UNISTD_H
#define DGN_BM_UNISTD_H
#include <stddef.h>
typedef long ssize_t;
int usleep(unsigned);
unsigned sleep(unsigned);
ssize_t read(int, void*, size_t);
ssize_t write(int, const void*, size_t);
int close(int);
char* getcwd(char*, size_t);
int isatty(int);
int rmdir(const char*);
int chdir(const char*);
#endif
