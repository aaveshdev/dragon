#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <time.h>
#include <errno.h>
#include <unistd.h>
#include <setjmp.h>
#include <regex.h>
#include <dirent.h>
#include <pthread.h>
#include <dlfcn.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <sys/stat.h>
#ifdef DGN_USER
#define dgn_spin_lock(l) ((void)(l))
#define dgn_spin_unlock(l) ((void)(l))
#else
#include "dgn_pcpu.h"
#endif

void dgn_bm_putc(char c);
int dgn_bm_getc(void);
unsigned long long dgn_bm_now_us(void);
void dgn_bm_halt(int code) __attribute__((noreturn));
void dgn_bm_crit_enter(void);
void dgn_bm_crit_exit(void);

int errno;

extern char __heap_start[], __heap_end[];
char* dgn_bm_ram_end(void) __attribute__((weak));
char* dgn_bm_heap_start(void) __attribute__((weak));
typedef struct Blk { size_t size; size_t prev; size_t free; size_t pad; } Blk;
typedef struct Fl { struct Fl* next; struct Fl* prev; } Fl;
#define NBINS 88
static Fl* bins[NBINS];
static int heap_ready;
static char* heap_top;
static size_t heap_peak, heap_used;
static Blk* blk_next(Blk* b) { char* n = (char*)b + sizeof(Blk) + b->size; return n < heap_top ? (Blk*)n : 0; }
static Fl* fl_of(Blk* b) { return (Fl*)((char*)b + sizeof(Blk)); }
static Blk* blk_of(Fl* f) { return (Blk*)((char*)f - sizeof(Blk)); }
static int bin_of(size_t sz) {
    if (sz <= 1024) return (int)(sz / 16);
    int i = 64; size_t t = 1024; while (t < sz && i < NBINS - 1) { t <<= 1; i++; }
    return i;
}
static void bin_add(Blk* b) {
    int i = bin_of(b->size); Fl* f = fl_of(b);
    f->prev = 0; f->next = bins[i]; if (bins[i]) bins[i]->prev = f; bins[i] = f;
}
static void bin_del(Blk* b) {
    int i = bin_of(b->size); Fl* f = fl_of(b);
    if (f->prev) f->prev->next = f->next; else bins[i] = f->next;
    if (f->next) f->next->prev = f->prev;
}
static void heap_init(void) {
    heap_ready = 1;
    heap_top = (char*)(((uintptr_t)(dgn_bm_ram_end ? dgn_bm_ram_end() : __heap_end)) & ~(uintptr_t)15);
    Blk* b = (Blk*)(((uintptr_t)(dgn_bm_heap_start ? dgn_bm_heap_start() : __heap_start) + 15) & ~(uintptr_t)15);
    b->size = (size_t)(heap_top - (char*)b) - sizeof(Blk); b->prev = 0; b->free = 1;
    bin_add(b);
}
static void* malloc_locked(size_t n);
static volatile int heap_lock;
void* malloc(size_t n) { dgn_spin_lock(&heap_lock); void* p = malloc_locked(n); dgn_spin_unlock(&heap_lock); return p; }
static void* malloc_locked(size_t n) {
    if (!heap_ready) heap_init();
    if (n < 16) n = 16;
    n = (n + 15) & ~(size_t)15;
    Blk* b = 0;
    for (int i = bin_of(n); i < NBINS && !b; i++) {
        for (Fl* f = bins[i]; f; f = f->next) if (blk_of(f)->size >= n) { b = blk_of(f); break; }
    }
    if (!b) { errno = ENOMEM; return 0; }
    bin_del(b);
    if (b->size >= n + sizeof(Blk) + 16) {
        Blk* r = (Blk*)((char*)b + sizeof(Blk) + n);
        r->size = b->size - n - sizeof(Blk); r->free = 1; r->prev = (size_t)b;
        Blk* nx = blk_next(r); if (nx) nx->prev = (size_t)r;
        b->size = n; bin_add(r);
    }
    b->free = 0; heap_used += b->size; if (heap_used > heap_peak) heap_peak = heap_used;
    return (char*)b + sizeof(Blk);
}
static void free_locked(void* p);
void free(void* p) { dgn_spin_lock(&heap_lock); free_locked(p); dgn_spin_unlock(&heap_lock); }
static void free_locked(void* p) {
    if (!p) return;
    Blk* b = (Blk*)((char*)p - sizeof(Blk));
    if (b->free) return;
    heap_used -= b->size;
    Blk* nx = blk_next(b);
    if (nx && nx->free) { bin_del(nx); b->size += sizeof(Blk) + nx->size; Blk* n2 = blk_next(b); if (n2) n2->prev = (size_t)b; }
    Blk* pv = (Blk*)b->prev;
    if (pv && pv->free) { bin_del(pv); pv->size += sizeof(Blk) + b->size; Blk* n3 = blk_next(pv); if (n3) n3->prev = (size_t)pv; b = pv; }
    b->free = 1; bin_add(b);
}
void* calloc(size_t a, size_t b) { size_t n = a * b; void* p = malloc(n); if (p) memset(p, 0, n); return p; }
void* realloc(void* p, size_t n) {
    if (!p) return malloc(n);
    Blk* b = (Blk*)((char*)p - sizeof(Blk));
    if (b->size >= n) return p;
    void* q = malloc(n); if (!q) return 0;
    memcpy(q, p, b->size); free(p); return q;
}
size_t dgn_bm_heap_used(void) { return heap_used; }
size_t dgn_bm_heap_peak(void) { return heap_peak; }

