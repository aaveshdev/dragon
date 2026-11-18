#include <stdint.h>
#include <stddef.h>

static inline void outb(uint16_t p, uint8_t v) { __asm__ volatile("outb %0, %1" :: "a"(v), "Nd"(p)); }
static inline void outw(uint16_t p, uint16_t v) { __asm__ volatile("outw %0, %1" :: "a"(v), "Nd"(p)); }
static inline void outl(uint16_t p, uint32_t v) { __asm__ volatile("outl %0, %1" :: "a"(v), "Nd"(p)); }
static inline uint8_t inb(uint16_t p) { uint8_t v; __asm__ volatile("inb %1, %0" : "=a"(v) : "Nd"(p)); return v; }

#define COM1 0x3F8
#define HPET 0xFED00000UL

extern int dgn_bm_con_on, dgn_bm_con_fbonly, dgn_bm_con_seronly;
extern void dgn_bm_con_out(char c);
void dgn_bm_putc(char c) {
    if (!dgn_bm_con_fbonly) { while (!(inb(COM1 + 5) & 0x20)) {} outb(COM1, (uint8_t)c); }
    if (dgn_bm_con_on && !dgn_bm_con_seronly) dgn_bm_con_out(c);
}
extern void dgn_bm_task_yield(void);
int  dgn_bm_getc_nb(void) { return (inb(COM1 + 5) & 1) ? inb(COM1) : -1; }
int  dgn_bm_getc(void) { while (!(inb(COM1 + 5) & 1)) dgn_bm_task_yield(); return inb(COM1); }

static uint64_t hpet_ps, tsc_hz, tsc_base;
static inline uint64_t rdtsc(void) { uint32_t lo, hi; __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi)); return ((uint64_t)hi << 32) | lo; }
unsigned long long dgn_bm_now_us(void) {
    if (hpet_ps) return *(volatile uint64_t*)(HPET + 0xF0) * hpet_ps / 1000000ULL;
    if (tsc_hz) { uint64_t d = rdtsc() - tsc_base; return (d / tsc_hz) * 1000000ULL + (d % tsc_hz) * 1000000ULL / tsc_hz; }
    return 0;
}
static void tsc_calibrate(void) {
    outb(0x61, (uint8_t)((inb(0x61) & 0xFC) | 1));
    outb(0x43, 0xB0); outb(0x42, 59659 & 0xFF); outb(0x42, 59659 >> 8);
    uint64_t t0 = rdtsc();
    while (!(inb(0x61) & 0x20)) {}
    uint64_t t1 = rdtsc();
    tsc_hz = (t1 - t0) * 1193182ULL / 59659ULL;
    tsc_base = rdtsc();
}
extern uint64_t dgn_rsdp_addr;
static inline uint16_t inw(uint16_t p) { uint16_t v; __asm__ volatile("inw %1, %0" : "=a"(v) : "Nd"(p)); return v; }
static int acpi_sum_ok(const uint8_t* t, uint32_t n) { uint8_t s = 0; for (uint32_t i = 0; i < n; i++) s += t[i]; return s == 0; }
static const uint8_t* acpi_fadt(void) {
    const uint8_t* r = (const uint8_t*)(uintptr_t)dgn_rsdp_addr;
    if (!r) {
        for (uint64_t a = 0xE0000; a < 0x100000; a += 16) {
            const uint8_t* q = (const uint8_t*)(uintptr_t)a;
            if (q[0] == 'R' && q[1] == 'S' && q[2] == 'D' && q[3] == ' ' && q[4] == 'P' && q[5] == 'T' && q[6] == 'R' && q[7] == ' ' && acpi_sum_ok(q, 20)) { r = q; break; }
        }
    }
    if (!r) return 0;
    uint32_t rsdt = *(const uint32_t*)(r + 16);
    if (!rsdt || rsdt >= 0xFFFFF000u) return 0;
    const uint8_t* t = (const uint8_t*)(uintptr_t)rsdt;
    uint32_t len = *(const uint32_t*)(t + 4);
    if (len < 36 || len > 65536) return 0;
    for (uint32_t i = 36; i + 4 <= len; i += 4) {
        uint32_t a = *(const uint32_t*)(t + i);
        if (!a || a >= 0xFFFFF000u) continue;
        const uint8_t* f = (const uint8_t*)(uintptr_t)a;
        if (f[0] == 'F' && f[1] == 'A' && f[2] == 'C' && f[3] == 'P') return f;
    }
    return 0;
}
int dgn_acpi_poweroff_ok;
static void acpi_poweroff(void) {
    const uint8_t* f = acpi_fadt();
    if (!f) return;
    uint32_t flen = *(const uint32_t*)(f + 4);
    if (flen < 72) return;
    uint32_t dsdt = *(const uint32_t*)(f + 40);
    uint16_t pm1a = (uint16_t)*(const uint32_t*)(f + 64), pm1b = (uint16_t)*(const uint32_t*)(f + 68);
    uint32_t smi = *(const uint32_t*)(f + 48);
    uint8_t en = f[52];
    if (!pm1a || !dsdt || dsdt >= 0xFFFFF000u) return;
    const uint8_t* d = (const uint8_t*)(uintptr_t)dsdt;
    uint32_t dl = *(const uint32_t*)(d + 4);
    if (dl < 44 || dl > (16u << 20)) return;
    int typa = -1, typb = 0;
    for (uint32_t i = 36; i + 12 < dl; i++) {
        if (d[i] == '_' && d[i + 1] == 'S' && d[i + 2] == '5' && d[i + 3] == '_' && d[i + 4] == 0x12) {
            const uint8_t* p = d + i + 5;
            p += ((p[0] >> 6) & 3) + 1;
            p++;
            if (*p == 0x0A) p++;
            typa = *p & 7; p++;
            if (*p == 0x0A) p++;
            typb = *p & 7;
            break;
        }
    }
    if (typa < 0) return;
    dgn_acpi_poweroff_ok = 1;
    if (smi && en && !(inw(pm1a) & 1)) { outb((uint16_t)smi, en); for (volatile long k = 0; k < 3000000L; k++) { if (inw(pm1a) & 1) break; } }
    outw(pm1a, (uint16_t)((typa << 10) | 0x2000));
    if (pm1b) outw(pm1b, (uint16_t)((typb << 10) | 0x2000));
    for (volatile long k = 0; k < 50000000L; k++) { }
}
void dgn_bm_halt(int code) {
    outl(0xF4, (uint32_t)code);
    if (code == 0 || code == 1) acpi_poweroff();
    outw(0x604, 0x2000);
    outw(0xB004, 0x2000);
    outw(0x4004, 0x3400);
    for (;;) __asm__ volatile("cli; hlt");
}

