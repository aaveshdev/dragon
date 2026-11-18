#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <stdio.h>
#include <setjmp.h>
#include "dgn_pcpu.h"

#define TICK_US        10000ULL
#define MAXT           256
#define STACK_BYTES    (64 * 1024)
#define IDLE_STACK     (8 * 1024)

enum { T_FREE = 0, T_READY, T_SLEEP, T_DEAD };
typedef struct { void* sp; char* stack; size_t size; int state; uint64_t wake; void** tryst; int try_top; int aff; volatile int on_cpu; } Task;

/* Task slots: 0 boot, 1 boot-CPU idle, 2..MAXT-1 Dragon tasks, MAXT+n idle of CPU n. */
extern char __stack_lo[];
static Task tasks[MAXT + DGN_MAXCPU];
DgnPcpu dgn_pcpu[DGN_MAXCPU] = { { .self = &dgn_pcpu[0] } };
static int sched_on;
static volatile uint64_t ticks;
static volatile int sched_lock;
volatile int dgn_bm_resched;
volatile int dgn_stw_req;
static volatile int stw_owner = -1;
static volatile int stw_stopped;
static int ncpu_online = 1;
#define IDLE_OF(c) ((c) == 0 ? 1 : MAXT + (c))
#define IS_IDLE(i) ((i) == 1 || (i) >= MAXT)

int  dgn_bm_try_cap(void);
void dgn_bm_try_save(void** dst, int* top);
void dgn_bm_try_load(void** src, int top);
void dgn_bm_halt(int code) __attribute__((noreturn));

extern void* dgn_arch_make_frame(char* stack, size_t size, void (*tramp)(void*), void* a0);
extern void  dgn_arch_yield(void);
extern void  dgn_arch_start(void);
extern void  dgn_arch_idle(void);
#define ecall_yield dgn_arch_yield

static inline void slock(void) { while (__sync_lock_test_and_set(&sched_lock, 1)) { while (sched_lock) dgn_cpu_relax(); } }
static inline void sunlock(void) { __sync_lock_release(&sched_lock); }

#if defined(__x86_64__)
extern void dgn_x86_ipi_all_but_self(int vec);
static inline uint64_t save_if(void) { uint64_t f; __asm__ volatile("pushfq; popq %0; cli" : "=r"(f) :: "memory"); return f; }
static inline void restore_if(uint64_t f) { if (f & 0x200) __asm__ volatile("sti" ::: "memory"); }
#else
static inline uint64_t save_if(void) { return 0; }
static inline void restore_if(uint64_t f) { (void)f; }
static void dgn_x86_ipi_all_but_self(int vec) { (void)vec; }
#endif

/* ---- stop the world (garbage collection) ---- */
static void park_here(volatile uint64_t* sp) {
    PC(stop_sp) = sp;
    __sync_synchronize();
    PC(parked) = 1;
    __sync_fetch_and_add(&stw_stopped, 1);
    while (dgn_stw_req) dgn_cpu_relax();
    __sync_fetch_and_sub(&stw_stopped, 1);
    PC(parked) = 0;
}
void dgn_bm_stw_park_frame(void* f, int from_user) {
    if (dgn_stw_req && PC(crit) == 0 && stw_owner != PC(cpu)) { PC(user_mode) = from_user; park_here((volatile uint64_t*)f); }
}
static void park_coop(void) {
    jmp_buf jb;
    setjmp(jb);
    PC(user_mode) = 0;
    park_here((volatile uint64_t*)&jb);
}
void dgn_bm_crit_enter(void) { PC(crit)++; }
void dgn_bm_crit_exit(void) {
    if (--PC(crit) == 0 && dgn_stw_req && stw_owner != PC(cpu)) park_coop();
}
int dgn_bm_stw_begin(void) {
    if (ncpu_online <= 1) { PC(crit)++; return 1; }
    jmp_buf jb;
    setjmp(jb);
    if (!__sync_bool_compare_and_swap(&stw_owner, -1, PC(cpu))) {
        /* another CPU is collecting: park until it is done */
        while (stw_owner != -1) {
            if (dgn_stw_req && !PC(parked)) { PC(user_mode) = 0; park_here((volatile uint64_t*)&jb); }
            dgn_cpu_relax();
        }
        return 0;
    }
    PC(crit)++;
    dgn_stw_req = 1;
    __sync_synchronize();
    dgn_x86_ipi_all_but_self(0xFC);
    while (stw_stopped < ncpu_online - 1) dgn_cpu_relax();
    return 1;
}
void dgn_bm_stw_end(void) {
    if (ncpu_online > 1) {
        dgn_stw_req = 0;
        __sync_synchronize();
        while (stw_stopped > 0) dgn_cpu_relax();
        stw_owner = -1;
    }
    PC(crit)--;
}

