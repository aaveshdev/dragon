#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>
#include <setjmp.h>

typedef struct {
    uint8_t fx[512];
    uint64_t r15, r14, r13, r12, r11, r10, r9, r8, rdi, rsi, rbp, rbx, rdx, rcx, rax;
    uint64_t vec, err, rip, cs, rflags, rsp, ss;
} Frame;

extern uint64_t pml4[];
extern void dgn_x86_resume(Frame* f);
extern void* dgn_sched_tick(void* f);
extern void dgn_bm_crit_enter(void);
extern void dgn_bm_crit_exit(void);

#define MAXP 256
#define MAXVMA 8
#define FRAME_CHUNK (4u << 20)
#define PTE_FOREIGN 0x200ULL
#define PTE_COW 0x400ULL
#define PTE_ADDR 0x000FFFFFFFFFF000ULL
extern uint64_t dgn_nx_mask;
enum { R_NONE = 0, R_SYSCALL = 1, R_TIMER = 2, R_FAULT = 3 };

typedef struct { uint64_t lo, hi; } Vma;
typedef struct {
    Frame frame __attribute__((aligned(16)));
    jmp_buf kjb;
    int used, as_owner, refs, zombie;
    uint64_t tid;
    uint64_t base, size;
    uint64_t* pm4;
    Vma vma[MAXVMA];
    int nvma;
    uint64_t npages;
    uint64_t fault_addr;
    uint64_t fsbase, gsbase;
    uint64_t lpid;
    int last_reason;
    Vma xr[16];
    int nxr;
} Proc;

static Proc procs[MAXP];
static Proc* running;
static uint8_t kstack[16384] __attribute__((aligned(16)));

extern char* dgn_bm_ram_end(void);
extern char* dgn_bm_heap_start(void);
static void* free_frames;
static uint64_t arena_bytes, arena_limit, frames_used;
static uint8_t* frame_refs;
static uint64_t refs_lo, refs_n;
static inline uint8_t* ref_of(void* f) {
    uint64_t i = ((uint64_t)(uintptr_t)f - refs_lo) >> 12;
    return i < refs_n ? &frame_refs[i] : 0;
}

static inline uint64_t irq_save(void) { uint64_t f; __asm__ volatile("pushfq; popq %0; cli" : "=r"(f) :: "memory"); return f; }
static inline void irq_restore(uint64_t f) { if (f & 0x200) __asm__ volatile("sti" ::: "memory"); }
static void* frame_alloc_i(void) {
    if (!frame_refs) {
        refs_lo = (uint64_t)(uintptr_t)dgn_bm_heap_start() & ~0xFFFULL;
        uint64_t hi = ((uint64_t)(uintptr_t)dgn_bm_ram_end() + 4095) & ~0xFFFULL;
        refs_n = (hi - refs_lo) >> 12;
        frame_refs = calloc(refs_n, 1);
    }
    if (!free_frames) {
        if (!arena_limit) {
            uint64_t span = (uint64_t)(uintptr_t)dgn_bm_ram_end() - (uint64_t)(uintptr_t)dgn_bm_heap_start();
            arena_limit = span / 10 * 6;
        }
        if (arena_bytes + FRAME_CHUNK > arena_limit) return 0;
        char* raw = malloc(FRAME_CHUNK + 4096);
        if (!raw) return 0;
        char* a = (char*)(((uintptr_t)raw + 4095) & ~(uintptr_t)4095);
        for (uint32_t off = 0; off < FRAME_CHUNK; off += 4096) {
            *(void**)(a + off) = free_frames;
            free_frames = a + off;
        }
        arena_bytes += FRAME_CHUNK;
    }
    void* f = free_frames;
    free_frames = *(void**)f;
    memset(f, 0, 4096);
    frames_used++;
    uint8_t* rc = ref_of(f);
    if (rc) *rc = 1;
    return f;
}
static void* frame_alloc(void) { uint64_t fl = irq_save(); void* r = frame_alloc_i(); irq_restore(fl); return r; }
static void frame_free(void* f) {
    uint64_t fl = irq_save();
    uint8_t* rc = ref_of(f);
    if (rc && *rc > 1) { (*rc)--; irq_restore(fl); return; }
    if (rc) *rc = 0;
    *(void**)f = free_frames;
    free_frames = f;
    frames_used--;
    irq_restore(fl);
}
uint64_t dgn_proc_frames_used(void) { return frames_used; }
uint64_t dgn_proc_frames_total(void) { return arena_bytes / 4096; }

