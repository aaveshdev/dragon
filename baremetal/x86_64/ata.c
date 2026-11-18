/* ATA on the primary channel (LBA28): bus-master DMA when available, PIO otherwise. */
#include <stdint.h>
static inline uint8_t inb(uint16_t p) { uint8_t v; __asm__ volatile("inb %1, %0" : "=a"(v) : "Nd"(p)); return v; }
static inline void outb(uint16_t p, uint8_t v) { __asm__ volatile("outb %0, %1" :: "a"(v), "Nd"(p)); }
static uint16_t ata_io = 0x1F0, ata_ctl = 0x3F6;
static uint8_t ata_sel = 0xE0;
static int ata_chan;
#define IO ata_io
static inline void outl(uint16_t p, uint32_t v) { __asm__ volatile("outl %0, %1" :: "a"(v), "Nd"(p)); }
static inline uint32_t inl(uint16_t p) { uint32_t v; __asm__ volatile("inl %1, %0" : "=a"(v) : "Nd"(p)); return v; }
static uint32_t pci_rd(int b, int d, int f, int r) { outl(0xCF8, 0x80000000u | (b << 16) | (d << 11) | (f << 8) | (r & 252)); return inl(0xCFC); }
static int wait_idle(void);
static uint16_t bm_base;
static int dma_state;
static struct { uint32_t addr; uint16_t cnt; uint16_t flags; } prd[4] __attribute__((aligned(4096)));
static void dma_probe(void) {
    dma_state = -1;
    for (int d = 0; d < 32 && dma_state < 0; d++)
        for (int f = 0; f < 8; f++) {
            uint32_t id = pci_rd(0, d, f, 0);
            if ((id & 0xFFFF) == 0xFFFF) { if (f == 0) break; continue; }
            uint32_t cc = pci_rd(0, d, f, 8);
            if ((cc >> 16) != 0x0101) continue;
            uint32_t bar4 = pci_rd(0, d, f, 0x20);
            if (!(bar4 & 1)) continue;
            outl(0xCF8, 0x80000000u | (d << 11) | (f << 8) | 4);
            uint32_t cmd = inl(0xCFC);
            outl(0xCFC, cmd | 5);
            bm_base = (uint16_t)(bar4 & 0xFFFC);
            if (bm_base == 0) continue;
            dma_state = 1;
            break;
        }
}
static int dma_rw(uint64_t sector, int n, uint8_t* a, int write) {
    uint64_t pa = (uint64_t)(uintptr_t)a;
    if (pa + (uint64_t)n * 512 > 0x100000000ULL || (pa & 1)) return -1;
    int k = 0; uint64_t cur = pa, left = (uint64_t)n * 512;
    while (left) {
        uint64_t room = 0x10000 - (cur & 0xFFFF);
        uint64_t c = left < room ? left : room;
        if (k >= 4) return -1;
        prd[k].addr = (uint32_t)cur; prd[k].cnt = (uint16_t)(c == 0x10000 ? 0 : c); prd[k].flags = 0;
        k++; cur += c; left -= c;
    }
    prd[k - 1].flags = 0x8000;
    uint16_t bmb = bm_base + (ata_chan ? 8 : 0);
    outb(ata_ctl, 2);
    if (!wait_idle()) return 0;
    outb(bmb, 0);
    outl(bmb + 4, (uint32_t)(uintptr_t)prd);
    outb(bmb + 2, 0x06);
    outb(IO + 6, ata_sel | ((sector >> 24) & 15));
    outb(IO + 2, (uint8_t)n);
    outb(IO + 3, (uint8_t)sector); outb(IO + 4, (uint8_t)(sector >> 8)); outb(IO + 5, (uint8_t)(sector >> 16));
    outb(IO + 7, write ? 0xCA : 0xC8);
    outb(bmb, write ? 0x01 : 0x09);
    int ok = 0;
    for (long i = 0; i < 40000000L; i++) {
        uint8_t st = inb(bmb + 2);
        if (st & 2) { ok = 0; break; }
        if ((st & 4) || !(st & 1)) { uint8_t as = inb(IO + 7); if (!(as & 0x80)) { ok = (as & 0x21) ? 0 : 1; break; } }
    }
    outb(bmb, 0);
    outb(bmb + 2, 0x06);
    return ok;
}
static int wait_idle(void) {
    for (long i = 0; i < 20000000L; i++) { uint8_t s = inb(IO + 7); if (!(s & 0x80)) return (s & 0x21) ? 0 : 1; }
    return 0;
}
static int wait_drq(void) {
    for (long i = 0; i < 20000000L; i++) { uint8_t s = inb(IO + 7); if (s & 0x21) return 0; if (!(s & 0x80) && (s & 8)) return 1; }
    return 0;
}
int dgn_ata_rw(uint64_t sector, int count, void* addr, int write) {
    uint8_t* a = (uint8_t*)addr;
    while (count > 0) {
        int n = count > 128 ? 128 : count;
        if (dma_state == 0) dma_probe();
        if (dma_state > 0) {
            int r = dma_rw(sector, n, a, write);
            if (r == 1) { a += (uint64_t)n * 512; sector += (uint64_t)n; count -= n; continue; }
            if (r == 0) { dma_state = -2; }
        }
        if (!wait_idle()) return 0;
        outb(IO + 6, ata_sel | ((sector >> 24) & 15));
        outb(IO + 2, (uint8_t)n);
        outb(IO + 3, (uint8_t)sector); outb(IO + 4, (uint8_t)(sector >> 8)); outb(IO + 5, (uint8_t)(sector >> 16));
        outb(IO + 7, write ? 0x30 : 0x20);
        for (int s = 0; s < n; s++) {
            if (!wait_drq()) return 0;
            uint64_t cnt = 256; uint8_t* p = a;
            if (write) __asm__ volatile("rep outsw" : "+S"(p), "+c"(cnt) : "d"((uint16_t)IO) : "memory");
            else __asm__ volatile("rep insw" : "+D"(p), "+c"(cnt) : "d"((uint16_t)IO) : "memory");
            a += 512;
        }
        if (write) { outb(IO + 7, 0xE7); if (!wait_idle()) return 0; }
        sector += (uint64_t)n; count -= n;
    }
    return 1;
}