void dgn_bm_safepoint(void) { PC(resched) = 0; if (PC(crit) == 0 && sched_on) ecall_yield(); }

uint64_t dgn_bm_ticks(void) { return ticks; }

static void idle_main(void* unused) { (void)unused; dgn_arch_idle(); }

static void sched_init(void) {
    if (sched_on) return;
    int cap = dgn_bm_try_cap();
    memset(tasks, 0, sizeof tasks);
    for (int i = 0; i < MAXT + DGN_MAXCPU; i++) tasks[i].on_cpu = -1;
    tasks[0].state = T_READY; tasks[0].tryst = malloc(sizeof(void*) * cap); tasks[0].on_cpu = 0;
    char* st = malloc(IDLE_STACK);
    tasks[1].state = T_READY; tasks[1].stack = st; tasks[1].size = IDLE_STACK; tasks[1].tryst = malloc(sizeof(void*) * cap); tasks[1].on_cpu = 0;
    tasks[1].sp = dgn_arch_make_frame(st, IDLE_STACK, idle_main, 0);
    PC(cur) = 0; PC(release) = -1; sched_on = 1;
    dgn_arch_start();
}

void dgn_sched_ensure(void) { sched_init(); }
int dgn_bm_stw_pending(void) { return dgn_stw_req; }

static int pick(int c, int from) {
    for (int k = 1; k <= MAXT; k++) {
        int i = (from + k) % MAXT;
        if (i == 1) continue;
        Task* t = &tasks[i];
        if (t->state == T_READY && t->on_cpu < 0 && (t->aff || c == 0)) return i;
    }
    return -1;
}

void* dgn_sched_switch(void* f) {
    int c = PC(cpu), old = PC(cur);
    slock();
    Task* o = &tasks[old];
    o->sp = f;
    dgn_bm_try_save(o->tryst, &o->try_top);
    for (int i = 0; i < MAXT; i++) if (tasks[i].state == T_SLEEP && tasks[i].wake <= ticks) tasks[i].state = T_READY;
    int next = pick(c, old);
    if (next < 0) next = IDLE_OF(c);
    for (int i = 2; i < MAXT; i++) if (tasks[i].state == T_DEAD && i != old && i != next && tasks[i].on_cpu < 0) {
        free(tasks[i].stack); free(tasks[i].tryst); memset(&tasks[i], 0, sizeof tasks[i]); tasks[i].on_cpu = -1;
    }
    if (next != old) {
        tasks[next].on_cpu = c;
        PC(release) = IS_IDLE(old) ? -1 : old;
    }
    PC(cur) = next;
    dgn_bm_try_load(tasks[next].tryst, tasks[next].try_top);
    void* sp = tasks[next].sp;
    sunlock();
    return sp;
}

/* called on the new task's stack right after the switch: the old task's stack is free for other CPUs now */
void dgn_sched_post_switch(void) {
    int r = PC(release);
    if (r >= 0) { PC(release) = -1; __sync_synchronize(); tasks[r].on_cpu = -1; }
}

