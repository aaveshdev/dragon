#include <stdint.h>
#include <stddef.h>

#define UART0 0x09000000UL
#define A64_MAGIC 0x5641454C49534249ULL

/* handed over by the UEFI loader (baremetal/aarch64/uefi) in x0; with QEMU -kernel x0 is a device tree and there is none */
typedef struct {
    uint64_t magic, version;
    uint64_t fb_base, fb_w, fb_h, fb_stride, fb_fmt;
    uint64_t heap_lo, heap_hi, rsdp, uart_base, flags;
    uint64_t normal_gb[16];
} A64Boot;
A64Boot* dgn_boot;
extern uint64_t dgn_boot_x0;
extern uint64_t l1_table[512], l1_hi[512];

static A64Boot* boot_from(uint64_t p) {
    if (p < 0x40000000ULL || (p & 7)) return 0;
    A64Boot* b = (A64Boot*)(uintptr_t)p;
    return b->magic == A64_MAGIC ? b : 0;
}
/* runs with the MMU off, before the tables are loaded: with UEFI every 1 GB block holding RAM is normal memory, the rest device */
void dgn_bm_mmu_prep(uint64_t x0) {
    A64Boot* b = boot_from(x0);
    if (!b) return;
    for (uint64_t g = 0; g < 1024; g++) {
        uint64_t* e = g < 512 ? &l1_table[g] : &l1_hi[g - 512];
        if ((b->normal_gb[g >> 6] >> (g & 63)) & 1) *e = (g << 30) | 0x705;
        else *e = (g << 30) | 0x601 | (3ULL << 53);
    }
}

static volatile uint32_t* uart = (volatile uint32_t*)UART0;

extern int dgn_bm_con_on, dgn_bm_con_fbonly, dgn_bm_con_seronly;
extern void dgn_bm_con_out(char c);
void dgn_bm_putc(char c) {
    if (uart && !dgn_bm_con_fbonly) { while (uart[0x18 / 4] & 0x20) {} uart[0] = (uint32_t)(uint8_t)c; }
    if (dgn_bm_con_on && !dgn_bm_con_seronly) dgn_bm_con_out(c);
}
extern void dgn_bm_task_yield(void);

int  dgn_bm_getc_nb(void) { return (!uart || (uart[0x18 / 4] & 0x10)) ? -1 : (int)(uart[0] & 0xff); }
int  dgn_bm_getc(void) { while (!uart || (uart[0x18 / 4] & 0x10)) dgn_bm_task_yield(); return (int)(uart[0] & 0xff); }
unsigned long long dgn_bm_now_us(void) {
    uint64_t c, f;
    __asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(c));
    __asm__ volatile("mrs %0, cntfrq_el0" : "=r"(f));
    return (unsigned long long)(c / (f / 1000000ULL));
}
void dgn_bm_halt(int code) {
    (void)code;
    register uint64_t x0 __asm__("x0") = 0x84000008UL;          /* PSCI SYSTEM_OFF, through hvc or smc depending on the board */
    if (dgn_boot && (dgn_boot->flags & 1)) __asm__ volatile("smc #0" : "+r"(x0) :: "memory");
    else __asm__ volatile("hvc #0" : "+r"(x0) :: "memory");
    for (;;) __asm__ volatile("wfi");
}

#define FWCFG 0x09020000UL
/* end of RAM: the size comes from fw_cfg (key 3, 64-bit little endian), RAM starts at 0x40000000 */
extern char __heap_start[];
char* dgn_bm_heap_start(void) { return dgn_boot ? (char*)(uintptr_t)dgn_boot->heap_lo : __heap_start; }
char* dgn_bm_ram_end(void) {
    static uint64_t end;
    if (dgn_boot) return (char*)(uintptr_t)dgn_boot->heap_hi;
    if (!end) {
        *(volatile uint16_t*)(FWCFG + 8) = (uint16_t)(3 << 8);
        uint64_t v = 0;
        for (int i = 0; i < 8; i++) v |= (uint64_t)*(volatile uint8_t*)FWCFG << (8 * i);
        end = 0x40000000UL + (v ? v : (128ULL << 20));
    }
    return (char*)(uintptr_t)end;
}

/* hardware addresses: the QEMU virt defaults, replaced by what the ACPI tables of the firmware say */
uint64_t dgn_gicd = 0x08000000UL, dgn_gicc = 0x08010000UL, dgn_gicr = 0x080A0000UL;
int dgn_timer_ppi = 30;
static uint64_t acpi_ecam;
static const uint8_t* acpi_find(const uint8_t* xsdt, const char* sig) {
    uint32_t len = *(const uint32_t*)(xsdt + 4);
    for (uint32_t off = 36; off + 8 <= len; off += 8) {
        const uint8_t* t = (const uint8_t*)(uintptr_t)*(const uint64_t*)(xsdt + off);
        if (t && t[0] == sig[0] && t[1] == sig[1] && t[2] == sig[2] && t[3] == sig[3]) return t;
    }
    return 0;
}
static void acpi_init(uint64_t rsdp) {
    const uint8_t* r = (const uint8_t*)(uintptr_t)rsdp;
    if (!r || r[15] < 2) return;
    const uint8_t* xsdt = (const uint8_t*)(uintptr_t)*(const uint64_t*)(r + 24);
    if (!xsdt) return;
    const uint8_t* t;
    if ((t = acpi_find(xsdt, "APIC"))) {
        uint32_t len = *(const uint32_t*)(t + 4);
        for (uint32_t off = 44; off + 2 <= len; off += t[off + 1] ? t[off + 1] : 2) {
            const uint8_t* e = t + off;
            if (e[0] == 0x0B && e[1] >= 40) { uint64_t a = *(const uint64_t*)(e + 32); if (a) dgn_gicc = a; }
            else if (e[0] == 0x0C && e[1] >= 24) dgn_gicd = *(const uint64_t*)(e + 8);
            else if (e[0] == 0x0E && e[1] >= 16) dgn_gicr = *(const uint64_t*)(e + 4);
        }
    }
    if ((t = acpi_find(xsdt, "GTDT"))) { uint32_t g = *(const uint32_t*)(t + 56); if (g >= 16 && g < 32) dgn_timer_ppi = (int)g; }
    if ((t = acpi_find(xsdt, "MCFG")) && *(const uint32_t*)(t + 4) >= 60) acpi_ecam = *(const uint64_t*)(t + 44);
    if ((t = acpi_find(xsdt, "SPCR"))) {
        uint8_t kind = t[36]; uint64_t a = *(const uint64_t*)(t + 44);
        if ((kind == 0x03 || kind == 0x0E) && a && t[40] == 0) uart = (volatile uint32_t*)(uintptr_t)a;
        else uart = 0;
    }
}

