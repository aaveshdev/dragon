#ifndef DGN_BM_REGEX_H
#define DGN_BM_REGEX_H
#include <stddef.h>
typedef struct { int unused; } regex_t;
typedef long regoff_t;
typedef struct { regoff_t rm_so, rm_eo; } regmatch_t;
#define REG_EXTENDED 1
#define REG_NOMATCH 1
int regcomp(regex_t*, const char*, int);
int regexec(const regex_t*, const char*, size_t, regmatch_t*, int);
void regfree(regex_t*);
#endif