static uint64_t gdt[8] __attribute__((aligned(16)));
typedef struct { uint32_t r0; uint64_t rsp0; uint64_t rsp1, rsp2; uint64_t r1; uint64_t ist[7]; uint64_t r2; uint16_t r3, iomap; } __attribute__((packed)) Tss;
static Tss tss;
#define DGN_MAXCPU 16
static uint64_t ap_gdt[DGN_MAXCPU][8] __attribute__((aligned(16)));
static Tss ap_tss[DGN_MAXCPU];

void dgn_x86_ap_gdt_init(int cpu, uint64_t rsp0) {
    uint64_t* g = ap_gdt[cpu];
    g[0] = 0; g[1] = 0x00AF9A000000FFFFULL; g[2] = 0x00CF92000000FFFFULL; g[3] = 0x00CFF2000000FFFFULL; g[4] = 0x00AFFA000000FFFFULL;
    Tss* t = &ap_tss[cpu];
    t->rsp0 = rsp0; t->iomap = sizeof(Tss);
    uint64_t b = (uint64_t)(uintptr_t)t, lim = sizeof(Tss) - 1;
    g[5] = (lim & 0xFFFF) | ((b & 0xFFFFFF) << 16) | (0x89ULL << 40) | (((lim >> 16) & 0xF) << 48) | (((b >> 24) & 0xFF) << 56);
    g[6] = b >> 32;
    struct { uint16_t limit; uint64_t base; } __attribute__((packed)) d = { sizeof ap_gdt[0] - 1, (uint64_t)(uintptr_t)g };
    __asm__ volatile("lgdt %0" :: "m"(d));
    __asm__ volatile("ltr %%ax" :: "a"((uint16_t)0x28));
}

void dgn_proc_mark_ctx(void (*mark)(void*)) {
    Proc* p = running;
    if (!p) return;
    uint64_t* w = (uint64_t*)&p->kjb;
    for (size_t i = 0; i < sizeof p->kjb / 8; i++) mark((void*)w[i]);
}

void dgn_x86_gdt_init(void) {
    gdt[0] = 0;
    gdt[1] = 0x00AF9A000000FFFFULL;
    gdt[2] = 0x00CF92000000FFFFULL;
    gdt[3] = 0x00CFF2000000FFFFULL;
    gdt[4] = 0x00AFFA000000FFFFULL;
    uint64_t b = (uint64_t)(uintptr_t)&tss, lim = sizeof tss - 1;
    gdt[5] = (lim & 0xFFFF) | ((b & 0xFFFFFF) << 16) | (0x89ULL << 40) | (((lim >> 16) & 0xF) << 48) | (((b >> 24) & 0xFF) << 56);
    gdt[6] = b >> 32;
    tss.rsp0 = (uint64_t)(uintptr_t)kstack + sizeof kstack;
    tss.iomap = sizeof tss;
    struct { uint16_t limit; uint64_t base; } __attribute__((packed)) d = { sizeof gdt - 1, (uint64_t)(uintptr_t)gdt };
    __asm__ volatile("lgdt %0" :: "m"(d));
    __asm__ volatile("ltr %%ax" :: "a"((uint16_t)0x28));
}


static uint64_t* pt_walk(Proc* p, uint64_t va, int create) {
    uint64_t* t = p->pm4;
    for (int sh = 39; sh >= 21; sh -= 9) {
        uint64_t* e = &t[(va >> sh) & 511];
        if (!(*e & 1)) {
            if (!create) return 0;
            void* f = frame_alloc();
            if (!f) return 0;
            *e = (uint64_t)(uintptr_t)f | 7;
        }
        t = (uint64_t*)(uintptr_t)(*e & PTE_ADDR);
    }
    return &t[(va >> 12) & 511];
}