extern uint32_t mb_info;
extern char __heap_start[], __bootinfo[];
uint64_t dgn_rsdp_addr;
struct MbSaved { uint64_t fb_addr; uint32_t pitch, w, h, mem_upper; uint8_t bpp, type, has_fb; } dgn_mb;
static uint64_t mb_ram_end, mb_ram_base, heap_lo, heap_hi;
static void heap_cut(uint64_t s, uint64_t e) {
    if (e <= heap_lo || s >= heap_hi) return;
    uint64_t a = s > heap_lo ? s - heap_lo : 0, b = e < heap_hi ? heap_hi - e : 0;
    if (a >= b) heap_hi = s; else heap_lo = (e + 15) & ~15ULL;
}
static void mb_save(void) {
    uint32_t* bi = (uint32_t*)__bootinfo;
    for (int i = 0; i < 1024; i++) bi[i] = 0;
    uint64_t obs_s[10], obs_e[10]; int nobs = 0;
    if (mb_info) {
        const uint8_t* p = (const uint8_t*)(uintptr_t)mb_info;
        uint32_t total = *(const uint32_t*)p;
        obs_s[nobs] = mb_info; obs_e[nobs++] = (uint64_t)mb_info + total;
        uint32_t nm = 0;
        for (uint32_t off = 8; off + 8 <= total; ) {
            uint32_t t = *(const uint32_t*)(p + off), sz = *(const uint32_t*)(p + off + 4);
            if (t == 0 || sz < 8) break;
            if (t == 4) dgn_mb.mem_upper = *(const uint32_t*)(p + off + 12);
            if (t == 8) {
                dgn_mb.fb_addr = *(const uint64_t*)(p + off + 8); dgn_mb.pitch = *(const uint32_t*)(p + off + 16);
                dgn_mb.w = *(const uint32_t*)(p + off + 20); dgn_mb.h = *(const uint32_t*)(p + off + 24);
                dgn_mb.bpp = p[off + 28]; dgn_mb.type = p[off + 29]; dgn_mb.has_fb = 1;
            }
            if (t == 12) bi[3] = 1;
            if (t == 14 && !dgn_rsdp_addr) dgn_rsdp_addr = (uint64_t)(uintptr_t)(p + off + 8);
            if (t == 15) dgn_rsdp_addr = (uint64_t)(uintptr_t)(p + off + 8);
            if (t == 6) {
                uint32_t es = *(const uint32_t*)(p + off + 8);
                for (uint32_t o = off + 16; es >= 24 && o + es <= off + sz; o += es) {
                    uint64_t base = *(const uint64_t*)(p + o), len = *(const uint64_t*)(p + o + 8); uint32_t ty = *(const uint32_t*)(p + o + 16);
                    if (ty == 1 && base < 0xFFFFF000ULL && base + len > 0xFFFFF000ULL) len = 0xFFFFF000ULL - base;
                    if (ty == 1 && base < 0xFFFFF000ULL && base + len > 0x100000ULL) { if (base < 0x100000ULL) { len -= 0x100000ULL - base; base = 0x100000ULL; } if (len > mb_ram_end - mb_ram_base) { mb_ram_base = base; mb_ram_end = base + len; } }
                }
            }
            if (t == 3 && nm < 8 && sz >= 16) {
                uint32_t ms = *(const uint32_t*)(p + off + 8), me = *(const uint32_t*)(p + off + 12);
                uint32_t* ent = bi + 4 + nm * 8;
                ent[0] = ms; ent[1] = me;
                char* nmz = (char*)(ent + 2);
                for (uint32_t k = 0; k < 23 && off + 16 + k < off + sz && p[off + 16 + k]; k++) nmz[k] = (char)p[off + 16 + k];
                if (nobs < 10) { obs_s[nobs] = ms; obs_e[nobs++] = me; }
                nm++;
            }
            off += (sz + 7) & ~7u;
        }
        bi[2] = nm;
    }
    bi[0] = 0x56414C53; bi[1] = dgn_mb.has_fb ? 1 : 0;
    uint64_t end = 0;
    if (mb_ram_end) end = mb_ram_end;
    else if (dgn_mb.mem_upper >= 16384) end = 0x100000ULL + (uint64_t)dgn_mb.mem_upper * 1024ULL;
    if (end) {
        if (end > 0xFFFFF000ULL) end = 0xFFFFF000ULL;
        heap_lo = ((uint64_t)(uintptr_t)__heap_start + 15) & ~15ULL;
        if (mb_ram_base > heap_lo) heap_lo = mb_ram_base;
        heap_hi = end - 0x10000ULL;
        for (int i = 0; i < nobs; i++) heap_cut(obs_s[i], obs_e[i]);
    }
}
char* dgn_bm_heap_start(void) { return heap_lo ? (char*)(uintptr_t)heap_lo : __heap_start; }
char* dgn_bm_ram_end(void) {
    if (heap_hi) return (char*)(uintptr_t)heap_hi;
    outb(0x70, 0x34); uint32_t lo = inb(0x71);
    outb(0x70, 0x35); uint32_t hi = inb(0x71);
    uint64_t end = 0x1000000ULL + (((uint64_t)hi << 8) | lo) * 65536ULL;
    if (end < 0x2000000ULL) { outb(0x70, 0x30); uint32_t a = inb(0x71); outb(0x70, 0x31); uint32_t b = inb(0x71); end = 0x100000ULL + (((uint64_t)b << 8) | a) * 1024ULL; }
    return (char*)(uintptr_t)(end - 0x100000ULL);
}

