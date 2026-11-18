/* Extra CPU cores: read the ACPI MADT, copy a trampoline to 0x8000, wake each core with INIT/SIPI and park it in a mailbox loop. */
#include <stdint.h>
#include <stddef.h>
#include "dgn_pcpu.h"

extern unsigned long long dgn_bm_now_us(void);
extern uint64_t dgn_rsdp_addr;
extern char pml4[];

#define MAXCPU 16
#define TRAMP 0x8000UL
#define LAPIC_DEF 0xFEE00000UL

static uint8_t ap_stack[MAXCPU][32768] __attribute__((aligned(4096)));

typedef struct { volatile uint64_t fn, arg, ret; volatile uint32_t busy, part; volatile uint32_t parts, pad; uint64_t pad2[2]; } Mail;
static Mail mail[MAXCPU] __attribute__((aligned(64)));
static uint8_t cpu_apic[MAXCPU];
static int ncpu = 1, nonline = 1, inited;
static volatile uint32_t online;
static uint64_t lapic = LAPIC_DEF;

static inline void pause_(void) { __asm__ volatile("pause" ::: "memory"); }
static void udelay(unsigned us) { unsigned long long t = dgn_bm_now_us() + us; while (dgn_bm_now_us() < t) pause_(); }
static uint32_t lr(uint32_t r) { return *(volatile uint32_t*)(lapic + r); }
static void lw(uint32_t r, uint32_t v) { *(volatile uint32_t*)(lapic + r) = v; }
static uint32_t my_apic(void) { return lr(0x20) >> 24; }

/* ---- jobs ---- */
static uint64_t job_hash(uint64_t n) {
    uint64_t h = 1469598103934665603ULL;
    for (uint64_t i = 0; i < n; i++) { h ^= i; h *= 1099511628211ULL; }
    return h;
}
static uint64_t run_job(uint64_t id, uint64_t arg) {
    if (id == 1) return job_hash(arg);
    if (id == 2) return my_apic();
    return 0;
}

#define WAKE_VEC 0xFD
#define TIMER_VEC 0xEF
static uint32_t lapic_10ms;
extern void dgn_x86_load_idt(void);
extern void dgn_x86_ap_gdt_init(int cpu, uint64_t rsp0);
extern void dgn_sched_ap_join(int cpu);
extern void dgn_sched_ensure(void);
static void wake(int cpu);
void dgn_smp_eoi(void) { lw(0xB0, 0); }

static void lapic_timer_start(void) {
    lw(0x3E0, 0x3);
    lw(0x320, (1u << 17) | TIMER_VEC);
    lw(0x380, lapic_10ms ? lapic_10ms : 1000000);
}

void dgn_smp_ap_main(uint32_t idx) {
    DgnPcpu* pc = &dgn_pcpu[idx];
    pc->self = pc; pc->cpu = (int)idx; pc->release = -1;
    __asm__ volatile("wrmsr" :: "c"(0xC0000101U), "a"((uint32_t)(uintptr_t)pc), "d"((uint32_t)((uintptr_t)pc >> 32)));
    dgn_x86_ap_gdt_init((int)idx, (uint64_t)(uintptr_t)(ap_stack[idx] + sizeof ap_stack[idx] - 16));
    __asm__ volatile("pushq $0x08\n\tleaq 1f(%%rip), %%rax\n\tpushq %%rax\n\tlretq\n1:\n\tmovw $0x10, %%ax\n\tmovw %%ax, %%ds\n\tmovw %%ax, %%es\n\tmovw %%ax, %%ss\n" ::: "rax", "memory");
    dgn_x86_load_idt();
    lw(0xF0, 0x1FF);
    dgn_sched_ap_join((int)idx);
    lapic_timer_start();
    __sync_fetch_and_add(&online, 1);
    Mail* m = &mail[idx];
    for (;;) {
        if (m->busy) {
            PC(crit)++;
            if (m->fn >= 0x10000) { ((void (*)(void*, int, int))(uintptr_t)m->fn)((void*)(uintptr_t)m->arg, (int)m->part, (int)m->parts); m->ret = 0; }
            else m->ret = run_job(m->fn, m->arg);
            __sync_synchronize();
            m->busy = 0;
            dgn_bm_crit_exit();
        } else {
            __asm__ volatile("sti; hlt" ::: "memory");
        }
    }
}