static int in_vma(Proc* p, uint64_t va) {
    for (int i = 0; i < p->nvma; i++) if (va >= p->vma[i].lo && va < p->vma[i].hi) return 1;
    return 0;
}

static int in_xr(Proc* p, uint64_t va) {
    Proc* o = &procs[p->as_owner];
    for (int i = 0; i < o->nxr; i++) if (va >= o->xr[i].lo && va < o->xr[i].hi) return 1;
    return 0;
}
static uint64_t pte_new(Proc* p, uint64_t va) { return 7 | (in_xr(p, va) ? 0 : dgn_nx_mask); }

static char* upage(Proc* p, uint64_t va, int alloc) {
    uint64_t* e = pt_walk(p, va, 0);
    if (e && (*e & 1)) return (char*)(uintptr_t)(*e & PTE_ADDR);
    if (!alloc || !in_vma(p, va)) return 0;
    char* f = frame_alloc();
    if (!f) return 0;
    e = pt_walk(p, va, 1);
    if (!e) { frame_free(f); return 0; }
    *e = (uint64_t)(uintptr_t)f | pte_new(p, va);
    procs[p->as_owner].npages++;
    return f;
}

static int cow_break(Proc* p, uint64_t va) {
    uint64_t* e = pt_walk(p, va, 0);
    if (!e || !(*e & 1) || !(*e & PTE_COW)) return 0;
    char* f = (char*)(uintptr_t)(*e & PTE_ADDR);
    uint8_t* rc = ref_of(f);
    if (!rc || *rc <= 1) {
        *e = (*e | 2) & ~PTE_COW;
    } else {
        char* nf = frame_alloc();
        if (!nf) return 0;
        memcpy(nf, f, 4096);
        (*rc)--;
        *e = (uint64_t)(uintptr_t)nf | 7 | (*e & dgn_nx_mask);
    }
    __asm__ volatile("invlpg (%0)" :: "r"(va & ~0xFFFULL) : "memory");
    return 1;
}

static char* uwpage(Proc* p, uint64_t va) {
    uint64_t* e = pt_walk(p, va, 0);
    if (e && (*e & 1) && (*e & PTE_COW)) { if (!cow_break(p, va)) return 0; }
    return upage(p, va, 1);
}

static void free_tree(Proc* p) {
    uint64_t* l4 = p->pm4;
    if (!l4) return;
    for (int i4 = 1; i4 < 512; i4++) {
        if (!(l4[i4] & 1)) continue;
        uint64_t* l3 = (uint64_t*)(uintptr_t)(l4[i4] & PTE_ADDR);
        for (int i3 = 0; i3 < 512; i3++) {
            if (!(l3[i3] & 1)) continue;
            uint64_t* l2 = (uint64_t*)(uintptr_t)(l3[i3] & PTE_ADDR);
            for (int i2 = 0; i2 < 512; i2++) {
                if (!(l2[i2] & 1)) continue;
                uint64_t* l1 = (uint64_t*)(uintptr_t)(l2[i2] & PTE_ADDR);
                for (int i1 = 0; i1 < 512; i1++)
                    if ((l1[i1] & 1) && !(l1[i1] & PTE_FOREIGN)) frame_free((void*)(uintptr_t)(l1[i1] & PTE_ADDR));
                frame_free(l1);
            }
            frame_free(l2);
        }
        frame_free(l3);
    }
    frame_free(l4);
    p->pm4 = 0;
}

int dgn_proc_create(uint64_t base, uint64_t size) {
    if ((base & 0xFFF) || (size & 0xFFF) || size == 0 || base < 0x8000000000ULL || base + size > 0x7F0000000000ULL) return -1;
    int id = -1;
    for (int i = 0; i < MAXP; i++) if (!procs[i].used) { id = i; break; }
    if (id < 0) return -1;
    Proc* p = &procs[id];
    memset(p, 0, sizeof *p);
    p->pm4 = frame_alloc();
    if (!p->pm4) return -1;
    p->pm4[0] = pml4[0];
    p->base = base; p->size = size;
    p->vma[0].lo = base; p->vma[0].hi = base + size; p->nvma = 1;
    p->used = 1; p->as_owner = id; p->refs = 1;
    return id;
}