/* QEMU virt leaves PCI BARs unassigned when booting with -kernel, so hand out addresses here (bus 0 only) */
uintptr_t dgn_ecam_base;
extern volatile int dgn_probing;
uint32_t dgn_probe32(uintptr_t a);
static uint32_t probe32(uintptr_t a) { uint32_t v = 0xffffffffu; dgn_probing = 1; __asm__ volatile("ldr %w0, [%1]" : "+r"(v) : "r"(a) : "memory"); dgn_probing = 0; return v; }
uint32_t dgn_probe32(uintptr_t a) { return probe32(a); }
#define ECAM dgn_ecam_base
#define MMIO_NEXT_START 0x10000000UL
#define MMIO_END 0x3eff0000UL
static inline volatile uint32_t* cfg(int dev, int fn, int reg) { return (volatile uint32_t*)(ECAM + ((uintptr_t)dev << 15) + ((uintptr_t)fn << 12) + reg); }
static void pci_find_ecam(void) {
    dgn_ecam_base = 0;
    if (acpi_ecam && (probe32(acpi_ecam) & 0xffff) != 0xffff) { dgn_ecam_base = acpi_ecam; return; }
    if ((probe32(0x3f000000UL) & 0xffff) != 0xffff) dgn_ecam_base = 0x3f000000UL;
    else if ((probe32(0x4010000000UL) & 0xffff) != 0xffff) dgn_ecam_base = 0x4010000000UL;
}
static void pci_assign(void) {
    /* the config window is at 0x3f000000 with highmem-ecam=off and at 0x4010000000 otherwise (the default) */
    dgn_ecam_base = 0;
    if ((probe32(0x3f000000UL) & 0xffff) != 0xffff) dgn_ecam_base = 0x3f000000UL;
    else if ((probe32(0x4010000000UL) & 0xffff) != 0xffff) dgn_ecam_base = 0x4010000000UL;
    if (!dgn_ecam_base) return;
    uint64_t mem = MMIO_NEXT_START, io = 0x1000;
    for (int d = 0; d < 32; d++) for (int f = 0; f < 8; f++) {
        if ((*cfg(d, f, 0) & 0xffff) == 0xffff) { if (f == 0) break; continue; }
        if (((*cfg(d, f, 12) >> 16) & 0x7f) != 0) continue;
        for (int b = 0; b < 6; b++) {
            int r = 16 + 4 * b; uint32_t old = *cfg(d, f, r);
            *cfg(d, f, r) = 0xffffffffu; uint32_t m = *cfg(d, f, r); *cfg(d, f, r) = old;
            if (m == 0 || m == 0xffffffffu) continue;
            if (m & 1) { uint32_t sz = (~(m & ~3u) + 1) & 0xffff; if (!sz) continue; io = (io + sz - 1) & ~(uint64_t)(sz - 1);
                *cfg(d, f, r) = (uint32_t)io | 1; io += sz; }
            else { int is64 = ((m >> 1) & 3) == 2; uint64_t mm = m & ~0xfu; if (is64) mm |= 0xffffffff00000000ULL;
                uint32_t hi = 0; if (is64) { uint32_t oh = *cfg(d, f, r + 4); *cfg(d, f, r + 4) = 0xffffffffu; hi = *cfg(d, f, r + 4); *cfg(d, f, r + 4) = oh; mm = ((uint64_t)hi << 32) | (m & ~0xfu); }
                uint64_t sz = ~mm + 1; if (!is64) sz &= 0xffffffffu; if (!sz) continue;
                mem = (mem + sz - 1) & ~(sz - 1); if (mem + sz > MMIO_END) continue;
                *cfg(d, f, r) = (uint32_t)mem | (m & 0xf); if (is64) *cfg(d, f, r + 4) = 0;
                mem += sz; }
            if (((m >> 1) & 3) == 2 && !(m & 1)) b++;
        }
        if ((*cfg(d, f, 8) >> 24) != 6) *cfg(d, f, 4) = (*cfg(d, f, 4) & 0xffff) | 7;
        if (f == 0 && !((*cfg(d, 0, 12) >> 23) & 1)) break;
    }
}

extern int main(int argc, char** argv);
void dgn_bm_entry(void) {
    dgn_boot = boot_from(dgn_boot_x0);
    if (dgn_boot && dgn_boot->rsdp) acpi_init(dgn_boot->rsdp);
    if (!dgn_boot) pci_assign(); else pci_find_ecam();
    char* argv[2] = { "dragon", 0 };
    int rc = main(1, argv);
    dgn_bm_halt(rc);
}
