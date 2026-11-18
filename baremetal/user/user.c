#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

uint64_t dgn_user_top, dgn_user_stack, dgn_user_args;

enum { SYS_exit = 0, SYS_write = 1, SYS_read = 2, SYS_time_us = 3, SYS_yield = 4, SYS_getpid = 5, SYS_sleep_ms = 6 };

static inline long sys4(long n, long a, long b, long c, long d) {
    long r;
    register long r10 __asm__("r10") = d;
    __asm__ volatile("int $0x81" : "=a"(r) : "a"(n), "D"(a), "S"(b), "d"(c), "r"(r10) : "memory", "rcx", "r11");
    return r;
}
long dgn_sys(long n, long a, long b, long c, long d) { return sys4(n, a, b, c, d); }

void dgn_bm_putc(char c) { sys4(SYS_write, 1, (long)&c, 1, 0); }
int  dgn_bm_getc_nb(void) { unsigned char c; long r = sys4(SYS_read, 0, (long)&c, 1, 1); return r == 1 ? c : -1; }
int  dgn_bm_getc(void) { unsigned char c; for (;;) { long r = sys4(SYS_read, 0, (long)&c, 1, 0); if (r == 1) return c; if (r < 0) return -1; sys4(SYS_yield, 0, 0, 0, 0); } }
unsigned long long dgn_bm_now_us(void) { return (unsigned long long)sys4(SYS_time_us, 0, 0, 0, 0); }
void dgn_bm_halt(int code) { for (;;) sys4(SYS_exit, code, 0, 0, 0); }
void dgn_bm_crit_enter(void) {}
void dgn_bm_crit_exit(void) {}
void dgn_bm_safepoint(void) {}
volatile int dgn_bm_resched;

char* dgn_bm_ram_end(void) { return (char*)(uintptr_t)(dgn_user_top - dgn_user_stack); }

extern void* dgn_bm_boot_stack_bottom(void);
void* dgn_bm_stack_bottom(void* main_bottom) { return main_bottom; }
void dgn_bm_scan_stacks(void (*mark)(void*)) { (void)mark; }
size_t dgn_bm_stack_free(void) { char probe; uintptr_t sp = (uintptr_t)&probe; uintptr_t lo = dgn_user_top - dgn_user_stack; return sp > lo ? sp - lo : 0; }

extern int main(int argc, char** argv);

int dgn_user_main(void) {
    static char* argv[34];
    int argc = 0;
    argv[argc++] = "dxe";
    char* a = (char*)(uintptr_t)dgn_user_args;
    if (a && *a) {
        argv[argc++] = a;
        for (char* c = a; *c && argc < 32; c++) if (*c == '\n') { *c = 0; argv[argc++] = c + 1; }
    }
    argv[argc] = 0;
    return main(argc, argv);
}

enum { SYS_win_create = 20, SYS_win_present = 21, SYS_win_event = 22, SYS_win_title = 23 };
uint32_t* dgn_bm_fb_ptr;
int dgn_bm_fb_w, dgn_bm_fb_h;
int dgn_bm_fb_init(int w, int h) {
    if (w < 16 || h < 16 || w > 1600 || h > 1200) return 0;
    long p = sys4(SYS_win_create, w, h, 0, 0);
    if (p <= 0) return 0;
    dgn_bm_fb_ptr = (uint32_t*)(uintptr_t)p; dgn_bm_fb_w = w; dgn_bm_fb_h = h;
    return 1;
}
int dgn_bm_cur_begin(int x, int y, int w, int h) { (void)x; (void)y; (void)w; (void)h; return 0; }
void dgn_bm_cur_end(int was) { (void)was; }
void dgn_bm_cur_enable(int on) { (void)on; }
void dgn_bm_cur_reset(void) {}
void dgn_win_title(const char* t) { sys4(SYS_win_title, (long)t, 0, 0, 0); }
void dgn_gfx_present_sys(void) { sys4(SYS_win_present, 0, 0, 0, 0); }
int dgn_bm_input_init(void) { return 1; }

int dgn_bm_input_event(char* name, int* a, int* b, int* c) {
    static int32_t rec[8] __attribute__((aligned(16)));
    long r = sys4(SYS_win_event, (long)rec, 0, 0, 0);
    if (r <= 0) return 0;
    *a = rec[1]; *b = rec[2]; *c = rec[3];
    const char* nm = (const char*)&rec[4];
    int i = 0; for (; i < 15 && nm[i]; i++) name[i] = nm[i]; name[i] = 0;
    return rec[0];
}
