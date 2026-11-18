#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

static inline void outw(uint16_t p, uint16_t v) { __asm__ volatile("outw %0, %1" :: "a"(v), "Nd"(p)); }
static inline void outl(uint16_t p, uint32_t v) { __asm__ volatile("outl %0, %1" :: "a"(v), "Nd"(p)); }
static inline uint16_t inw(uint16_t p) { uint16_t v; __asm__ volatile("inw %1, %0" : "=a"(v) : "Nd"(p)); return v; }
static inline uint8_t inb(uint16_t p) { uint8_t v; __asm__ volatile("inb %1, %0" : "=a"(v) : "Nd"(p)); return v; }

#define FW_FILE_DIR 0x0019
static inline uint32_t bs32(uint32_t v) { return __builtin_bswap32(v); }
static inline uint64_t bs64(uint64_t v) { return __builtin_bswap64(v); }
static void fw_select(uint16_t sel) { outw(0x510, sel); }
static uint8_t fw_byte(void) { return inb(0x511); }
static uint32_t fw_u32(void) { uint32_t v = 0; for (int i = 0; i < 4; i++) v = (v << 8) | fw_byte(); return v; }

typedef struct { uint32_t control; uint32_t length; uint64_t address; } __attribute__((packed)) FwDma;
static int fw_dma_write(uint16_t sel, const void* data, uint32_t len) {
    static volatile FwDma dma __attribute__((aligned(16)));
    dma.control = bs32(((uint32_t)sel << 16) | 8   | 16  );
    dma.length = bs32(len);
    dma.address = bs64((uint64_t)(uintptr_t)data);
    __asm__ volatile("" ::: "memory");
    uint64_t a = (uint64_t)(uintptr_t)&dma;
    outl(0x514, bs32((uint32_t)(a >> 32)));
    outl(0x518, bs32((uint32_t)a));
    for (int i = 0; i < 1000000; i++) { if ((bs32(dma.control) & ~1u) == 0) return (bs32(dma.control) & 1) ? -1 : 0; }
    return -1;
}

typedef struct { uint64_t addr; uint32_t fourcc, flags, width, height, stride; } __attribute__((packed)) RamfbCfg;

uint32_t* dgn_bm_fb_ptr;
int dgn_bm_fb_w, dgn_bm_fb_h;

extern uint64_t pd[];
extern uint64_t pdpt[];
extern uint64_t pd_hi[];
extern struct MbSaved { uint64_t fb_addr; uint32_t pitch, w, h, mem_upper; uint8_t bpp, type, has_fb; } dgn_mb;
static int hi_used;
static void reload_cr3(void) { uint64_t cr3; __asm__ volatile("movq %%cr3, %0; movq %0, %%cr3" : "=r"(cr3) :: "memory"); }
/* identity-map [addr, addr+len) uncached, also above 4 GB (many laptops put the framebuffer there) */
static int map_range(uint64_t addr, uint64_t len) {
    uint64_t a0 = addr & ~0x1FFFFFULL, a1 = (addr + len + 0x1FFFFFULL) & ~0x1FFFFFULL;
    for (uint64_t a = a0; a < a1; a += 0x200000) {
        uint64_t g = a >> 30;
        if (g >= 512) return 0;
        if (g < 4) { pd[a >> 21] |= 0x18; continue; }
        if (!(pdpt[g] & 1)) {
            if (hi_used >= 4) return 0;
            uint64_t* t = pd_hi + (uint64_t)hi_used * 512; hi_used++;
            for (int i = 0; i < 512; i++) t[i] = 0;
            pdpt[g] = (uint64_t)(uintptr_t)t | 3;
        }
        uint64_t* t = (uint64_t*)(uintptr_t)(pdpt[g] & ~0xFFFULL);
        t[(a >> 21) & 511] = a | 0x83 | 0x18;
    }
    reload_cr3();
    return 1;
}

