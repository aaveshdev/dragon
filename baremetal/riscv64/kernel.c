#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>

typedef struct { uint64_t x[32]; double f[32]; uint64_t mepc, mstatus; } Frame;

#define CLINT_MTIMECMP 0x02004000UL
#define CLINT_MTIME    0x0200BFF8UL
#define TICK_US        10000ULL
#define TIMEBASE_HZ    10000000ULL
#define TICK_COUNTS    (TIMEBASE_HZ / 1000000ULL * TICK_US)

extern void* dgn_sched_tick(void* f);
extern void* dgn_sched_yield_trap(void* f);
extern int dgn_sched_cur(void);
void dgn_bm_halt(int code) __attribute__((noreturn));

void dgn_arch_yield(void) { __asm__ volatile("ecall" ::: "memory"); }

static void arm_timer(void) {
    volatile uint64_t* cmp = (volatile uint64_t*)CLINT_MTIMECMP;
    *cmp = *(volatile uint64_t*)CLINT_MTIME + TICK_COUNTS;
}

void* dgn_arch_make_frame(char* stack, size_t size, void (*tramp)(void*), void* a0) {
    char* top = (char*)(((uintptr_t)stack + size) & ~(uintptr_t)15);
    Frame* f = (Frame*)(top - sizeof(Frame));
    memset(f, 0, sizeof *f);
    f->x[2] = (uint64_t)(uintptr_t)top;
    f->x[10] = (uint64_t)(uintptr_t)a0;
    f->mepc = (uint64_t)(uintptr_t)tramp;
    f->mstatus = (3ULL << 11) | (1ULL << 7) | (3ULL << 13);
    return f;
}

void dgn_arch_start(void) {
    arm_timer();
    __asm__ volatile("csrs mie, %0" :: "r"(1UL << 7));
    __asm__ volatile("csrs mstatus, %0" :: "r"(1UL << 3));
}
void dgn_arch_idle(void) { for (;;) __asm__ volatile("wfi"); }

Frame* dgn_bm_trap_c(Frame* f) {
    uint64_t cause; __asm__ volatile("csrr %0, mcause" : "=r"(cause));
    if (cause >> 63) {
        if ((cause & 0xff) == 7) { arm_timer(); return (Frame*)dgn_sched_tick(f); }
        return f;
    }
    if (cause == 11) {
        f->mepc += 4;
        return (Frame*)dgn_sched_yield_trap(f);
    }
    uint64_t tval; __asm__ volatile("csrr %0, mtval" : "=r"(tval));
    printf("\n[dragon] fatal trap: mcause=%lx mepc=%lx mtval=%lx (task %d)\n", (unsigned long)cause, (unsigned long)f->mepc, (unsigned long)tval, dgn_sched_cur());
    dgn_bm_halt(1);
}