int dgn_proc_map_shared(int pid, uint64_t va, uint64_t kaddr, uint64_t n) {
    if (pid < 0 || pid >= MAXP || !procs[pid].used) return -1;
    if ((va & 0xFFF) || (kaddr & 0xFFF) || va < 0x8000000000ULL || va + n > 0x7F0000000000ULL) return -1;
    Proc* p = &procs[pid];
    for (uint64_t off = 0; off < n; off += 4096) {
        uint64_t* e = pt_walk(p, va + off, 1);
        if (!e) return -1;
        if ((*e & 1) && !(*e & PTE_FOREIGN)) { frame_free((void*)(uintptr_t)(*e & PTE_ADDR)); if (procs[p->as_owner].npages) procs[p->as_owner].npages--; }
        *e = (kaddr + off) | 7 | PTE_FOREIGN | dgn_nx_mask;
    }
    return 0;
}

int dgn_proc_set_exec(int pid, uint64_t va, uint64_t n, int on) {
    if (pid < 0 || pid >= MAXP || !procs[pid].used || !dgn_nx_mask) return 0;
    Proc* p = &procs[pid];
    Proc* o = &procs[p->as_owner];
    uint64_t lo = va & ~0xFFFULL, hi = (va + n + 4095) & ~0xFFFULL;
    if (hi <= lo) return -1;
    Vma keep[17]; int nk = 0;
    for (int i = 0; i < o->nxr; i++) {
        Vma r = o->xr[i];
        if (r.hi <= lo || r.lo >= hi) { keep[nk++] = r; continue; }
        if (r.lo < lo) { keep[nk].lo = r.lo; keep[nk++].hi = lo; }
        if (r.hi > hi) { keep[nk].lo = hi; keep[nk++].hi = r.hi; }
    }
    if (on) { if (nk < 16) { keep[nk].lo = lo; keep[nk++].hi = hi; } }
    if (nk > 16) nk = 16;
    memcpy(o->xr, keep, nk * sizeof(Vma)); o->nxr = nk;
    for (uint64_t a = lo; a < hi; a += 4096) {
        uint64_t* e = pt_walk(p, a, 0);
        if (!e || !(*e & 1) || (*e & PTE_FOREIGN)) continue;
        if (on) *e &= ~dgn_nx_mask; else *e |= dgn_nx_mask;
    }
    __asm__ volatile("movq %%cr3, %%rax; movq %%rax, %%cr3" ::: "rax", "memory");
    return 0;
}

int dgn_proc_release(int pid, uint64_t va, uint64_t n) {
    if (pid < 0 || pid >= MAXP || !procs[pid].used) return -1;
    Proc* p = &procs[pid];
    if ((va & 0xFFF) || va < p->base || va + n > p->base + p->size) return -1;
    for (uint64_t off = 0; off < n; off += 4096) {
        uint64_t* e = pt_walk(p, va + off, 0);
        if (e && (*e & 1) && !(*e & PTE_FOREIGN)) {
            frame_free((void*)(uintptr_t)(*e & PTE_ADDR));
            *e = 0;
            if (procs[p->as_owner].npages) procs[p->as_owner].npages--;
        }
    }
    __asm__ volatile("movq %%cr3, %%rax; movq %%rax, %%cr3" ::: "rax", "memory");
    return 0;
}

int dgn_proc_alive(int pid);
void dgn_proc_destroy(int pid) {
    if (pid < 0 || pid >= MAXP || !procs[pid].used || procs[pid].zombie) return;
    Proc* p = &procs[pid];
    Proc* o = &procs[p->as_owner];
    if (o == p) {
        if (--p->refs > 0) { p->zombie = 1; return; }
        free_tree(p);
        memset(p, 0, sizeof *p);
        return;
    }
    memset(p, 0, sizeof *p);
    if (--o->refs <= 0 && o->zombie) { free_tree(o); memset(o, 0, sizeof *o); }
}