/* padded rows (pitch != width*4): draw into a packed shadow copy and copy the rows out */
static uint32_t* hw_fb; static long hw_pitch;
static int dirty_on, dx0, dy0, dx1, dy1;
void dgn_bm_fb_sync(int x, int y, int w, int h) {
    if (!hw_fb || !dgn_bm_fb_ptr) return;
    int x1 = x + w, y1 = y + h;
    if (x < 0) x = 0; if (y < 0) y = 0; if (x1 > dgn_bm_fb_w) x1 = dgn_bm_fb_w; if (y1 > dgn_bm_fb_h) y1 = dgn_bm_fb_h;
    if (x1 <= x || y1 <= y) return;
    for (int j = y; j < y1; j++) memcpy(hw_fb + (long)j * hw_pitch + x, dgn_bm_fb_ptr + (long)j * dgn_bm_fb_w + x, (size_t)(x1 - x) * 4);
}
void dgn_bm_fb_dirty(int x, int y, int w, int h) {
    if (!hw_fb) return;
    if (!dirty_on) { dx0 = x; dy0 = y; dx1 = x + w; dy1 = y + h; dirty_on = 1; return; }
    if (x < dx0) dx0 = x; if (y < dy0) dy0 = y; if (x + w > dx1) dx1 = x + w; if (y + h > dy1) dy1 = y + h;
}
void dgn_bm_fb_flush(void) {
    if (!hw_fb || !dirty_on) return;
    dirty_on = 0;
    dgn_bm_fb_sync(dx0, dy0, dx1 - dx0, dy1 - dy0);
}

/* progress marks on the first pixels: show how far start-up got without a serial port */
static uint32_t* mk_fb; static long mk_pitch; static int mk_w, mk_h;
void dgn_bm_early_fb(void) {
    if (!dgn_mb.has_fb || dgn_mb.bpp != 32 || dgn_mb.type != 1 || !dgn_mb.fb_addr || dgn_mb.w < 64 || dgn_mb.h < 64 || dgn_mb.pitch < dgn_mb.w * 4) return;
    if (!map_range(dgn_mb.fb_addr, (uint64_t)dgn_mb.pitch * dgn_mb.h)) return;
    mk_fb = (uint32_t*)(uintptr_t)dgn_mb.fb_addr; mk_pitch = dgn_mb.pitch / 4; mk_w = (int)dgn_mb.w; mk_h = (int)dgn_mb.h;
    for (int y = 0; y < 40 && y < mk_h; y++) for (int x = 0; x < mk_w; x++) mk_fb[(long)y * mk_pitch + x] = 0x101830;
    extern void dgn_bm_mark(int n, uint32_t color);
    dgn_bm_mark(0, 0xFFFFFF);
}
static uint32_t mk_col[16];
static void mark_draw(int n, uint32_t color) {
    for (int y = 8; y < 32; y++) for (int x = 8 + n * 32; x < 8 + n * 32 + 24 && x < mk_w; x++) mk_fb[(long)y * mk_pitch + x] = color;
}
void dgn_bm_mark(int n, uint32_t color) {
    if (!mk_fb || n < 0 || n >= 16) return;
    mk_col[n] = color; mark_draw(n, color);
}
static void mark_redraw(void) { if (!mk_fb) return; for (int n = 0; n < 16; n++) if (mk_col[n]) mark_draw(n, mk_col[n]); }
#ifndef FORCE_SHADOW
#define FORCE_SHADOW 0
#endif
static int mb_fb(void) {
    if (!dgn_mb.has_fb) return 0;
    uint64_t addr = dgn_mb.fb_addr; uint32_t pitch = dgn_mb.pitch, w = dgn_mb.w, h = dgn_mb.h;
    if (dgn_mb.bpp != 32 || dgn_mb.type != 1 || addr == 0 || w < 64 || h < 64 || pitch < w * 4 || (pitch & 3)) return 0;
    if (!map_range(addr, (uint64_t)pitch * h)) return 0;
    if (pitch == w * 4 && !FORCE_SHADOW) {
        dgn_bm_fb_ptr = (uint32_t*)(uintptr_t)addr; dgn_bm_fb_w = (int)w; dgn_bm_fb_h = (int)h;
        memset(dgn_bm_fb_ptr, 0, (size_t)pitch * h);
    } else {
        uint32_t* sh = (uint32_t*)malloc((size_t)w * h * 4 + 4096);
        if (!sh) return 0;
        memset(sh, 0, (size_t)w * h * 4);
        memset((void*)(uintptr_t)addr, 0, (size_t)pitch * h);
        hw_fb = (uint32_t*)(uintptr_t)addr; hw_pitch = pitch / 4;
        dgn_bm_fb_ptr = sh; dgn_bm_fb_w = (int)w; dgn_bm_fb_h = (int)h;
    }
    { extern void dgn_bm_cur_reset(void); dgn_bm_cur_reset(); }
    return 1;
}