void* dgn_sched_tick(void* f) {
    ticks++;
    if (IS_IDLE(PC(cur)) && PC(crit) == 0) return dgn_sched_switch(f);
    PC(resched) = 1;
    return f;
}
void* dgn_sched_tick_cpu(void* f) {
    if (IS_IDLE(PC(cur)) && PC(crit) == 0) return dgn_sched_switch(f);
    PC(resched) = 1;
    return f;
}

void* dgn_sched_yield_trap(void* f) {
    if (PC(crit) > 0) return f;
    return dgn_sched_switch(f);
}
int dgn_sched_active(void) { return sched_on; }
int dgn_sched_cur(void) { return PC(cur); }

/* ---- extra CPUs ---- */
void dgn_sched_ap_join(int cpu) {
    Task* t = &tasks[IDLE_OF(cpu)];
    memset(t, 0, sizeof *t);
    t->state = T_READY; t->on_cpu = cpu;
    t->tryst = malloc(sizeof(void*) * dgn_bm_try_cap());
    PC(cur) = IDLE_OF(cpu); PC(release) = -1;
    while (!__sync_bool_compare_and_swap(&stw_owner, -1, cpu)) dgn_cpu_relax();
    __sync_fetch_and_add(&ncpu_online, 1);
    stw_owner = -1;
}
int dgn_sched_cpus(void) { return ncpu_online; }

static void task_exit(void) {
    uint64_t fl = save_if();
    slock();
    tasks[PC(cur)].state = T_DEAD;
    sunlock();
    restore_if(fl);
    for (;;) ecall_yield();
}
static void tramp(void* blk) {
    void (*entry)(void*) = *(void (**)(void*))blk;
    entry((char*)blk + 16);
    task_exit();
}
int dgn_bm_task_create_aff(void (*entry)(void*), const void* data, size_t n, size_t stack_bytes, int aff) {
    sched_init();
    int id = -1;
    dgn_bm_crit_enter();
    slock();
    for (int i = 2; i < MAXT; i++) if (tasks[i].state == T_FREE) { id = i; tasks[i].state = T_DEAD; tasks[i].on_cpu = 0; break; }
    sunlock();
    if (id < 0) { dgn_bm_crit_exit(); return -1; }
    if (stack_bytes == 0) stack_bytes = STACK_BYTES;
    if (stack_bytes < 16 * 1024) stack_bytes = 16 * 1024;
    char* st = malloc(stack_bytes);
    void** ts = malloc(sizeof(void*) * dgn_bm_try_cap());
    if (!st || !ts) { free(st); free(ts); slock(); memset(&tasks[id], 0, sizeof tasks[id]); tasks[id].on_cpu = -1; sunlock(); dgn_bm_crit_exit(); return -1; }
    char* top = (char*)(((uintptr_t)st + stack_bytes) & ~(uintptr_t)15);
    char* blk = top - ((16 + n + 15) & ~(size_t)15) - 16;
    blk = (char*)((uintptr_t)blk & ~(uintptr_t)15);
    *(void**)blk = (void*)entry; memcpy(blk + 16, data, n);
    Task* t = &tasks[id];
    void* sp = dgn_arch_make_frame(st, (size_t)(blk - st), tramp, blk);
    slock();
    t->stack = st; t->size = stack_bytes; t->tryst = ts; t->try_top = 0; t->aff = aff; t->wake = 0;
    t->sp = sp;
    t->on_cpu = -1;
    t->state = T_READY;
    sunlock();
    dgn_bm_crit_exit();
    if (aff && ncpu_online > 1) dgn_x86_ipi_all_but_self(0xFD);
    return id - 1;
}
int dgn_bm_task_create(void (*entry)(void*), const void* data, size_t n, size_t stack_bytes) {
    return dgn_bm_task_create_aff(entry, data, n, stack_bytes, 0);
}
void dgn_bm_task_yield(void) { if (!sched_on) return; ecall_yield(); }
void dgn_bm_task_sleep_ms(uint64_t ms) {
    sched_init();
    if (ms == 0) { ecall_yield(); return; }
    dgn_bm_crit_enter();
    slock();
    tasks[PC(cur)].wake = ticks + (ms * 1000ULL + TICK_US - 1) / TICK_US;
    tasks[PC(cur)].state = T_SLEEP;
    sunlock();
    dgn_bm_crit_exit();
    for (;;) { ecall_yield(); if (tasks[PC(cur)].state == T_READY) break; }
}

