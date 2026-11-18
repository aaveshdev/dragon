#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

#define VBASE   0x10001000UL
#define VSTEP   0x1000UL
#define NSLOT   8
#define QN      64

#define R_MAGIC 0x000
#define R_VER   0x004
#define R_DEV   0x008
#define R_DFEAT 0x010
#define R_DFSEL 0x014
#define R_GFEAT 0x020
#define R_GFSEL 0x024
#define R_QSEL  0x030
#define R_QMAX  0x034
#define R_QNUM  0x038
#define R_QREADY 0x044
#define R_QNOTIFY 0x050
#define R_STATUS 0x070
#define R_QDESCL 0x080
#define R_QDESCH 0x084
#define R_QDRVL 0x090
#define R_QDRVH 0x094
#define R_QDEVL 0x0a0
#define R_QDEVH 0x0a4
#define R_CFG   0x100

typedef struct { uint64_t addr; uint32_t len; uint16_t flags, next; } Desc;
typedef struct { uint16_t type, code; uint32_t value; } Ev;
typedef struct {
    uintptr_t base; int kind;
    Desc* desc; volatile uint16_t* avail; volatile uint8_t* used; Ev* ev; uint16_t last; uint32_t qn;
} Dev;

static Dev devs[NSLOT]; static int ndev;

static inline volatile uint32_t* reg(uintptr_t b, int off) { return (volatile uint32_t*)(b + off); }
static inline void fence_rw(void) { __asm__ volatile("fence rw, rw" ::: "memory"); }

#define R_GPAGE 0x028
#define R_QALIGN 0x03c
#define R_QPFN  0x040
static int setup(Dev* d) {
    uintptr_t b = d->base;
    uint32_t ver = *reg(b, R_VER);
    *reg(b, R_STATUS) = 0;
    *reg(b, R_STATUS) = 1;
    *reg(b, R_STATUS) = 1 | 2;
    if (ver >= 2) { *reg(b, R_GFSEL) = 1; *reg(b, R_GFEAT) = 1; *reg(b, R_GFSEL) = 0; *reg(b, R_GFEAT) = 0; }
    else { *reg(b, R_GFEAT) = 0; }
    if (ver >= 2) { *reg(b, R_STATUS) = 1 | 2 | 8; if (!(*reg(b, R_STATUS) & 8)) return 0; }
    *reg(b, R_QSEL) = 0;
    uint32_t qmax = *reg(b, R_QMAX); if (qmax == 0) return 0;
    uint32_t qn = qmax < QN ? qmax : QN;
    *reg(b, R_QNUM) = qn;
    char* mem = (char*)malloc(4096 + 8192 + 4096);
    if (!mem) return 0;
    uintptr_t m = ((uintptr_t)mem + 4095) & ~(uintptr_t)4095;
    memset((void*)m, 0, 8192 + 8 * QN);
    d->desc = (Desc*)m; d->avail = (volatile uint16_t*)(m + 16 * qn); d->used = (volatile uint8_t*)(m + 4096); d->ev = (Ev*)(m + 8192);
    for (uint32_t i = 0; i < qn; i++) {
        d->desc[i].addr = (uint64_t)(uintptr_t)&d->ev[i]; d->desc[i].len = 8; d->desc[i].flags = 2; d->desc[i].next = 0;
        d->avail[2 + i] = (uint16_t)i;
    }
    fence_rw();
    d->avail[1] = (uint16_t)qn;
    if (ver >= 2) {
        *reg(b, R_QDESCL) = (uint32_t)(uintptr_t)d->desc; *reg(b, R_QDESCH) = (uint32_t)((uint64_t)(uintptr_t)d->desc >> 32);
        *reg(b, R_QDRVL) = (uint32_t)(uintptr_t)d->avail; *reg(b, R_QDRVH) = (uint32_t)((uint64_t)(uintptr_t)d->avail >> 32);
        *reg(b, R_QDEVL) = (uint32_t)(uintptr_t)d->used;  *reg(b, R_QDEVH) = (uint32_t)((uint64_t)(uintptr_t)d->used >> 32);
        *reg(b, R_QREADY) = 1;
        *reg(b, R_STATUS) = 1 | 2 | 8 | 4;
    } else {
        *reg(b, R_GPAGE) = 4096; *reg(b, R_QALIGN) = 4096;
        *reg(b, R_QPFN) = (uint32_t)((uintptr_t)m >> 12);
        *reg(b, R_STATUS) = 1 | 2 | 4;
    }
    *reg(b, R_QNOTIFY) = 0;
    d->last = 0; d->qn = qn;
    return 1;
}

static int kind_of(uintptr_t b) {
    volatile uint8_t* cfg = (volatile uint8_t*)(b + R_CFG);
    cfg[0] = 1; cfg[1] = 0;
    fence_rw();
    int n = cfg[2]; char name[64]; if (n > 63) n = 63;
    for (int i = 0; i < n; i++) name[i] = (char)cfg[8 + i];
    name[n] = 0;
    if (strstr(name, "Keyboard")) return 1;
    if (strstr(name, "Tablet")) return 3;
    if (strstr(name, "Mouse")) return 2;
    return 0;
}

int dgn_bm_input_init(void) {
    ndev = 0;
    for (int i = 0; i < NSLOT; i++) {
        uintptr_t b = VBASE + VSTEP * (uintptr_t)i;
        if (*reg(b, R_MAGIC) != 0x74726976 || *reg(b, R_DEV) != 18) continue;
        Dev* d = &devs[ndev]; memset(d, 0, sizeof *d); d->base = b;
        d->kind = kind_of(b);
        if (!d->kind) continue;
        if (setup(d)) ndev++;
    }
    return ndev;
}

int dgn_bm_input_poll(int* kind, int* type, int* code, int* value) {
    for (int i = 0; i < ndev; i++) {
        Dev* d = &devs[i];
        fence_rw();
        uint16_t idx = *(volatile uint16_t*)(d->used + 2);
        if (d->last == idx) continue;
        uint32_t id = *(volatile uint32_t*)(d->used + 4 + 8 * (d->last % d->qn));
        Ev e = d->ev[id % d->qn];
        d->last++;
        uint16_t a = d->avail[1];
        d->avail[2 + (a % d->qn)] = (uint16_t)id;
        fence_rw();
        d->avail[1] = (uint16_t)(a + 1);
        *reg(d->base, R_QNOTIFY) = 0;
        *kind = d->kind; *type = e.type; *code = e.code; *value = (int)e.value;
        return 1;
    }
    return 0;
}
