#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

#define VBASE 0x0a000000UL
#define VSTEP 0x200UL
extern uint32_t dgn_probe32(uintptr_t a);
#define NSLOT 32
#define R_MAGIC 0x000
#define R_VER 0x004
#define R_DEV 0x008
#define R_GFEAT 0x020
#define R_GFSEL 0x024
#define R_GPAGE 0x028
#define R_QSEL 0x030
#define R_QMAX 0x034
#define R_QNUM 0x038
#define R_QALIGN 0x03c
#define R_QPFN 0x040
#define R_QREADY 0x044
#define R_QNOTIFY 0x050
#define R_STATUS 0x070
#define R_QDESCL 0x080
#define R_QDESCH 0x084
#define R_QDRVL 0x090
#define R_QDRVH 0x094
#define R_QDEVL 0x0a0
#define R_QDEVH 0x0a4
#define R_CFG 0x100
#define QN 8

typedef struct { uint64_t addr; uint32_t len; uint16_t flags, next; } Desc;
static uintptr_t base; static Desc* desc; static volatile uint16_t* avail; static volatile uint8_t* used;
static uint16_t last; static uint64_t nsect;
static struct { uint32_t type, res; uint64_t sector; } __attribute__((aligned(16))) req;
static volatile uint8_t status;

static inline volatile uint32_t* reg(int off) { return (volatile uint32_t*)(base + off); }
static inline void fence_rw(void) { __asm__ volatile("dmb sy" ::: "memory"); }

uint64_t dgn_bm_disk_init(void) {
    if (nsect) return nsect;
    for (int s = 0; s < NSLOT; s++) {
        uintptr_t b = VBASE + VSTEP * (uintptr_t)s;
        if (dgn_probe32(b) != 0x74726976 || dgn_probe32(b + R_DEV) != 2) continue;
        base = b;
        uint32_t ver = *reg(R_VER);
        *reg(R_STATUS) = 0; *reg(R_STATUS) = 1; *reg(R_STATUS) = 3;
        if (ver >= 2) { *reg(R_GFSEL) = 1; *reg(R_GFEAT) = 1; *reg(R_GFSEL) = 0; *reg(R_GFEAT) = 0; *reg(R_STATUS) = 11; if (!(*reg(R_STATUS) & 8)) return 0; }
        else *reg(R_GFEAT) = 0;
        *reg(R_QSEL) = 0;
        uint32_t qmax = *reg(R_QMAX); if (qmax == 0) return 0;
        *reg(R_QNUM) = QN;
        char* mem = malloc(4096 * 3); if (!mem) return 0;
        uintptr_t m = ((uintptr_t)mem + 4095) & ~(uintptr_t)4095;
        memset((void*)m, 0, 8192);
        desc = (Desc*)m; avail = (volatile uint16_t*)(m + 16 * QN); used = (volatile uint8_t*)(m + 4096);
        if (ver >= 2) {
            *reg(R_QDESCL) = (uint32_t)(uintptr_t)desc; *reg(R_QDESCH) = (uint32_t)((uint64_t)(uintptr_t)desc >> 32);
            *reg(R_QDRVL) = (uint32_t)(uintptr_t)avail; *reg(R_QDRVH) = (uint32_t)((uint64_t)(uintptr_t)avail >> 32);
            *reg(R_QDEVL) = (uint32_t)(uintptr_t)used;  *reg(R_QDEVH) = (uint32_t)((uint64_t)(uintptr_t)used >> 32);
            *reg(R_QREADY) = 1; *reg(R_STATUS) = 15;
        } else {
            *reg(R_GPAGE) = 4096; *reg(R_QALIGN) = 4096; *reg(R_QPFN) = (uint32_t)((uintptr_t)m >> 12); *reg(R_STATUS) = 7;
        }
        last = 0;
        volatile uint32_t* cap = (volatile uint32_t*)(b + R_CFG);
        nsect = ((uint64_t)cap[1] << 32) | cap[0];
        return nsect;
    }
    return 0;
}

static int rw_chunk(uint64_t sector, uint8_t* buf, uint32_t n, int write) {
    req.type = write ? 1 : 0; req.res = 0; req.sector = sector; status = 0xff;
    desc[0].addr = (uint64_t)(uintptr_t)&req; desc[0].len = 16; desc[0].flags = 1; desc[0].next = 1;
    desc[1].addr = (uint64_t)(uintptr_t)buf; desc[1].len = n * 512; desc[1].flags = (write ? 0 : 2) | 1; desc[1].next = 2;
    desc[2].addr = (uint64_t)(uintptr_t)&status; desc[2].len = 1; desc[2].flags = 2; desc[2].next = 0;
    uint16_t a = avail[1];
    avail[2 + (a % QN)] = 0;
    fence_rw();
    avail[1] = (uint16_t)(a + 1);
    fence_rw();
    *reg(R_QNOTIFY) = 0;
    for (long spin = 0; spin < 200000000L; spin++) {
        fence_rw();
        if (*(volatile uint16_t*)(used + 2) != last) { last++; return status == 0 ? 0 : -1; }
    }
    return -1;
}

int dgn_bm_disk_rw(uint64_t sector, void* buf, uint32_t count, int write) {
    if (!nsect || sector + count > nsect) return -1;
    uint8_t* p = (uint8_t*)buf;
    while (count) {
        uint32_t n = count > 32 ? 32 : count;
        if (rw_chunk(sector, p, n, write) != 0) return -1;
        sector += n; p += n * 512; count -= n;
    }
    return 0;
}