/* ---- trampoline (assembled into .rodata, copied to 0x8000) ---- */
#define OFF(sym) "(" #sym " - smp_tramp_start + 0x8000)"
__asm__(
    ".section .rodata\n.balign 16\n.globl smp_tramp_start\n.globl smp_tramp_end\nsmp_tramp_start:\n"
    ".code16\n"
    "cli\ncld\n"
    "xorw %ax, %ax\nmovw %ax, %ds\n"
    "lgdtl " OFF(smp_tg_desc) "\n"
    "movl %cr0, %eax\norl $1, %eax\nmovl %eax, %cr0\n"
    "ljmpl $0x08, $" OFF(smp_pm) "\n"
    ".code32\nsmp_pm:\n"
    "movw $0x10, %ax\nmovw %ax, %ds\nmovw %ax, %es\nmovw %ax, %ss\nmovw %ax, %fs\nmovw %ax, %gs\n"
    "movl %cr4, %eax\norl $0x20, %eax\nmovl %eax, %cr4\n"
    "movl " OFF(smp_p_cr3) ", %eax\nmovl %eax, %cr3\n"
    "movl $0xC0000080, %ecx\nrdmsr\norl $0x100, %eax\nwrmsr\n"
    "movl %cr0, %eax\norl $0x80000001, %eax\nmovl %eax, %cr0\n"
    "ljmpl $0x18, $" OFF(smp_lm) "\n"
    ".code64\nsmp_lm:\n"
    "movw $0x10, %ax\nmovw %ax, %ds\nmovw %ax, %es\nmovw %ax, %ss\nxorl %eax, %eax\nmovw %ax, %fs\nmovw %ax, %gs\n"
    "movl $" OFF(smp_param) ", %ebx\n"
    "movq (%rbx), %rsp\n"
    "movq %cr0, %rax\nandq $-5, %rax\norq $2, %rax\nmovq %rax, %cr0\n"
    "movq %cr4, %rax\norq $0x600, %rax\nmovq %rax, %cr4\n"
    "movl 24(%rbx), %edi\n"
    "movq 8(%rbx), %rax\n"
    "call *%rax\n"
    "1: hlt\njmp 1b\n"
    ".balign 16\nsmp_tg:\n"
    ".quad 0\n.quad 0x00CF9A000000FFFF\n.quad 0x00CF92000000FFFF\n.quad 0x00AF9A000000FFFF\n"
    "smp_tg_desc:\n.word 31\n.long " OFF(smp_tg) "\n"
    ".balign 16\nsmp_param:\n.quad 0\n.quad 0\n.quad 0\n.long 0\n.long 0\n"
    "smp_p_cr3:\n.long 0\n.long 0\n"
    "smp_tramp_end:\n"
    ".text\n"
);
extern const uint8_t smp_tramp_start[], smp_tramp_end[];

/* ---- ACPI ---- */
static int sig_eq(const uint8_t* p, const char* s) { for (int i = 0; i < 4; i++) if (p[i] != (uint8_t)s[i]) return 0; return 1; }
static const uint8_t* find_rsdp(void) {
    if (dgn_rsdp_addr) return (const uint8_t*)(uintptr_t)dgn_rsdp_addr;
    uint32_t ebda = (uint32_t)(*(volatile uint16_t*)0x40E) << 4;
    uint32_t ranges[2][2] = { { ebda, ebda + 1024 }, { 0xE0000, 0x100000 } };
    for (int r = 0; r < 2; r++) {
        if (r == 0 && (ebda < 0x80000 || ebda > 0x9FC00)) continue;
        for (uint32_t a = ranges[r][0]; a + 20 <= ranges[r][1]; a += 16) {
            const uint8_t* p = (const uint8_t*)(uintptr_t)a;
            if (p[0] == 'R' && p[1] == 'S' && p[2] == 'D' && p[3] == ' ' && p[4] == 'P' && p[5] == 'T' && p[6] == 'R' && p[7] == ' ') {
                uint8_t s = 0; for (int i = 0; i < 20; i++) s += p[i];
                if (s == 0) return p;
            }
        }
    }
    return 0;
}
static const uint8_t* find_table(const uint8_t* rsdp, const char* sig) {
    uint8_t rev = rsdp[15];
    uint64_t root; int w;
    if (rev >= 2) { root = *(const uint64_t*)(rsdp + 24); w = 8; } else { root = *(const uint32_t*)(rsdp + 16); w = 4; }
    if (!root || root > 0xFFFFF000ULL) return 0;
    const uint8_t* t = (const uint8_t*)(uintptr_t)root;
    uint32_t len = *(const uint32_t*)(t + 4);
    for (uint32_t o = 36; o + w <= len; o += w) {
        uint64_t a = w == 8 ? *(const uint64_t*)(t + o) : *(const uint32_t*)(t + o);
        if (!a || a > 0xFFFFF000ULL) continue;
        if (sig_eq((const uint8_t*)(uintptr_t)a, sig)) return (const uint8_t*)(uintptr_t)a;
    }
    return 0;
}

static int send_ipi(uint32_t apic, uint32_t lo) {
    uint64_t fl; __asm__ volatile("pushfq; popq %0; cli" : "=r"(fl) :: "memory");
    lw(0x310, apic << 24);
    lw(0x300, lo);
    int ok = 0;
    for (int i = 0; i < 100000; i++) { if (!(lr(0x300) & 0x1000)) { ok = 1; break; } pause_(); }
    if (fl & 0x200) __asm__ volatile("sti" ::: "memory");
    return ok;
}

