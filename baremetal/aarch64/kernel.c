#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>

typedef struct { uint64_t x[31]; uint64_t sp, elr, spsr, fpcr, fpsr; __uint128_t q[32]; } Frame;

extern uint64_t dgn_gicd, dgn_gicc, dgn_gicr;
extern int dgn_timer_ppi;
#define GICD dgn_gicd
#define GICC dgn_gicc
#define TIMER_PPI dgn_timer_ppi
#define TICK_US 10000ULL

extern void* dgn_sched_tick(void* f);
extern void* dgn_sched_yield_trap(void* f);
extern int dgn_sched_cur(void);
void dgn_bm_halt(int code) __attribute__((noreturn));

void dgn_arch_yield(void) { __asm__ volatile("svc #0" ::: "memory"); }

static uint64_t tick_counts;
static void arm_timer(void) {
    __asm__ volatile("msr cntp_tval_el0, %0" :: "r"(tick_counts));
    __asm__ volatile("msr cntp_ctl_el0, %0" :: "r"((uint64_t)1));
}

void* dgn_arch_make_frame(char* stack, size_t size, void (*tramp)(void*), void* a0) {
    char* top = (char*)(((uintptr_t)stack + size) & ~(uintptr_t)15);
    Frame* f = (Frame*)(top - sizeof(Frame));
    memset(f, 0, sizeof *f);
    f->sp = (uint64_t)(uintptr_t)top;
    f->x[0] = (uint64_t)(uintptr_t)a0;
    f->elr = (uint64_t)(uintptr_t)tramp;
    f->spsr = 5;
    return f;
}

#define ICC_SRE_EL1    "S3_0_C12_C12_5"
#define ICC_PMR_EL1    "S3_0_C4_C6_0"
#define ICC_IAR1_EL1   "S3_0_C12_C12_0"
#define ICC_EOIR1_EL1  "S3_0_C12_C12_1"
#define ICC_IGRPEN1    "S3_0_C12_C12_7"
static int gic3;
static uint64_t gicr_frame;

static void gic3_init(void) {
    uint64_t mpidr; __asm__ volatile("mrs %0, mpidr_el1" : "=r"(mpidr));
    uint64_t aff = (mpidr & 0xffffff) | ((mpidr >> 8) & 0xff000000ULL);
    volatile uint32_t* d = (volatile uint32_t*)GICD;
    d[0] |= (1u << 4) | (1u << 1) | 1u;
    uint64_t r = dgn_gicr, first = r, found = 0;
    for (int i = 0; i < 256; i++) {
        uint64_t typer = *(volatile uint64_t*)(r + 8);
        if ((typer >> 32) == aff) { found = r; break; }
        if (typer & 16) break;
        r += (typer & 2) ? 0x40000 : 0x20000;
    }
    gicr_frame = found ? found : first;
    volatile uint32_t* waker = (volatile uint32_t*)(gicr_frame + 0x14);
    *waker &= ~2u;
    for (int i = 0; i < 1000000 && (*waker & 4); i++) {}
    uint64_t sgi = gicr_frame + 0x10000;
    *(volatile uint32_t*)(sgi + 0x80) = 0xffffffffu;
    *(volatile uint8_t*)(sgi + 0x400 + TIMER_PPI) = 0x80;
    *(volatile uint32_t*)(sgi + 0x100) = 1u << TIMER_PPI;
    uint64_t v;
    __asm__ volatile("mrs %0, " ICC_SRE_EL1 : "=r"(v));
    __asm__ volatile("msr " ICC_SRE_EL1 ", %0" :: "r"(v | 7));
    __asm__ volatile("isb");
    __asm__ volatile("msr " ICC_PMR_EL1 ", %0" :: "r"((uint64_t)0xff));
    __asm__ volatile("msr " ICC_IGRPEN1 ", %0" :: "r"((uint64_t)1));
    __asm__ volatile("isb");
}

void dgn_arch_start(void) {
    uint64_t freq;
    __asm__ volatile("mrs %0, cntfrq_el0" : "=r"(freq));
    tick_counts = freq / 1000000ULL * TICK_US;
    uint32_t pidr2 = *(volatile uint32_t*)(GICD + 0xFE8);      /* GICv2 keeps its ID registers here (arch revision 2); a v3 distributor has them at 0xFFE8 */
    gic3 = ((pidr2 >> 4) & 15) != 2;
    if (gic3) gic3_init();
    else {
        *(volatile uint32_t*)(GICD + 0x000) = 1;
        *(volatile uint32_t*)(GICD + 0x100) = 1u << TIMER_PPI;
        *(volatile uint8_t*)(GICD + 0x400 + TIMER_PPI) = 0x80;
        *(volatile uint32_t*)(GICC + 0x004) = 0xff;
        *(volatile uint32_t*)(GICC + 0x000) = 1;
    }
    arm_timer();
    __asm__ volatile("msr daifclr, #2" ::: "memory");
}
void dgn_arch_idle(void) { for (;;) __asm__ volatile("wfi"); }

volatile int dgn_probing;
Frame* dgn_bm_sync_c(Frame* f) {
    uint64_t esr; __asm__ volatile("mrs %0, esr_el1" : "=r"(esr));
    if (dgn_probing && (esr >> 26) == 0x25) { f->elr += 4; return f; }
    if ((esr >> 26) == 0x15) return (Frame*)dgn_sched_yield_trap(f);
    uint64_t far; __asm__ volatile("mrs %0, far_el1" : "=r"(far));
    printf("\n[dragon] fatal trap: esr=%lx elr=%lx far=%lx (task %d)\n", (unsigned long)esr, (unsigned long)f->elr, (unsigned long)far, dgn_sched_cur());
    dgn_bm_halt(1);
}

Frame* dgn_bm_irq_c(Frame* f) {
    uint32_t iar;
    if (gic3) { uint64_t v; __asm__ volatile("mrs %0, " ICC_IAR1_EL1 : "=r"(v)); iar = (uint32_t)v; }
    else iar = *(volatile uint32_t*)(GICC + 0x00c);
    uint32_t id = gic3 ? (iar & 0xffffff) : (iar & 0x3ff);
    if (id == (uint32_t)TIMER_PPI) {
        arm_timer();
        if (gic3) __asm__ volatile("msr " ICC_EOIR1_EL1 ", %0" :: "r"((uint64_t)iar));
        else *(volatile uint32_t*)(GICC + 0x010) = iar;
        return (Frame*)dgn_sched_tick(f);
    }
    if (id < 1020) {
        if (gic3) __asm__ volatile("msr " ICC_EOIR1_EL1 ", %0" :: "r"((uint64_t)iar));
        else *(volatile uint32_t*)(GICC + 0x010) = iar;
    }
    return f;
}

void dgn_bm_fatal_c(Frame* f, int kind) {
    uint64_t esr; __asm__ volatile("mrs %0, esr_el1" : "=r"(esr));
    printf("\n[dragon] unexpected exception %d: esr=%lx elr=%lx\n", kind, (unsigned long)esr, (unsigned long)f->elr);
    dgn_bm_halt(1);
}