static inline uint32_t inl(uint16_t p) { uint32_t v; __asm__ volatile("inl %1, %0" : "=a"(v) : "Nd"(p)); return v; }
static uint32_t pcir(int b, int d, int f, int o) { outl(0xCF8, 0x80000000u | ((uint32_t)b << 16) | ((uint32_t)d << 11) | ((uint32_t)f << 8) | (uint32_t)(o & 0xFC)); return inl(0xCFC); }
static void pciw(int b, int d, int f, int o, uint32_t v) { outl(0xCF8, 0x80000000u | ((uint32_t)b << 16) | ((uint32_t)d << 11) | ((uint32_t)f << 8) | (uint32_t)(o & 0xFC)); outl(0xCFC, v); }
/* size of the adapter's first memory BAR (Bochs register 0xA is unimplemented on some hypervisors) */
static uint64_t pci_vga_bar(void) {
    for (int b = 0; b < 4; b++) for (int d = 0; d < 32; d++) {
        uint32_t id = pcir(b, d, 0, 0);
        if ((id & 0xFFFF) == 0xFFFF) continue;
        if ((pcir(b, d, 0, 8) >> 24) != 3) continue;
        for (int o = 0x10; o <= 0x24; o += 4) {
            uint32_t bar = pcir(b, d, 0, o);
            if ((bar & 1) || (bar & 6) == 4) continue;
            uint32_t cmd = pcir(b, d, 0, 4);
            pciw(b, d, 0, 4, cmd & ~3u);
            pciw(b, d, 0, o, 0xFFFFFFFFu);
            uint32_t m = pcir(b, d, 0, o);
            pciw(b, d, 0, o, bar);
            pciw(b, d, 0, 4, cmd);
            m &= ~0xFu;
            if (!m) continue;
            return (uint64_t)((~m) + 1u);
        }
    }
    return 0;
}
static int fb_from_mb, dispi_on;
static uint64_t dispi_vram;
static uint16_t dispi_r(int i) { outw(0x1CE, (uint16_t)i); return inw(0x1CF); }
static void dispi_w(int i, uint16_t v) { outw(0x1CE, (uint16_t)i); outw(0x1CF, v); }
static uint64_t dispi_diag;
static void probe_note(uint16_t id, uint64_t rep, uint64_t bar) {
    uint64_t r = rep >> 20, b = bar >> 20;
    if (r > 255) r = 255;
    if (b > 65535) b = 65535;
    dispi_diag = (uint64_t)id | (r << 16) | (7ULL << 24) | (b << 32);
}
static void dispi_probe(void) {
    uint16_t id = dispi_r(0);
    uint64_t bar = pci_vga_bar();
    if (id < 0xB0C0 || id > 0xB0CF) { probe_note(id, 0, bar); return; }
    dispi_w(0, 0xB0C5);
    uint64_t vram = (uint64_t)dispi_r(0xA) * 65536ULL;
    probe_note(id, vram, bar);
    if (hw_fb) return;
    uint64_t need = (uint64_t)dgn_mb.pitch * dgn_mb.h;
    if (bar > vram && bar <= (256ULL << 20)) vram = bar;
    if (vram < need) vram = need;
    if (vram > (64ULL << 20)) vram = 64ULL << 20;
    uint64_t addr = dgn_mb.fb_addr;
    if (addr + vram > 0x100000000ULL) vram = addr < 0x100000000ULL ? 0x100000000ULL - addr : need;
    if (!map_range(addr, vram)) return;
    dispi_vram = vram;
    dispi_on = 1;
}
long long dgn_bm_fb_vram(void) { return dispi_on ? (long long)dispi_vram : (fb_from_mb ? 0 : -1); }
long long dgn_bm_fb_diag(void) { return (long long)dispi_diag; }
static int dispi_try(int w, int h, int order) {
    dispi_w(4, 0);
    if (order == 0) {
        dispi_w(1, (uint16_t)w); dispi_w(2, (uint16_t)h); dispi_w(3, 32);
    } else {
        dispi_w(3, 32); dispi_w(2, (uint16_t)h); dispi_w(1, (uint16_t)w);
    }
    dispi_w(6, (uint16_t)w);
    dispi_w(8, 0);
    dispi_w(9, 0);
    dispi_w(4, 0x01 | 0x40);
    return dispi_r(1) == w && dispi_r(2) == h;
}
static int dispi_set(int w, int h) {
    if (w < 320 || h < 200 || w > 4096 || h > 4096) return 0;
    uint64_t vmb = dispi_vram >> 20;
    uint64_t base = (uint64_t)(dispi_r(0) & 0xFFFF) | (vmb << 16);
    if ((uint64_t)w * h * 4 > dispi_vram) { dispi_diag = base | (1ULL << 24); return 0; }
    int ok = dispi_try(w, h, 0);
    int stage = 0;
    if (!ok) { ok = dispi_try(w, h, 1); stage = 1; }
    if (!ok) {
        dispi_diag = base | ((uint64_t)(stage + 2) << 24) | ((uint64_t)dispi_r(1) << 32) | ((uint64_t)dispi_r(2) << 48);
        int ow = dgn_bm_fb_w, oh = dgn_bm_fb_h;
        if (ow > 0 && oh > 0 && dispi_try(ow, oh, 0)) { dgn_bm_fb_w = ow; dgn_bm_fb_h = oh; }
        return 0;
    }
    dispi_diag = base | ((uint64_t)w << 32) | ((uint64_t)h << 48);
    dgn_bm_fb_w = w; dgn_bm_fb_h = h;
    memset(dgn_bm_fb_ptr, 0, (size_t)w * h * 4);
    { extern void dgn_bm_cur_reset(void); dgn_bm_cur_reset(); }
    return 1;
}

