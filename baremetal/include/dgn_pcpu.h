#ifndef DGN_PCPU_H
#define DGN_PCPU_H
#include <stdint.h>
/* Per-CPU data: on x86-64 each core keeps its block address in GS base, so PC(field) is one GS-relative access. */
#define DGN_MAX_TRY 256
#define DGN_MAXCPU 16
typedef struct DgnPcpu {
    struct DgnPcpu* self;
    int cpu;
    int crit;
    int cur;
    volatile int resched;
    int try_top;
    int idle;
    int gc_stopped;
    volatile uint64_t* stop_sp;
    int release;
    int parked;
    int user_mode;
    const char* sl_p;
    uint64_t sl_n;
    void* try_stack[DGN_MAX_TRY];
} DgnPcpu;
#if defined(__x86_64__)
#define PC(f) (((__attribute__((address_space(256))) DgnPcpu*)0)->f)
#else
extern DgnPcpu dgn_pcpu[];
#define PC(f) (dgn_pcpu[0].f)
#endif
extern DgnPcpu dgn_pcpu[DGN_MAXCPU];
#if defined(__x86_64__)
static inline void dgn_cpu_relax(void) { __asm__ volatile("pause" ::: "memory"); }
#else
static inline void dgn_cpu_relax(void) { __asm__ volatile("" ::: "memory"); }
#endif
/* spin locks: the holder cannot be descheduled or stopped for a collection (crit > 0) while it holds one */
static inline void dgn_spin_lock(volatile int* l) {
    PC(crit)++;
    while (__sync_lock_test_and_set(l, 1)) { while (*l) dgn_cpu_relax(); }
}
void dgn_bm_crit_exit(void);
static inline void dgn_spin_unlock(volatile int* l) { __sync_lock_release(l); dgn_bm_crit_exit(); }
extern volatile int dgn_stw_req;
#endif