static void wake(int cpu) { send_ipi(cpu_apic[cpu], 0x4000 | WAKE_VEC); }
void dgn_x86_ipi_all_but_self(int vec) {
    uint64_t fl; __asm__ volatile("pushfq; popq %0; cli" : "=r"(fl) :: "memory");
    lw(0x300, 0xC4000u | (uint32_t)vec);
    for (int i = 0; i < 100000; i++) { if (!(lr(0x300) & 0x1000)) break; pause_(); }
    if (fl & 0x200) __asm__ volatile("sti" ::: "memory");
}
int dgn_smp_count(void) { return nonline; }

int dgn_smp_init(void) {
    if (inited) return nonline;
    inited = 1;
    const uint8_t* rsdp = find_rsdp();
    if (!rsdp) return 1;
    const uint8_t* madt = find_table(rsdp, "APIC");
    if (!madt) return 1;
    lapic = *(const uint32_t*)(madt + 36);
    if (!lapic) lapic = LAPIC_DEF;
    uint32_t len = *(const uint32_t*)(madt + 4), bsp = my_apic();
    int found = 0;
    for (uint32_t o = 44; o + 2 <= len; ) {
        uint8_t ty = madt[o], l = madt[o + 1];
        if (l < 2) break;
        if (ty == 0 && l >= 8 && (*(const uint32_t*)(madt + o + 4) & 1)) {
            uint8_t id = madt[o + 3];
            if (id != bsp && found < MAXCPU - 1) cpu_apic[++found] = id;
        }
        o += l;
    }
    if (!found) return 1;
    uint32_t svr = lr(0xF0);
    if (!(svr & 0x100)) lw(0xF0, svr | 0x1FF);
    {
        lw(0x3E0, 0x3); lw(0x320, 0x10000 | TIMER_VEC); lw(0x380, 0xFFFFFFFFu);
        udelay(10000);
        uint32_t left = lr(0x390);
        lw(0x320, 0x10000 | TIMER_VEC);
        lapic_10ms = 0xFFFFFFFFu - left;
        if (lapic_10ms < 1000) lapic_10ms = 1000000;
    }
    dgn_sched_ensure();
    uint8_t* low = (uint8_t*)TRAMP;
    for (size_t i = 0; i < (size_t)(smp_tramp_end - smp_tramp_start); i++) low[i] = smp_tramp_start[i];
    /* param block and cr3 slot live at known offsets: find them from the end of the blob */
    uint32_t tl = (uint32_t)(smp_tramp_end - smp_tramp_start);
    volatile uint64_t* P = (volatile uint64_t*)(low + tl - 8 - 32);
    volatile uint32_t* cr3p = (volatile uint32_t*)(low + tl - 8);
    for (int i = 1; i <= found; i++) {
        uint32_t before = online;
        *cr3p = (uint32_t)(uintptr_t)pml4;
        P[0] = (uint64_t)(uintptr_t)(ap_stack[i] + sizeof ap_stack[i] - 16);
        P[1] = (uint64_t)(uintptr_t)dgn_smp_ap_main;
        P[2] = 0;
        *(volatile uint32_t*)(P + 3) = (uint32_t)i;
        __sync_synchronize();
        send_ipi(cpu_apic[i], 0x4500);
        udelay(10000);
        send_ipi(cpu_apic[i], 0x4600 | (TRAMP >> 12));
        udelay(300);
        send_ipi(cpu_apic[i], 0x4600 | (TRAMP >> 12));
        for (int w = 0; w < 500 && online == before; w++) udelay(1000);
        if (online == before) break;
    }
    nonline = 1 + (int)online;
    ncpu = nonline;
    return nonline;
}

int dgn_smp_start(int cpu, int job, uint64_t arg) {
    if (cpu < 1 || cpu >= nonline) return -1;
    Mail* m = &mail[cpu];
    if (m->busy) return -2;
    m->fn = (uint64_t)job; m->arg = arg;
    __sync_synchronize();
    m->busy = 1;
    wake(cpu);
    return 0;
}
int dgn_smp_busy(int cpu) { return cpu >= 1 && cpu < MAXCPU ? (int)mail[cpu].busy : 0; }
uint64_t dgn_smp_result(int cpu) { return cpu >= 1 && cpu < MAXCPU ? mail[cpu].ret : 0; }

/* Run f(ctx, part, parts) on every online core and wait. Serial when one core is online or a call is already running. */
static volatile int par_lock;
int dgn_smp_parallel_on = 1;
void dgn_smp_parallel(void (*f)(void*, int, int), void* ctx) {
    int n = nonline;
    if (n < 2 || !dgn_smp_parallel_on || __sync_lock_test_and_set(&par_lock, 1)) { f(ctx, 0, 1); return; }
    for (int i = 1; i < n; i++) {
        Mail* m = &mail[i];
        m->fn = (uint64_t)(uintptr_t)f; m->arg = (uint64_t)(uintptr_t)ctx; m->part = (uint32_t)i; m->parts = (uint32_t)n;
        __sync_synchronize();
        m->busy = 1;
        wake(i);
    }
    f(ctx, 0, n);
    for (int i = 1; i < n; i++) while (mail[i].busy) pause_();
    __sync_lock_release(&par_lock);
}
void dgn_smp_set_parallel(int on) { dgn_smp_parallel_on = on; }