#if defined(__x86_64__)
void* memcpy(void* d, const void* s, size_t n) {
    void* r = d; size_t q = n >> 3, t = n & 7;
    __asm__ volatile("rep movsq" : "+D"(d), "+S"(s), "+c"(q) : : "memory");
    __asm__ volatile("rep movsb" : "+D"(d), "+S"(s), "+c"(t) : : "memory");
    return r;
}
void* memmove(void* d, const void* s, size_t n) {
    if ((char*)d <= (const char*)s || (char*)d >= (const char*)s + n) return memcpy(d, s, n);
    char* a = (char*)d + n - 1; const char* b = (const char*)s + n - 1;
    __asm__ volatile("std\n\trep movsb\n\tcld" : "+D"(a), "+S"(b), "+c"(n) : : "memory");
    return d;
}
void* memset(void* d, int c, size_t n) {
    void* r = d; size_t q = n >> 3, t = n & 7;
    unsigned long v = (unsigned char)c * 0x0101010101010101UL;
    __asm__ volatile("rep stosq" : "+D"(d), "+c"(q) : "a"(v) : "memory");
    __asm__ volatile("rep stosb" : "+D"(d), "+c"(t) : "a"(v) : "memory");
    return r;
}
#else
void* memcpy(void* d, const void* s, size_t n) {
    char* a = d; const char* b = s;
    if ((((unsigned long)a | (unsigned long)b) & 7) == 0) { while (n >= 8) { *(unsigned long*)a = *(const unsigned long*)b; a += 8; b += 8; n -= 8; } }
    while (n--) *a++ = *b++;
    return d;
}
void* memmove(void* d, const void* s, size_t n) {
    char* a = d; const char* b = s;
    if (a < b) { while (n--) *a++ = *b++; } else { a += n; b += n; while (n--) *--a = *--b; }
    return d;
}
void* memset(void* d, int c, size_t n) {
    char* a = d;
    if ((((unsigned long)a) & 7) == 0 && n >= 8) { unsigned long v = (unsigned char)c * 0x0101010101010101UL; while (n >= 8) { *(unsigned long*)a = v; a += 8; n -= 8; } }
    while (n--) *a++ = (char)c;
    return d;
}
#endif
int memcmp(const void* x, const void* y, size_t n) {
    const unsigned char *a = x, *b = y;
    for (; n; n--, a++, b++) if (*a != *b) return *a - *b;
    return 0;
}
void* memchr(const void* s, int c, size_t n) { const unsigned char* a = s; for (; n; n--, a++) if (*a == (unsigned char)c) return (void*)a; return 0; }
#define DGN_LO 0x0101010101010101ULL
#define DGN_HI 0x8080808080808080ULL
size_t strlen(const char* s) {
    const char* p = s;
    while (((uintptr_t)p & 7) && *p) p++;
    if (!((uintptr_t)p & 7)) {
        const unsigned long long* w = (const unsigned long long*)p;
        while (!((*w - DGN_LO) & ~*w & DGN_HI)) w++;
        p = (const char*)w;
    }
    while (*p) p++;
    return (size_t)(p - s);
}
int strcmp(const char* a, const char* b) { while (*a && *a == *b) { a++; b++; } return (unsigned char)*a - (unsigned char)*b; }
int strncmp(const char* a, const char* b, size_t n) {
    for (; n; n--, a++, b++) { if (*a != *b) return (unsigned char)*a - (unsigned char)*b; if (!*a) return 0; }
    return 0;
}
int strcasecmp(const char* a, const char* b) { while (*a && tolower(*a) == tolower(*b)) { a++; b++; } return tolower((unsigned char)*a) - tolower((unsigned char)*b); }
int strncasecmp(const char* a, const char* b, size_t n) {
    for (; n; n--, a++, b++) { int d = tolower((unsigned char)*a) - tolower((unsigned char)*b); if (d) return d; if (!*a) return 0; }
    return 0;
}
char* strcpy(char* d, const char* s) { char* r = d; while ((*d++ = *s++)) {} return r; }
char* strncpy(char* d, const char* s, size_t n) { size_t i = 0; for (; i < n && s[i]; i++) d[i] = s[i]; for (; i < n; i++) d[i] = 0; return d; }
char* strcat(char* d, const char* s) { strcpy(d + strlen(d), s); return d; }
char* strncat(char* d, const char* s, size_t n) { char* e = d + strlen(d); while (n-- && *s) *e++ = *s++; *e = 0; return d; }
char* strchr(const char* s, int c) { for (;; s++) { if (*s == (char)c) return (char*)s; if (!*s) return 0; } }
char* strrchr(const char* s, int c) { const char* r = 0; for (;; s++) { if (*s == (char)c) r = s; if (!*s) return (char*)r; } }
char* strstr(const char* h, const char* n) {
    size_t m = strlen(n); if (!m) return (char*)h;
    for (; *h; h++) if (*h == *n && !strncmp(h, n, m)) return (char*)h;
    return 0;
}
char* strdup(const char* s) { size_t n = strlen(s) + 1; char* p = malloc(n); if (p) memcpy(p, s, n); return p; }
char* strndup(const char* s, size_t n) { size_t l = 0; while (l < n && s[l]) l++; char* p = malloc(l + 1); if (p) { memcpy(p, s, l); p[l] = 0; } return p; }
size_t strspn(const char* s, const char* a) { size_t n = 0; while (s[n] && strchr(a, s[n])) n++; return n; }
size_t strcspn(const char* s, const char* r) { size_t n = 0; while (s[n] && !strchr(r, s[n])) n++; return n; }
char* strpbrk(const char* s, const char* a) { for (; *s; s++) if (strchr(a, *s)) return (char*)s; return 0; }
char* strerror(int e) { return e == ENOENT ? "no such file" : e == ENOMEM ? "out of memory" : "error"; }