int dgn_bm_task_id(void) { return (!sched_on || PC(cur) == 0) ? 0 : PC(cur) - 1; }
int dgn_bm_task_alive(int id) { int i = id == 0 ? 0 : id + 1; return id >= 0 && i < MAXT && i != 1 && (tasks[i].state == T_READY || tasks[i].state == T_SLEEP); }
int dgn_bm_task_count(void) { int n = 0; for (int i = 0; i < MAXT; i++) if (i != 1 && (tasks[i].state == T_READY || tasks[i].state == T_SLEEP)) n++; return n; }

size_t dgn_bm_stack_free(void) {
    char probe; uintptr_t sp = (uintptr_t)&probe;
    uintptr_t lo = (!sched_on || PC(cur) == 0) ? (uintptr_t)__stack_lo : (uintptr_t)tasks[PC(cur)].stack;
    return sp > lo ? (size_t)(sp - lo) : 0;
}

void* dgn_bm_stack_bottom(void* main_bottom) {
    if (!sched_on || PC(cur) == 0) return main_bottom;
    return (void*)(tasks[PC(cur)].stack + tasks[PC(cur)].size);
}
void dgn_bm_scan_stacks(void (*mark)(void*)) {
    if (!sched_on) return;
    int me = PC(cur);
    for (int i = 0; i < MAXT; i++) {
        if (i == me || i == 1) continue;
        Task* t = &tasks[i];
        if (t->state != T_READY && t->state != T_SLEEP) continue;
        void** lo = (void**)t->sp;
        int oc = t->on_cpu;
        if (oc >= 0 && oc != PC(cpu)) lo = (void**)dgn_pcpu[oc].stop_sp;
        void** hi = i == 0 ? 0 : (void**)(t->stack + t->size);
        if (i == 0) { extern void* dgn_bm_boot_stack_bottom(void); hi = (void**)dgn_bm_boot_stack_bottom(); }
        if (oc >= 0 && oc != PC(cpu) && dgn_pcpu[oc].user_mode) {
            /* that CPU was in user mode: its registers are in the saved context, so scan the whole stack */
            lo = i == 0 ? (void**)__stack_lo : (void**)t->stack;
            extern void dgn_proc_mark_ctx(void (*mark)(void*));
            dgn_proc_mark_ctx(mark);
        }
        if (!lo || !hi || lo >= hi) continue;
        for (void** p = lo; p < hi; p++) mark(*p);
    }
}

#define MAXM 128
static int mutex_owner[MAXM]; static int mutex_used[MAXM];
int dgn_bm_mutex_new(void) {
    dgn_bm_crit_enter();
    for (int i = 0; i < MAXM; i++) if (!mutex_used[i]) { mutex_used[i] = 1; mutex_owner[i] = -1; dgn_bm_crit_exit(); return i; }
    dgn_bm_crit_exit(); return -1;
}
int dgn_bm_mutex_trylock(int m) {
    if (m < 0 || m >= MAXM || !mutex_used[m]) return 0;
    int me = PC(cur);
    if (__sync_bool_compare_and_swap(&mutex_owner[m], -1, me)) return 1;
    return mutex_owner[m] == me;
}
void dgn_bm_mutex_lock(int m) { while (!dgn_bm_mutex_trylock(m)) { if (m < 0 || m >= MAXM || !mutex_used[m]) return; dgn_bm_task_sleep_ms(1); } }
void dgn_bm_mutex_unlock(int m) { if (m >= 0 && m < MAXM && mutex_used[m] && mutex_owner[m] == PC(cur)) mutex_owner[m] = -1; }