extern void dgn_x86_init(void);
extern int main(int argc, char** argv);
extern void dgn_bm_early_fb(void);
void dgn_bm_entry(void) {
    mb_save();
    dgn_bm_early_fb();
    outb(COM1 + 1, 0); outb(COM1 + 3, 0x83); outb(COM1, 1); outb(COM1 + 1, 0); outb(COM1 + 3, 3); outb(COM1 + 2, 0xC7); outb(COM1 + 4, 3);
    { extern void dgn_bm_mark(int, uint32_t); dgn_bm_mark(1, 0xFFE040); }
    dgn_x86_init();
    { extern void dgn_bm_mark(int, uint32_t); dgn_bm_mark(2, 0x40FF40); }
    uint64_t caps = *(volatile uint64_t*)HPET;
    uint32_t period_fs = (uint32_t)(caps >> 32);
    if (caps != ~0ULL && period_fs >= 1000000 && period_fs <= 100000000) { hpet_ps = period_fs / 1000; *(volatile uint64_t*)(HPET + 0x10) |= 1; }
    else tsc_calibrate();
    { extern void dgn_bm_mark(int, uint32_t); dgn_bm_mark(3, 0x40C0FF); }
    char* argv[2] = { "dragon", 0 };
    int rc = main(1, argv);
    dgn_bm_halt(rc);
}