int dgn_proc_clone(int pid) {
    if (pid < 0 || pid >= MAXP || !dgn_proc_alive(pid)) return -1;
    Proc* src = &procs[pid];
    int id = -1;
    for (int i = 0; i < MAXP; i++) if (!procs[i].used) { id = i; break; }
    if (id < 0) return -1;
    Proc* p = &procs[id];
    memset(p, 0, sizeof *p);
    p->pm4 = src->pm4;
    p->base = src->base; p->size = src->size;
    memcpy(p->vma, src->vma, sizeof p->vma); p->nvma = src->nvma;
    p->as_owner = src->as_owner; procs[src->as_owner].refs++;
    p->used = 1;
    memcpy(&p->frame, &src->frame, sizeof p->frame);
    p->fsbase = src->fsbase; p->gsbase = src->gsbase;
    p->lpid = src->lpid;
    return id;
}

int dgn_proc_fork(int pid) {
    if (pid < 0 || pid >= MAXP || !procs[pid].used) return -1;
    Proc* src = &procs[pid];
    int id = -1;
    for (int i = 0; i < MAXP; i++) if (!procs[i].used) { id = i; break; }
    if (id < 0) return -1;
    Proc* p = &procs[id];
    memset(p, 0, sizeof *p);
    p->pm4 = frame_alloc();
    if (!p->pm4) return -1;
    p->pm4[0] = pml4[0];
    p->base = src->base; p->size = src->size;
    memcpy(p->vma, src->vma, sizeof p->vma); p->nvma = src->nvma;
    memcpy(p->xr, procs[src->as_owner].xr, sizeof p->xr); p->nxr = procs[src->as_owner].nxr;
    p->used = 1; p->as_owner = id; p->refs = 1;
    uint64_t* l4 = src->pm4;
    for (int i4 = 1; i4 < 512; i4++) {
        if (!(l4[i4] & 1)) continue;
        uint64_t* l3 = (uint64_t*)(uintptr_t)(l4[i4] & PTE_ADDR);
        for (int i3 = 0; i3 < 512; i3++) {
            if (!(l3[i3] & 1)) continue;
            uint64_t* l2 = (uint64_t*)(uintptr_t)(l3[i3] & PTE_ADDR);
            for (int i2 = 0; i2 < 512; i2++) {
                if (!(l2[i2] & 1)) continue;
                uint64_t* l1 = (uint64_t*)(uintptr_t)(l2[i2] & PTE_ADDR);
                for (int i1 = 0; i1 < 512; i1++) {
                    if (!(l1[i1] & 1)) continue;
                    uint64_t va = ((uint64_t)i4 << 39) | ((uint64_t)i3 << 30) | ((uint64_t)i2 << 21) | ((uint64_t)i1 << 12);
                    uint64_t* e = pt_walk(p, va, 1);
                    if (!e) { dgn_proc_destroy(id); return -1; }
                    if (l1[i1] & PTE_FOREIGN) { *e = l1[i1]; continue; }
                    char* sf = (char*)(uintptr_t)(l1[i1] & PTE_ADDR);
                    uint8_t* rc = ref_of(sf);
                    if (rc && *rc < 250) {
                        (*rc)++;
                        l1[i1] = (l1[i1] & ~2ULL) | PTE_COW;
                        *e = l1[i1];
                    } else {
                        char* f = frame_alloc();
                        if (!f) { dgn_proc_destroy(id); return -1; }
                        memcpy(f, sf, 4096);
                        *e = (uint64_t)(uintptr_t)f | 7 | (l1[i1] & dgn_nx_mask);
                    }
                    p->npages++;
                }
            }
        }
    }
    memcpy(&p->frame, &src->frame, sizeof p->frame);
    p->fsbase = src->fsbase; p->gsbase = src->gsbase;
    p->lpid = 0; p->tid = 0;
    __asm__ volatile("movq %%cr3, %%rax; movq %%rax, %%cr3" ::: "rax", "memory");
    return id;
}


int dgn_proc_alive(int pid) { return pid >= 0 && pid < MAXP && procs[pid].used && !procs[pid].zombie; }
uint64_t dgn_proc_base(int pid) { return dgn_proc_alive(pid) ? procs[pid].base : 0; }
uint64_t dgn_proc_size(int pid) { return dgn_proc_alive(pid) ? procs[pid].size : 0; }
uint64_t dgn_proc_mem(int pid) { return dgn_proc_alive(pid) ? procs[pid].npages * 4096 : 0; }