int isdigit(int c) { return c >= '0' && c <= '9'; }
int isupper(int c) { return c >= 'A' && c <= 'Z'; }
int islower(int c) { return c >= 'a' && c <= 'z'; }
int isalpha(int c) { return isupper(c) || islower(c); }
int isalnum(int c) { return isalpha(c) || isdigit(c); }
int isspace(int c) { return c == ' ' || (c >= 9 && c <= 13); }
int isxdigit(int c) { return isdigit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F'); }
int isprint(int c) { return c >= 32 && c < 127; }
int isgraph(int c) { return c > 32 && c < 127; }
int iscntrl(int c) { return c < 32 || c == 127; }
int ispunct(int c) { return isgraph(c) && !isalnum(c); }
int toupper(int c) { return islower(c) ? c - 32 : c; }
int tolower(int c) { return isupper(c) ? c + 32 : c; }
int abs(int a) { return a < 0 ? -a : a; }
long labs(long a) { return a < 0 ? -a : a; }

#define TWO52 4503599627370496.0

#if defined(__x86_64__)
double fabs(double x) { union { double d; unsigned long long u; } v; v.d = x; v.u &= 0x7FFFFFFFFFFFFFFFULL; return v.d; }
double sqrt(double x) { double r; __asm__("sqrtsd %1, %0" : "=x"(r) : "x"(x)); return r; }
#elif defined(__aarch64__)
double fabs(double x) { double r; __asm__("fabs %d0, %d1" : "=w"(r) : "w"(x)); return r; }
double sqrt(double x) { double r; __asm__("fsqrt %d0, %d1" : "=w"(r) : "w"(x)); return r; }
#else
double fabs(double x) { double r; __asm__("fabs.d %0, %1" : "=f"(r) : "f"(x)); return r; }
double sqrt(double x) { double r; __asm__("fsqrt.d %0, %1" : "=f"(r) : "f"(x)); return r; }
#endif
double trunc(double x) { if (!(fabs(x) < TWO52)) return x; return (double)(long long)x; }
double floor(double x) { double t = trunc(x); return (t > x) ? t - 1.0 : t; }
double ceil(double x) { double t = trunc(x); return (t < x) ? t + 1.0 : t; }
double round(double x) { return x < 0 ? -floor(-x + 0.5) : floor(x + 0.5); }
double rint(double x) { return round(x); }
long long llround(double x) { return (long long)round(x); }
long lround(double x) { return (long)round(x); }
double fmin(double a, double b) { return a < b ? a : b; }
double fmax(double a, double b) { return a > b ? a : b; }
double fmod(double a, double b) { if (b == 0) return NAN; double q = trunc(a / b); return a - q * b; }
double ldexp(double x, int e) { while (e > 0) { x *= 2; e--; } while (e < 0) { x *= 0.5; e++; } return x; }
double frexp(double x, int* e) {
    *e = 0; if (x == 0 || isnan(x) || isinf(x)) return x;
    while (fabs(x) >= 1.0) { x *= 0.5; (*e)++; }
    while (fabs(x) < 0.5) { x *= 2.0; (*e)--; }
    return x;
}
double modf(double x, double* ip) { double t = trunc(x); *ip = t; return x - t; }
double exp(double x) {
    if (isnan(x)) return x; if (x > 709.78) return INFINITY; if (x < -745.0) return 0;
    const double ln2 = 0.69314718055994530942;
    double k = round(x / ln2); double r = x - k * ln2;
    double term = 1, sum = 1;
    for (int i = 1; i < 22; i++) { term *= r / i; sum += term; }
    return ldexp(sum, (int)k);
}
double log(double x) {
    if (isnan(x) || x < 0) return NAN; if (x == 0) return -INFINITY; if (isinf(x)) return x;
    int e; double m = frexp(x, &e);
    if (m < 0.70710678118654752) { m *= 2; e--; }
    double s = (m - 1) / (m + 1), s2 = s * s, term = s, sum = 0;
    for (int i = 1; i < 60; i += 2) { sum += term / i; term *= s2; }
    return 2 * sum + e * 0.69314718055994530942;
}
double log10(double x) { return log(x) / 2.30258509299404568402; }
double log2(double x) { return log(x) / 0.69314718055994530942; }
double pow(double x, double y) {
    if (y == 0) return 1; if (x == 1) return 1;
    if (y == trunc(y) && fabs(y) < 1e9) {
        long long n = (long long)y; int neg = n < 0; if (neg) n = -n;
        double r = 1, b = x; while (n) { if (n & 1) r *= b; b *= b; n >>= 1; }
        return neg ? 1.0 / r : r;
    }
    if (x < 0) return NAN; if (x == 0) return y > 0 ? 0 : INFINITY;
    return exp(y * log(x));
}
double cbrt(double x) { return x < 0 ? -pow(-x, 1.0 / 3) : pow(x, 1.0 / 3); }
double hypot(double a, double b) { return sqrt(a * a + b * b); }
static double sin_core(double r) { double t = r, s = r, r2 = r * r; for (int i = 1; i < 14; i++) { t *= -r2 / ((2 * i) * (2 * i + 1)); s += t; } return s; }
static double cos_core(double r) { double t = 1, s = 1, r2 = r * r; for (int i = 1; i < 14; i++) { t *= -r2 / ((2 * i - 1) * (2 * i)); s += t; } return s; }
#define PI 3.14159265358979323846
double sin(double x) {
    if (isnan(x) || isinf(x)) return NAN;
    double k = round(x / (PI / 2)); double r = x - k * (PI / 2); int q = (int)fmod(k, 4); if (q < 0) q += 4;
    return q == 0 ? sin_core(r) : q == 1 ? cos_core(r) : q == 2 ? -sin_core(r) : -cos_core(r);
}
double cos(double x) { return sin(x + PI / 2); }
double tan(double x) { return sin(x) / cos(x); }
double atan(double x) {
    if (isnan(x)) return x; int neg = x < 0; if (neg) x = -x; int inv = x > 1; if (inv) x = 1 / x;
    int halve = 0; if (x > 0.4142135623730950) { x = (x - 1) / (x + 1); halve = 1; }
    double t = x, s = x, x2 = x * x; for (int i = 1; i < 40; i++) { t *= -x2; s += t / (2 * i + 1); }
    if (halve) s += PI / 4; if (inv) s = PI / 2 - s; return neg ? -s : s;
}
double atan2(double y, double x) {
    if (x > 0) return atan(y / x); if (x < 0) return y >= 0 ? atan(y / x) + PI : atan(y / x) - PI;
    return y > 0 ? PI / 2 : y < 0 ? -PI / 2 : 0;
}
double asin(double x) { if (x > 1 || x < -1) return NAN; return atan2(x, sqrt(1 - x * x)); }
double acos(double x) { if (x > 1 || x < -1) return NAN; return PI / 2 - asin(x); }
double sinh(double x) { return (exp(x) - exp(-x)) / 2; }
double cosh(double x) { return (exp(x) + exp(-x)) / 2; }
double tanh(double x) { if (x > 20) return 1; if (x < -20) return -1; double a = exp(x), b = exp(-x); return (a - b) / (a + b); }

double strtod(const char* s, char** end) {
    const char* p = s; while (isspace((unsigned char)*p)) p++;
    int neg = 0; if (*p == '-') { neg = 1; p++; } else if (*p == '+') p++;
    double v = 0; int any = 0;
    while (isdigit((unsigned char)*p)) { v = v * 10 + (*p - '0'); p++; any = 1; }
    if (*p == '.') { p++; double f = 0.1; while (isdigit((unsigned char)*p)) { v += (*p - '0') * f; f *= 0.1; p++; any = 1; } }
    if (!any) { if (end) *end = (char*)s; return 0; }
    if (*p == 'e' || *p == 'E') {
        const char* q = p + 1; int en = 0; if (*q == '-') { en = 1; q++; } else if (*q == '+') q++;
        if (isdigit((unsigned char)*q)) { int e = 0; while (isdigit((unsigned char)*q)) { e = e * 10 + (*q - '0'); q++; } v *= pow(10, en ? -e : e); p = q; }
    }
    if (end) *end = (char*)p; return neg ? -v : v;
}
double atof(const char* s) { return strtod(s, 0); }
long long strtoll(const char* s, char** end, int base) {
    const char* p = s; while (isspace((unsigned char)*p)) p++;
    int neg = 0; if (*p == '-') { neg = 1; p++; } else if (*p == '+') p++;
    if ((base == 0 || base == 16) && p[0] == '0' && (p[1] == 'x' || p[1] == 'X')) { p += 2; base = 16; }
    else if (base == 0) base = (p[0] == '0') ? 8 : 10;
    long long v = 0; int any = 0;
    for (;; p++) {
        int d = isdigit((unsigned char)*p) ? *p - '0' : isalpha((unsigned char)*p) ? tolower(*p) - 'a' + 10 : 99;
        if (d >= base) break; v = v * base + d; any = 1;
    }
    if (end) *end = (char*)(any ? p : s); return neg ? -v : v;
}
long strtol(const char* s, char** e, int b) { return (long)strtoll(s, e, b); }
unsigned long strtoul(const char* s, char** e, int b) { return (unsigned long)strtoll(s, e, b); }
int atoi(const char* s) { return (int)strtoll(s, 0, 10); }
long atol(const char* s) { return (long)strtoll(s, 0, 10); }
long long atoll(const char* s) { return strtoll(s, 0, 10); }

typedef struct { char* buf; size_t cap, len; } Out;
static void out_c(Out* o, char c) { if (o->len + 1 < o->cap) o->buf[o->len] = c; o->len++; }
static void out_pad(Out* o, int n, char c) { while (n-- > 0) out_c(o, c); }
static void out_num(Out* o, unsigned long long v, int base, int upper, int neg, int width, int prec, int left, int zero, char sign) {
    char tmp[72]; int n = 0; const char* dg = upper ? "0123456789ABCDEF" : "0123456789abcdef";
    if (v == 0) tmp[n++] = '0'; while (v) { tmp[n++] = dg[v % base]; v /= base; }
    int digits = n; if (prec > digits) digits = prec;
    int total = digits + (neg || sign ? 1 : 0);
    if (!left && !(zero && prec < 0)) out_pad(o, width - total, ' ');
    if (neg) out_c(o, '-'); else if (sign) out_c(o, sign);
    if (!left && zero && prec < 0) out_pad(o, width - total, '0');
    out_pad(o, digits - n, '0'); while (n) out_c(o, tmp[--n]);
    if (left) out_pad(o, width - total, ' ');
}

static int fmt_float(char* dst, double x, int prec, char mode, int alt) {
    char* d = dst;
    if (isnan(x)) { strcpy(d, "nan"); return 3; }
    if (isinf(x)) { strcpy(d, "inf"); return 3; }
    if (prec < 0) prec = 6;
    if (mode == 'g' && prec == 0) prec = 1;
    int e10 = 0;
    if (x != 0) { e10 = (int)floor(log10(x)); double p = pow(10, e10); if (p > x) e10--; else if (x >= p * 10) e10++; }
    if (mode == 'g') {

        double scaled = x / pow(10, e10) * pow(10, prec - 1); double r = floor(scaled + 0.5);
        if (r >= pow(10, prec)) e10++;
        if (x == 0) e10 = 0;
        if (e10 < -4 || e10 >= prec) { mode = 'e'; prec = prec - 1; } else { mode = 'f'; prec = prec - 1 - e10; }
        if (!alt) {   alt = -1; }
    }
    int stripzero = (alt == -1);
    if (mode == 'f') {
        double scale = pow(10, prec); double sx = x * scale; double r = floor(sx + 0.5);
        if (x < 1e15 && prec <= 15) {
            unsigned long long all = (unsigned long long)r; unsigned long long ip = all, fp = 0;
            if (prec > 0) { unsigned long long sc = 1; for (int i = 0; i < prec; i++) sc *= 10; ip = all / sc; fp = all % sc; }
            char t[32]; int n = 0; if (ip == 0) t[n++] = '0'; while (ip) { t[n++] = '0' + ip % 10; ip /= 10; } while (n) *d++ = t[--n];
            if (prec > 0) {
                char f[20]; for (int i = prec - 1; i >= 0; i--) { f[i] = '0' + fp % 10; fp /= 10; }
                int fl = prec; if (stripzero) while (fl > 0 && f[fl - 1] == '0') fl--;
                if (fl > 0) { *d++ = '.'; for (int i = 0; i < fl; i++) *d++ = f[i]; }
            }
        } else {
            double ip = floor(x); char t[400]; int n = 0; if (ip == 0) t[n++] = '0';
            while (ip >= 1 && n < 380) { double q = floor(ip / 10); t[n++] = '0' + (int)(ip - q * 10); ip = q; }
            while (n) *d++ = t[--n];
        }
    } else {
        double m = x == 0 ? 0 : x / pow(10, e10); double sc = pow(10, prec); double r = floor(m * sc + 0.5);
        if (r >= sc * 10) { r = floor(r / 10); e10++; }
        unsigned long long all = (unsigned long long)r, ip = all, fp = 0;
        if (prec > 0) { unsigned long long s2 = 1; for (int i = 0; i < prec && i < 18; i++) s2 *= 10; ip = all / s2; fp = all % s2; }
        *d++ = '0' + (char)(ip % 10);
        if (prec > 0) {
            char f[20]; int pp = prec < 18 ? prec : 18; for (int i = pp - 1; i >= 0; i--) { f[i] = '0' + fp % 10; fp /= 10; }
            int fl = pp; if (stripzero) while (fl > 0 && f[fl - 1] == '0') fl--;
            if (fl > 0) { *d++ = '.'; for (int i = 0; i < fl; i++) *d++ = f[i]; }
        }
        *d++ = 'e'; int ee = e10; if (ee < 0) { *d++ = '-'; ee = -ee; } else *d++ = '+';
        if (ee >= 100) { *d++ = '0' + ee / 100; ee %= 100; } *d++ = '0' + ee / 10; *d++ = '0' + ee % 10;
    }
    *d = 0; return (int)(d - dst);
}
int vsnprintf(char* buf, size_t cap, const char* f, va_list ap) {
    Out o = { buf, cap, 0 };
    for (; *f; f++) {
        if (*f != '%') { out_c(&o, *f); continue; }
        f++; int left = 0, zero = 0, plus = 0, space = 0, alt = 0;
        for (;; f++) { if (*f == '-') left = 1; else if (*f == '0') zero = 1; else if (*f == '+') plus = 1; else if (*f == ' ') space = 1; else if (*f == '#') alt = 1; else break; }
        int width = 0, prec = -1;
        if (*f == '*') { width = va_arg(ap, int); if (width < 0) { left = 1; width = -width; } f++; } else while (isdigit((unsigned char)*f)) width = width * 10 + (*f++ - '0');
        if (*f == '.') { f++; prec = 0; if (*f == '*') { prec = va_arg(ap, int); f++; } else while (isdigit((unsigned char)*f)) prec = prec * 10 + (*f++ - '0'); }
        int lng = 0; while (*f == 'l' || *f == 'z' || *f == 'j' || *f == 't') { lng++; f++; } if (*f == 'h') { f++; if (*f == 'h') f++; }
        char sign = plus ? '+' : space ? ' ' : 0;
        switch (*f) {
        case 'd': case 'i': { long long v = lng ? va_arg(ap, long long) : va_arg(ap, int); if (!lng) v = (int)v; int neg = v < 0; out_num(&o, neg ? (unsigned long long)-v : (unsigned long long)v, 10, 0, neg, width, prec, left, zero, sign); break; }
        case 'u': { unsigned long long v = lng ? va_arg(ap, unsigned long long) : va_arg(ap, unsigned); out_num(&o, v, 10, 0, 0, width, prec, left, zero, 0); break; }
        case 'x': case 'X': { unsigned long long v = lng ? va_arg(ap, unsigned long long) : va_arg(ap, unsigned); if (alt && v) { out_c(&o, '0'); out_c(&o, *f); } out_num(&o, v, 16, *f == 'X', 0, width, prec, left, zero, 0); break; }
        case 'o': { unsigned long long v = lng ? va_arg(ap, unsigned long long) : va_arg(ap, unsigned); out_num(&o, v, 8, 0, 0, width, prec, left, zero, 0); break; }
        case 'p': { unsigned long long v = (unsigned long long)(uintptr_t)va_arg(ap, void*); out_c(&o, '0'); out_c(&o, 'x'); out_num(&o, v, 16, 0, 0, width, prec, left, zero, 0); break; }
        case 'c': { char c = (char)va_arg(ap, int); if (!left) out_pad(&o, width - 1, ' '); out_c(&o, c); if (left) out_pad(&o, width - 1, ' '); break; }
        case 's': { const char* s = va_arg(ap, const char*); if (!s) s = "(null)"; size_t n = strlen(s); if (prec >= 0 && (size_t)prec < n) n = (size_t)prec;
                    if (!left) out_pad(&o, width - (int)n, ' '); for (size_t i = 0; i < n; i++) out_c(&o, s[i]); if (left) out_pad(&o, width - (int)n, ' '); break; }
        case 'f': case 'F': case 'e': case 'E': case 'g': case 'G': {
            double x = va_arg(ap, double); int neg = x < 0 || (x == 0 && 1 / x < 0); if (neg) x = -x;
            char t[480]; int n = fmt_float(t, x, prec, (char)tolower(*f), alt);
            if (*f == 'E' || *f == 'G') for (int i = 0; i < n; i++) t[i] = (char)toupper(t[i]);
            int total = n + (neg || sign ? 1 : 0);
            if (!left && !zero) out_pad(&o, width - total, ' ');
            if (neg) out_c(&o, '-'); else if (sign) out_c(&o, sign);
            if (!left && zero) out_pad(&o, width - total, '0');
            for (int i = 0; i < n; i++) out_c(&o, t[i]);
            if (left) out_pad(&o, width - total, ' '); break; }
        case '%': out_c(&o, '%'); break;
        default: out_c(&o, '%'); if (*f) out_c(&o, *f); else f--; break;
        }
    }
    if (cap) buf[o.len < cap ? o.len : cap - 1] = 0;
    return (int)o.len;
}
int snprintf(char* b, size_t n, const char* f, ...) { va_list a; va_start(a, f); int r = vsnprintf(b, n, f, a); va_end(a); return r; }
int sprintf(char* b, const char* f, ...) { va_list a; va_start(a, f); int r = vsnprintf(b, (size_t)-1 >> 1, f, a); va_end(a); return r; }
int vsprintf(char* b, const char* f, va_list a) { return vsnprintf(b, (size_t)-1 >> 1, f, a); }

#ifdef DGN_USER

struct DgnBmFile { int fd; char* buf; size_t len, cap, pos; int mode; int dirty; char path[160]; };
extern long dgn_sys(long n, long a, long b, long c, long d);
#define SYS_file_read 10
#define SYS_file_write 11
#else
struct DgnBmFile { int fd; };
#endif
static struct DgnBmFile f_in = { 0 }, f_out = { 1 }, f_err = { 2 };
FILE* stdin = &f_in; FILE* stdout = &f_out; FILE* stderr = &f_err;
#ifdef DGN_USER
static void put_str(const char* s, size_t n) { extern long dgn_sys(long, long, long, long, long); if (n) dgn_sys(1, 1, (long)s, (long)n, 0); }
#else

#define NCAP 8
static struct { int used, task; char* buf; size_t len, cap; } caps[NCAP];
static int ncap_used;
extern int dgn_bm_task_id(void);
int dgn_bm_cap_begin(void) {
    dgn_bm_crit_enter();
    int r = -1;
    for (int i = 0; i < NCAP; i++) if (!caps[i].used) { caps[i].used = 1; caps[i].task = dgn_bm_task_id(); caps[i].len = 0; ncap_used++; r = i; break; }
    dgn_bm_crit_exit(); return r;
}
void dgn_bm_cap_end(int i) { dgn_bm_crit_enter(); if (i >= 0 && i < NCAP && caps[i].used) { caps[i].used = 0; free(caps[i].buf); caps[i].buf = 0; caps[i].len = caps[i].cap = 0; ncap_used--; } dgn_bm_crit_exit(); }

char* dgn_bm_cap_take(int i) {
    if (i < 0 || i >= NCAP || !caps[i].used) return 0;
    dgn_bm_crit_enter();
    char* r = malloc(caps[i].len + 1);
    if (r) { memcpy(r, caps[i].buf, caps[i].len); r[caps[i].len] = 0; }
    caps[i].len = 0;
    dgn_bm_crit_exit(); return r;
}
static int cap_put(const char* s, size_t n) {
    int t = dgn_bm_task_id();
    for (int i = 0; i < NCAP; i++) if (caps[i].used && caps[i].task == t) {
        for (size_t k = 0; k < n; k++) {
            if (s[k] == '\r') continue;
            if (caps[i].len + 1 >= caps[i].cap) { if (caps[i].cap >= (1u << 20)) return 1; size_t nc = caps[i].cap ? caps[i].cap * 2 : 1024; char* nb = realloc(caps[i].buf, nc); if (!nb) return 1; caps[i].buf = nb; caps[i].cap = nc; }
            caps[i].buf[caps[i].len++] = s[k];
        }
        return 1;
    }
    return 0;
}
static void put_str(const char* s, size_t n) {
    dgn_bm_crit_enter();
    if (ncap_used && cap_put(s, n)) { dgn_bm_crit_exit(); return; }
    for (size_t i = 0; i < n; i++) { if (s[i] == '\n') dgn_bm_putc('\r'); dgn_bm_putc(s[i]); }
    dgn_bm_crit_exit();
}
#endif
#ifdef DGN_USER
static void file_put(FILE* fp, const char* s, size_t n) {
    if (fp->pos + n > fp->cap) { size_t nc = (fp->pos + n) * 2 + 64; char* nb = realloc(fp->buf, nc); if (!nb) return; fp->buf = nb; fp->cap = nc; }
    memcpy(fp->buf + fp->pos, s, n); fp->pos += n; if (fp->pos > fp->len) fp->len = fp->pos; fp->dirty = 1;
}
#define OUT(fp, s, n) ((fp)->fd > 2 ? file_put((fp), (s), (n)) : put_str((s), (n)))
#else
#define OUT(fp, s, n) put_str((s), (n))
#endif
int vfprintf(FILE* fp, const char* f, va_list ap) {
    char tmp[1024]; va_list c; va_copy(c, ap); int n = vsnprintf(tmp, sizeof tmp, f, ap);
    if (n >= (int)sizeof tmp) { char* big = malloc((size_t)n + 1); if (big) { vsnprintf(big, (size_t)n + 1, f, c); OUT(fp, big, (size_t)n); free(big); va_end(c); return n; } n = (int)sizeof tmp - 1; }
    va_end(c); OUT(fp, tmp, (size_t)n); return n;
}
int vprintf(const char* f, va_list a) { return vfprintf(stdout, f, a); }
int printf(const char* f, ...) { va_list a; va_start(a, f); int r = vfprintf(stdout, f, a); va_end(a); return r; }
int fprintf(FILE* fp, const char* f, ...) { va_list a; va_start(a, f); int r = vfprintf(fp, f, a); va_end(a); return r; }
int puts(const char* s) { put_str(s, strlen(s)); put_str("\n", 1); return 1; }
int putchar(int c) { char ch = (char)c; put_str(&ch, 1); return c; }
#ifdef DGN_USER
int fputc(int c, FILE* fp) { char ch = (char)c; OUT(fp, &ch, 1); return c; }
int fputs(const char* s, FILE* fp) { OUT(fp, s, strlen(s)); return 1; }
size_t fwrite(const void* p, size_t sz, size_t n, FILE* fp) { if (fp && fp->fd >= 1) OUT(fp, p, sz * n); return n; }
int fflush(FILE* fp) {
    if (fp && fp->fd > 2 && fp->dirty && fp->mode != 0) { dgn_sys(SYS_file_write, (long)fp->path, (long)fp->buf, (long)fp->len, 0); fp->dirty = 0; }
    return 0;
}
#else
int fputc(int c, FILE* fp) { (void)fp; return putchar(c); }
int fputs(const char* s, FILE* fp) { (void)fp; put_str(s, strlen(s)); return 1; }
size_t fwrite(const void* p, size_t sz, size_t n, FILE* fp) { if (fp && fp->fd >= 1) put_str(p, sz * n); return n; }
int fflush(FILE* fp) { (void)fp; return 0; }
#endif

int getchar(void) {
    int c = dgn_bm_getc(); if (c == '\r') c = '\n';
    if (c == '\n') { dgn_bm_putc('\r'); dgn_bm_putc('\n'); } else if (c >= 32 && c < 127) dgn_bm_putc((char)c);
    return c;
}
#ifdef DGN_USER
int fgetc(FILE* fp) { if (fp->fd > 2) return fp->pos < fp->len ? (unsigned char)fp->buf[fp->pos++] : -1; return dgn_bm_getc(); }
#else
int fgetc(FILE* fp) { (void)fp; return dgn_bm_getc(); }
#endif
char* fgets(char* b, int n, FILE* fp) {
    int i = 0;
#ifdef DGN_USER
    if (fp->fd > 2) { while (i < n - 1 && fp->pos < fp->len) { char c = fp->buf[fp->pos++]; b[i++] = c; if (c == '\n') break; } b[i] = 0; return i ? b : 0; }
#endif
    while (i < n - 1) { int c = dgn_bm_getc(); if (c < 0) break; if (c == '\r') c = '\n'; putchar(c); b[i++] = (char)c; if (c == '\n') break; }
    b[i] = 0; return i ? b : 0;
}
#ifdef DGN_USER
int feof(FILE* fp) { return fp->fd > 2 && fp->pos >= fp->len; }
FILE* fopen(const char* p, const char* m) {
    if (strlen(p) >= sizeof(((struct DgnBmFile*)0)->path)) { errno = ENOENT; return 0; }
    int mode = m[0] == 'r' ? 0 : m[0] == 'w' ? 1 : 2;
    FILE* fp = calloc(1, sizeof *fp);
    if (!fp) return 0;
    fp->fd = 3; fp->mode = mode; strcpy(fp->path, p);
    if (mode == 0) {
        long sz = dgn_sys(SYS_file_read, (long)p, 0, 0, 0);
        if (sz < 0) { free(fp); errno = ENOENT; return 0; }
        fp->buf = malloc((size_t)sz + 1); fp->cap = (size_t)sz + 1;
        if (!fp->buf) { free(fp); return 0; }
        if (sz > 0) dgn_sys(SYS_file_read, (long)p, (long)fp->buf, sz, 0);
        fp->len = (size_t)sz;
    } else if (mode == 2) {
        long sz = dgn_sys(SYS_file_read, (long)p, 0, 0, 0);
        if (sz > 0) { fp->buf = malloc((size_t)sz + 1); fp->cap = (size_t)sz + 1; if (fp->buf) { dgn_sys(SYS_file_read, (long)p, (long)fp->buf, sz, 0); fp->len = (size_t)sz; fp->pos = fp->len; } }
    } else {
        fp->dirty = 1;
    }
    return fp;
}
int fclose(FILE* fp) { if (!fp || fp->fd <= 2) return 0; fflush(fp); free(fp->buf); free(fp); return 0; }
size_t fread(void* p, size_t s, size_t n, FILE* fp) {
    if (!fp || fp->fd <= 2 || s == 0) return 0;
    size_t avail = fp->len - fp->pos, want = s * n; if (want > avail) want = avail - avail % s;
    memcpy(p, fp->buf + fp->pos, want); fp->pos += want; return want / s;
}
int fseek(FILE* fp, long o, int w) { if (!fp || fp->fd <= 2) return -1; long b = w == 0 ? 0 : w == 1 ? (long)fp->pos : (long)fp->len; long np = b + o; if (np < 0 || (size_t)np > fp->len) return -1; fp->pos = (size_t)np; return 0; }
long ftell(FILE* fp) { return fp && fp->fd > 2 ? (long)fp->pos : -1; }
void rewind(FILE* fp) { if (fp && fp->fd > 2) fp->pos = 0; }
#else
int feof(FILE* fp) { (void)fp; return 0; }
FILE* fopen(const char* p, const char* m) { (void)p; (void)m; errno = ENOENT; return 0; }
int fclose(FILE* fp) { (void)fp; return 0; }
size_t fread(void* p, size_t s, size_t n, FILE* fp) { (void)p; (void)s; (void)n; (void)fp; return 0; }
int fseek(FILE* fp, long o, int w) { (void)fp; (void)o; (void)w; return -1; }
long ftell(FILE* fp) { (void)fp; return -1; }
void rewind(FILE* fp) { (void)fp; }
#endif
int remove(const char* p) { (void)p; return -1; }
int rename(const char* a, const char* b) { (void)a; (void)b; return -1; }
int sscanf(const char* s, const char* f, ...) { (void)s; (void)f; return 0; }

#ifdef DGN_USER
extern long dgn_sys(long n, long a, long b, long c, long d);
#endif

void exit(int code) { dgn_bm_halt(code); }
void abort(void) { printf("abort()\n"); dgn_bm_halt(134); }
static unsigned long long rng = 88172645463325252ULL;
void srand(unsigned s) { rng = 88172645463325252ULL ^ ((unsigned long long)s << 13); }
int rand(void) { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; return (int)((rng >> 33) & 0x7fffffff); }
char* getenv(const char* n) { (void)n; return 0; }
int setenv(const char* a, const char* b, int c) { (void)a; (void)b; (void)c; return -1; }
int system(const char* c) { (void)c; return -1; }
void qsort(void* base, size_t n, size_t sz, int (*cmp)(const void*, const void*)) {
    char* a = base; char tmp[64];
    for (size_t i = 1; i < n; i++) for (size_t j = i; j > 0 && cmp(a + (j - 1) * sz, a + j * sz) > 0; j--) {
        char* x = a + (j - 1) * sz; char* y = a + j * sz;
        for (size_t k = 0; k < sz; k += sizeof tmp) { size_t m = sz - k < sizeof tmp ? sz - k : sizeof tmp; memcpy(tmp, x + k, m); memcpy(x + k, y + k, m); memcpy(y + k, tmp, m); }
    }
}
#define EPOCH_BASE 1760000000L
time_t time(time_t* t) { time_t v = EPOCH_BASE + (time_t)(dgn_bm_now_us() / 1000000ULL); if (t) *t = v; return v; }
clock_t clock(void) { return (clock_t)dgn_bm_now_us(); }
int clock_gettime(int id, struct timespec* ts) { unsigned long long u = dgn_bm_now_us(); ts->tv_sec = (id == CLOCK_REALTIME ? EPOCH_BASE : 0) + (long)(u / 1000000ULL); ts->tv_nsec = (long)(u % 1000000ULL) * 1000; return 0; }
#ifdef DGN_USER
int usleep(unsigned us) { if (us >= 1000) { dgn_sys(6, us / 1000, 0, 0, 0); return 0; } unsigned long long end = dgn_bm_now_us() + us; while (dgn_bm_now_us() < end) {} return 0; }
#else
int usleep(unsigned us) { unsigned long long end = dgn_bm_now_us() + us; while (dgn_bm_now_us() < end) {} return 0; }
#endif
unsigned sleep(unsigned s) { usleep(s * 1000000u); return 0; }
static struct tm g_tm;
struct tm* gmtime(const time_t* t) {
    long s = (long)*t, days = s / 86400, rem = s % 86400;
    g_tm.tm_hour = (int)(rem / 3600); g_tm.tm_min = (int)(rem % 3600 / 60); g_tm.tm_sec = (int)(rem % 60);
    g_tm.tm_wday = (int)((days + 4) % 7);
    long z = days + 719468, era = z / 146097, doe = z - era * 146097, yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    long y = yoe + era * 400, doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    g_tm.tm_mday = (int)(doy - (153 * mp + 2) / 5 + 1); long m = mp < 10 ? mp + 3 : mp - 9; if (m <= 2) y++;
    g_tm.tm_mon = (int)m - 1; g_tm.tm_year = (int)y - 1900; g_tm.tm_yday = 0; g_tm.tm_isdst = 0; return &g_tm;
}
struct tm* localtime(const time_t* t) { return gmtime(t); }
time_t mktime(struct tm* tm) { (void)tm; return -1; }
size_t strftime(char* s, size_t n, const char* f, const struct tm* t) { (void)f; return (size_t)snprintf(s, n, "%04d-%02d-%02d %02d:%02d:%02d", t->tm_year + 1900, t->tm_mon + 1, t->tm_mday, t->tm_hour, t->tm_min, t->tm_sec); }

int stat(const char* p, struct stat* s) { (void)p; (void)s; errno = ENOENT; return -1; }
int mkdir(const char* p, unsigned m) { (void)p; (void)m; return -1; }
int rmdir(const char* p) { (void)p; return -1; }
int chdir(const char* p) { (void)p; return -1; }
char* getcwd(char* b, size_t n) { if (n > 1) { b[0] = '/'; b[1] = 0; return b; } return 0; }
int isatty(int fd) { (void)fd; return 1; }
ssize_t read(int fd, void* b, size_t n) { (void)fd; (void)b; (void)n; return -1; }
ssize_t write(int fd, const void* b, size_t n) { if (fd == 1 || fd == 2) { put_str(b, n); return (ssize_t)n; } return -1; }
int close(int fd) { (void)fd; return -1; }
#ifdef DGN_USER

struct DgnBmDir { char* buf; long len, pos; struct dirent ent; };
DIR* opendir(const char* p) {
    long sz = dgn_sys(15, (long)p, 0, 0, 0);
    if (sz < 0) return 0;
    DIR* d = calloc(1, sizeof *d); if (!d) return 0;
    d->buf = malloc((size_t)sz + 1); if (!d->buf) { free(d); return 0; }
    if (sz > 0) dgn_sys(15, (long)p, (long)d->buf, sz, 0);
    d->buf[sz] = 0; d->len = sz;
    return d;
}
struct dirent* readdir(DIR* d) {
    if (!d || d->pos >= d->len) return 0;
    int i = 0;
    while (d->pos < d->len && d->buf[d->pos] != '\n') { if (i < 255) d->ent.d_name[i++] = d->buf[d->pos]; d->pos++; }
    d->pos++;
    if (i > 0 && d->ent.d_name[i - 1] == '/') i--;
    d->ent.d_name[i] = 0;
    return &d->ent;
}
int closedir(DIR* d) { if (d) { free(d->buf); free(d); } return 0; }
#else
DIR* opendir(const char* p) { (void)p; return 0; }
struct dirent* readdir(DIR* d) { (void)d; return 0; }
int closedir(DIR* d) { (void)d; return -1; }
#endif
int pthread_create(pthread_t* t, const void* a, void* (*fn)(void*), void* arg) { (void)t; (void)a; (void)fn; (void)arg; return 11; }
int pthread_detach(pthread_t t) { (void)t; return 0; }
int pthread_join(pthread_t t, void** r) { (void)t; (void)r; return 0; }
int pthread_mutex_init(pthread_mutex_t* m, const void* a) { (void)m; (void)a; return 0; }
int pthread_mutex_lock(pthread_mutex_t* m) { (void)m; return 0; }
int pthread_mutex_unlock(pthread_mutex_t* m) { (void)m; return 0; }
int pthread_mutex_destroy(pthread_mutex_t* m) { (void)m; return 0; }
int socket(int a, int b, int c) { (void)a; (void)b; (void)c; return -1; }
int connect(int s, const struct sockaddr* a, socklen_t l) { (void)s; (void)a; (void)l; return -1; }
long send(int s, const void* b, size_t n, int f) { (void)s; (void)b; (void)n; (void)f; return -1; }
long recv(int s, void* b, size_t n, int f) { (void)s; (void)b; (void)n; (void)f; return -1; }
int bind(int s, const struct sockaddr* a, socklen_t l) { (void)s; (void)a; (void)l; return -1; }
int listen(int s, int n) { (void)s; (void)n; return -1; }
int accept(int s, struct sockaddr* a, socklen_t* l) { (void)s; (void)a; (void)l; return -1; }
int setsockopt(int s, int a, int b, const void* v, socklen_t l) { (void)s; (void)a; (void)b; (void)v; (void)l; return -1; }
unsigned short htons(unsigned short v) { return (unsigned short)((v << 8) | (v >> 8)); }
unsigned short ntohs(unsigned short v) { return htons(v); }
unsigned htonl(unsigned v) { return __builtin_bswap32(v); }
unsigned inet_addr(const char* s) { (void)s; return 0xffffffffu; }
struct hostent* gethostbyname(const char* n) { (void)n; return 0; }
int getaddrinfo(const char* a, const char* b, const struct addrinfo* c, struct addrinfo** d) { (void)a; (void)b; (void)c; (void)d; return -1; }
void freeaddrinfo(struct addrinfo* a) { (void)a; }
void* dlopen(const char* p, int f) { (void)p; (void)f; return 0; }
void* dlsym(void* h, const char* n) { (void)h; (void)n; return 0; }
int dlclose(void* h) { (void)h; return 0; }
char* dlerror(void) { return "dynamic loading is not available on bare metal"; }
int regcomp(regex_t* r, const char* p, int f) { (void)r; (void)p; (void)f; return 1; }
int regexec(const regex_t* r, const char* s, size_t n, regmatch_t* m, int f) { (void)r; (void)s; (void)n; (void)m; (void)f; return REG_NOMATCH; }
void regfree(regex_t* r) { (void)r; }
