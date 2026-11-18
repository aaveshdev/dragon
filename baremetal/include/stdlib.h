#ifndef DGN_BM_STDLIB_H
#define DGN_BM_STDLIB_H
#include <stddef.h>
void* malloc(size_t);
void* calloc(size_t, size_t);
void* realloc(void*, size_t);
void free(void*);
void exit(int) __attribute__((noreturn));
void abort(void) __attribute__((noreturn));
int atoi(const char*);
long atol(const char*);
double atof(const char*);
double strtod(const char*, char**);
long strtol(const char*, char**, int);
unsigned long strtoul(const char*, char**, int);
long long strtoll(const char*, char**, int);
int rand(void);
void srand(unsigned);
#define RAND_MAX 2147483647
char* getenv(const char*);
int system(const char*);
void qsort(void*, size_t, size_t, int (*)(const void*, const void*));
int abs(int);
long labs(long);
#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
long long atoll(const char*);
int setenv(const char*, const char*, int);
#endif