static int ubounds(Proc* p, uint64_t ua, uint64_t n) {
    return ua >= p->base && ua - p->base <= p->size && n <= p->size - (ua - p->base);
}
static int uread8(Proc* p, uint64_t va, uint8_t* out) {
    char* pg = upage(p, va, 0);
    if (!pg) { *out = 0; return in_vma(p, va); }
    *out = (uint8_t)pg[va & 4095];
    return 1;
}
int dgn_proc_copy(int pid, uint64_t ua, void* k, uint64_t n, int to_user) {
    if (!dgn_proc_alive(pid)) return -1;
    Proc* p = &procs[pid];
    if (!ubounds(p, ua, n)) return -1;
    char* kb = (char*)k;
    while (n) {
        uint64_t off = ua & 4095, chunk = 4096 - off;
        if (chunk > n) chunk = n;
        char* pg = to_user ? uwpage(p, ua) : upage(p, ua, 0);
        if (!pg) {
            if (to_user) return -1;
            memset(kb, 0, chunk);
        } else if (to_user) memcpy(pg + off, kb, chunk);
        else memcpy(kb, pg + off, chunk);
        ua += chunk; kb += chunk; n -= chunk;
    }
    return 0;
}

int64_t dgn_proc_strlen(int pid, uint64_t ua, uint64_t max) {
    if (!dgn_proc_alive(pid)) return -1;
    Proc* p = &procs[pid];
    for (uint64_t i = 0; i < max; i++) {
        uint8_t c;
        if (!ubounds(p, ua + i, 1) || !uread8(p, ua + i, &c)) return -1;
        if (!c) return (int64_t)i;
    }
    return -1;
}

int dgn_proc_start(int pid, uint64_t entry, uint64_t sp, uint64_t a0, uint64_t a1) {
    if (!dgn_proc_alive(pid)) return -1;
    Proc* p = &procs[pid];
    memset(&p->frame, 0, sizeof p->frame);
    *(uint16_t*)(p->frame.fx + 0) = 0x037F;
    *(uint32_t*)(p->frame.fx + 24) = 0x1F80;
    p->frame.rip = entry; p->frame.rsp = sp; p->frame.rdi = a0; p->frame.rsi = a1;
    p->frame.cs = 0x23; p->frame.ss = 0x1B; p->frame.rflags = 0x202;
    return 0;
}

uint64_t dgn_proc_reg(int pid, int i) {
    if (!dgn_proc_alive(pid)) return 0;
    Proc* p = &procs[pid];
    switch (i) {
    case 0: return p->frame.rax; case 1: return p->frame.rdi; case 2: return p->frame.rsi; case 3: return p->frame.rdx;
    case 4: return p->frame.r10; case 5: return p->frame.r8; case 6: return p->frame.r9; case 7: return p->frame.vec;
    case 8: return p->frame.err; case 9: return p->frame.rip; case 10: return p->fault_addr; case 11: return p->frame.rsp;
    case 12: return p->frame.rcx; case 13: return p->frame.r11; case 22: return p->frame.rflags; case 20: return p->fsbase; case 24: return p->gsbase; case 21: return p->lpid; case 23: return p->tid;
    case 14: return p->frame.rbx; case 15: return p->frame.rbp; case 16: return p->frame.r12; case 17: return p->frame.r13; case 18: return p->frame.r14; case 19: return p->frame.r15;
    }
    return 0;
}
void dgn_proc_set_reg(int pid, int i, uint64_t v) {
    if (!dgn_proc_alive(pid)) return;
    Proc* p = &procs[pid];
    switch (i) { case 0: p->frame.rax = v; break; case 1: p->frame.rdi = v; break; case 2: p->frame.rsi = v; break; case 3: p->frame.rdx = v; break;
                 case 4: p->frame.r10 = v; break; case 5: p->frame.r8 = v; break; case 6: p->frame.r9 = v; break;
                 case 11: p->frame.rsp = v; break; case 12: p->frame.rcx = v; break; case 13: p->frame.r11 = v; break; case 22: p->frame.rflags = (v & 0xDD5) | 0x202; break; case 9: p->frame.rip = v; break; case 20: p->fsbase = v; break; case 24: p->gsbase = v; break; case 21: p->lpid = v; break; case 23: p->tid = v; break;
                 case 14: p->frame.rbx = v; break; case 15: p->frame.rbp = v; break; case 16: p->frame.r12 = v; break;
                 case 17: p->frame.r13 = v; break; case 18: p->frame.r14 = v; break; case 19: p->frame.r15 = v; break; }
}