int dgn_bm_fb_init(int w, int h) {
    if (fb_from_mb) {
        if (w == dgn_bm_fb_w && h == dgn_bm_fb_h) return 1;
        if (dispi_on) return dispi_set(w, h);
        return 1;
    }
    if (mb_fb()) { fb_from_mb = 1; dgn_bm_mark(4, 0xC060FF); mark_redraw(); dispi_probe(); return 1; }
    if (w < 64 || h < 64 || w > 2048 || h > 1536) return 0;
    fw_select(FW_FILE_DIR);
    uint32_t count = fw_u32(); int sel = -1;
    for (uint32_t i = 0; i < count; i++) {
        uint32_t size = fw_u32(); (void)size;
        uint16_t s = (uint16_t)((fw_byte() << 8) | fw_byte()); fw_byte(); fw_byte();
        char name[56]; for (int k = 0; k < 56; k++) name[k] = (char)fw_byte();
        if (sel < 0 && strcmp(name, "etc/ramfb") == 0) sel = s;
    }
    if (sel < 0) return 0;
    uint32_t* nfb = (uint32_t*)malloc((size_t)w * h * 4 + 4096);
    if (!nfb) return 0;
    memset(nfb, 0, (size_t)w * h * 4);
    RamfbCfg c;
    c.addr = bs64((uint64_t)(uintptr_t)nfb);
    c.fourcc = bs32(0x34325258u);
    c.flags = 0; c.width = bs32((uint32_t)w); c.height = bs32((uint32_t)h); c.stride = bs32((uint32_t)w * 4);
    if (fw_dma_write((uint16_t)sel, &c, sizeof c) != 0) { free(nfb); return 0; }
    { uint32_t* old = dgn_bm_fb_ptr; dgn_bm_fb_ptr = nfb; if (old) free(old); }
    dgn_bm_fb_w = w; dgn_bm_fb_h = h;
    { extern void dgn_bm_cur_reset(void); dgn_bm_cur_reset(); }
    return 1;
}
