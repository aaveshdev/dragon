#ifndef DGN_BM_TIME_H
#define DGN_BM_TIME_H
#include <stddef.h>
typedef long time_t; typedef long clock_t;
struct timespec { time_t tv_sec; long tv_nsec; };
struct tm { int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday, tm_isdst; };
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define CLOCKS_PER_SEC 1000000
time_t time(time_t*);
clock_t clock(void);
int clock_gettime(int, struct timespec*);
struct tm* localtime(const time_t*);
struct tm* gmtime(const time_t*);
time_t mktime(struct tm*);
size_t strftime(char*, size_t, const char*, const struct tm*);
#endif