extern void dgn_bm_task_sleep_ms(uint64_t ms);
int dgn_proc_run(int pid) {
    if (!dgn_proc_alive(pid)) return -1;
    static int started;
    if (!started) { started = 1; dgn_bm_task_sleep_ms(0); }
    Proc* p = &procs[pid];
    int r = setjmp(p->kjb);
    if (r == 0) {
        __asm__ volatile("cli");
        running = p;
        __asm__ volatile("wrmsr" :: "c"(0xC0000100U), "a"((uint32_t)p->fsbase), "d"((uint32_t)(p->fsbase >> 32)));
        __asm__ volatile("movq %0, %%cr3" :: "r"((uint64_t)(uintptr_t)p->pm4) : "memory");
        dgn_x86_resume(&p->frame);
    }
    __asm__ volatile("sti" ::: "memory");
    return r;
}


extern unsigned long long dgn_bm_now_us(void);
static int fast_syscall(Proc* p) {
    uint64_t n = p->frame.rax, r;
    switch (n) {
    case 39: if (!p->lpid) return 0; r = p->lpid; break;
    case 186: if (!p->tid) return 0; r = p->tid; break;
    case 131: case 102: case 104: case 107: case 108: r = 0; break;
    case 218: if (!p->lpid) return 0; r = p->lpid; break;
    case 228: {
        if (p->frame.rdi == 0 || p->frame.rdi == 5) return 0;
        uint64_t us = dgn_bm_now_us(), ts = p->frame.rsi;
        if (!ubounds(p, ts, 16)) return 0;
        uint64_t v[2] = { us / 1000000, (us % 1000000) * 1000 };
        if (dgn_proc_copy((int)(p - procs), ts, v, 16, 1) != 0) return 0;
        r = 0; break;
    }
    default: return 0;
    }
    p->frame.rax = r;
    return 1;
}
void dgn_x86_user_trap(Frame* f) __attribute__((noreturn));
void dgn_x86_user_trap(Frame* f) {
    Proc* p = running;
    int reason;
    uint64_t v = f->vec;
    memcpy(&p->frame, f, sizeof *f);
    if (v == 32) { dgn_sched_tick(f); reason = R_TIMER; }
    else if (v == 0x81) reason = R_SYSCALL;
    else if (v == 14 && !(f->err & 1) && ({ uint64_t cr2; __asm__ volatile("movq %%cr2, %0" : "=r"(cr2)); in_vma(p, cr2) && upage(p, cr2 & ~0xFFFULL, 1); })) {
        dgn_x86_resume(&p->frame);
        for (;;) {}
    }
    else if (v == 14 && (f->err & 3) == 3 && ({ uint64_t cr2; __asm__ volatile("movq %%cr2, %0" : "=r"(cr2)); cow_break(p, cr2); })) {
        dgn_x86_resume(&p->frame);
        for (;;) {}
    }
    else if (v == 6 && ({ uint8_t b0 = 0, b1 = 0; uread8(p, f->rip, &b0) && uread8(p, f->rip + 1, &b1) && b0 == 0x0F && b1 == 0x05; })) {
        p->frame.rcx = f->rip + 2; p->frame.r11 = f->rflags; p->frame.rip = f->rip + 2;
        if (fast_syscall(p)) { dgn_x86_resume(&p->frame); for (;;) {} }
        reason = R_SYSCALL;
    }
    else {
        reason = R_FAULT;
        uint64_t cr2; __asm__ volatile("movq %%cr2, %0" : "=r"(cr2));
        p->fault_addr = cr2;
    }
    p->last_reason = reason;
    running = 0;
    __asm__ volatile("movq %0, %%cr3" :: "r"((uint64_t)(uintptr_t)pml4) : "memory");
    longjmp(p->kjb, reason);
    for (;;) {}
}