/* Find the first hard disk on the four IDE positions (ATAPI drives are skipped). Returns its size in sectors, or 0. */
uint32_t dgn_ata_probe(void) {
    static const uint16_t bases[2] = { 0x1F0, 0x170 }, ctls[2] = { 0x3F6, 0x376 };
    for (int ch = 0; ch < 2; ch++) {
        uint16_t b = bases[ch], c = ctls[ch];
        if (inb(b + 7) == 0xFF) continue;
        for (int dv = 0; dv < 2; dv++) {
            outb(b + 6, 0xA0 | (dv << 4));
            for (int i = 0; i < 4; i++) inb(c);
            outb(b + 2, 0); outb(b + 3, 0); outb(b + 4, 0); outb(b + 5, 0);
            outb(b + 7, 0xEC);
            if (inb(b + 7) == 0) continue;
            long i = 0;
            while ((inb(b + 7) & 0x80) && i < 3000000L) i++;
            if (inb(b + 4) != 0 || inb(b + 5) != 0) continue;
            int ok = 0;
            for (i = 0; i < 3000000L; i++) { uint8_t s = inb(b + 7); if (s & 1) break; if (!(s & 0x80) && (s & 8)) { ok = 1; break; } }
            if (!ok) continue;
            uint16_t id[256]; uint16_t* p = id; uint64_t cnt = 256;
            __asm__ volatile("rep insw" : "+D"(p), "+c"(cnt) : "d"(b) : "memory");
            uint32_t sectors = (uint32_t)id[60] | ((uint32_t)id[61] << 16);
            if (sectors == 0) continue;
            ata_io = b; ata_ctl = c; ata_sel = 0xE0 | (dv << 4); ata_chan = ch;
            return sectors;
        }
    }
    return 0;
}
