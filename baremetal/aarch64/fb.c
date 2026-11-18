#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

#define FWCFG_BASE   0x09020000UL
#define FW_FILE_DIR  0x0019

static inline uint16_t bs16(uint16_t v) { return (uint16_t)((v << 8) | (v >> 8)); }
static inline uint32_t bs32(uint32_t v) { return __builtin_bswap32(v); }
static inline uint64_t bs64(uint64_t v) { return __builtin_bswap64(v); }

static void fw_select(uint16_t sel) { *(volatile uint16_t*)(FWCFG_BASE + 8) = bs16(sel); }
static uint8_t fw_byte(void) { return *(volatile uint8_t*)(FWCFG_BASE); }
static uint32_t fw_u32(void) { uint32_t v = 0; for (int i = 0; i < 4; i++) v = (v << 8) | fw_byte(); return v; }

typedef struct { uint32_t control; uint32_t length; uint64_t address; } __attribute__((packed)) FwDma;
static int fw_dma_write(uint16_t sel, const void* data, uint32_t len) {
    static volatile FwDma dma __attribute__((aligned(16)));
    dma.control = bs32(((uint32_t)sel << 16) | 8   | 16  );
    dma.length = bs32(len);
    dma.address = bs64((uint64_t)(uintptr_t)data);
    __asm__ volatile("dmb sy" ::: "memory");
    *(volatile uint64_t*)(FWCFG_BASE + 16) = bs64((uint64_t)(uintptr_t)&dma);
    for (int i = 0; i < 1000000; i++) { if ((bs32(dma.control) & ~1u) == 0) return (bs32(dma.control) & 1) ? -1 : 0; }
    return -1;
}

typedef struct { uint64_t addr; uint32_t fourcc, flags, width, height, stride; } __attribute__((packed)) RamfbCfg;

uint32_t* dgn_bm_fb_ptr;
int dgn_bm_fb_w, dgn_bm_fb_h;

typedef struct {
    uint64_t magic, version;
    uint64_t fb_base, fb_w, fb_h, fb_stride, fb_fmt;
    uint64_t heap_lo, heap_hi, rsdp, uart_base, flags;
    uint64_t normal_gb[16];
} A64Boot;
extern A64Boot* dgn_boot;

/* UEFI: the firmware's framebuffer is the screen, drawing goes to a shadow copy that is copied out (and converted when the pixels are RGB) */
static volatile uint32_t* hw_fb; static long hw_pitch; static int hw_rgb;
static int dirty_on, dx0, dy0, dx1, dy1;

long long dgn_bm_fb_vram(void) { return -1; }
int dgn_bm_fb_init(int w, int h) {
    if (dgn_boot) {
        if (!dgn_boot->fb_base || dgn_boot->fb_w < 64 || dgn_boot->fb_h < 64) return 0;
        w = (int)dgn_boot->fb_w; h = (int)dgn_boot->fb_h;
        if (dgn_bm_fb_ptr) free(dgn_bm_fb_ptr);
        dgn_bm_fb_ptr = (uint32_t*)malloc((size_t)w * h * 4 + 4096);
        if (!dgn_bm_fb_ptr) return 0;
        memset(dgn_bm_fb_ptr, 0, (size_t)w * h * 4);
        hw_fb = (volatile uint32_t*)(uintptr_t)dgn_boot->fb_base; hw_pitch = (long)dgn_boot->fb_stride; hw_rgb = dgn_boot->fb_fmt == 0;
        for (long j = 0; j < h; j++) for (long i = 0; i < w; i++) hw_fb[j * hw_pitch + i] = 0;
        dgn_bm_fb_w = w; dgn_bm_fb_h = h;
        { extern void dgn_bm_cur_reset(void); dgn_bm_cur_reset(); }
        return 1;
    }
    if (w < 64 || h < 64 || w > 2048 || h > 1536) return 0;
    fw_select(FW_FILE_DIR);
    uint32_t count = fw_u32(), found = 0; int sel = -1;
    for (uint32_t i = 0; i < count; i++) {
        uint32_t size = fw_u32(); (void)size;
        uint16_t s = (uint16_t)((fw_byte() << 8) | fw_byte()); fw_byte(); fw_byte();
        char name[56]; for (int k = 0; k < 56; k++) name[k] = (char)fw_byte();
        if (!found && strcmp(name, "etc/ramfb") == 0) { sel = s; found = 1; }
    }
    if (sel < 0) return 0;
    if (dgn_bm_fb_ptr) free(dgn_bm_fb_ptr);
    dgn_bm_fb_ptr = (uint32_t*)malloc((size_t)w * h * 4 + 4096);
    if (!dgn_bm_fb_ptr) return 0;
    memset(dgn_bm_fb_ptr, 0, (size_t)w * h * 4);
    RamfbCfg c;
    c.addr = bs64((uint64_t)(uintptr_t)dgn_bm_fb_ptr);
    c.fourcc = bs32(0x34325258u);
    c.flags = 0; c.width = bs32((uint32_t)w); c.height = bs32((uint32_t)h); c.stride = bs32((uint32_t)w * 4);
    if (fw_dma_write((uint16_t)sel, &c, sizeof c) != 0) return 0;
    dgn_bm_fb_w = w; dgn_bm_fb_h = h;
    { extern void dgn_bm_cur_reset(void); dgn_bm_cur_reset(); }
    return 1;
}
long long dgn_bm_fb_diag(void) { return 0; }
void dgn_bm_fb_sync(int x, int y, int w, int h) {
    if (!hw_fb || !dgn_bm_fb_ptr) return;
    int x1 = x + w, y1 = y + h;
    if (x < 0) x = 0;
    if (y < 0) y = 0;
    if (x1 > dgn_bm_fb_w) x1 = dgn_bm_fb_w;
    if (y1 > dgn_bm_fb_h) y1 = dgn_bm_fb_h;
    if (x1 <= x || y1 <= y) return;
    for (int j = y; j < y1; j++) {
        volatile uint32_t* d = hw_fb + (long)j * hw_pitch;
        const uint32_t* s = dgn_bm_fb_ptr + (long)j * dgn_bm_fb_w;
        if (hw_rgb) for (int i = x; i < x1; i++) { uint32_t p = s[i]; d[i] = (p & 0xff00ff00u) | ((p & 0xff) << 16) | ((p >> 16) & 0xff); }
        else for (int i = x; i < x1; i++) d[i] = s[i];
    }
}
void dgn_bm_fb_dirty(int x, int y, int w, int h) {
    if (!hw_fb) return;
    if (!dirty_on) { dx0 = x; dy0 = y; dx1 = x + w; dy1 = y + h; dirty_on = 1; return; }
    if (x < dx0) dx0 = x;
    if (y < dy0) dy0 = y;
    if (x + w > dx1) dx1 = x + w;
    if (y + h > dy1) dy1 = y + h;
}
void dgn_bm_fb_flush(void) {
    if (!hw_fb || !dirty_on) return;
    dirty_on = 0;
    dgn_bm_fb_sync(dx0, dy0, dx1 - dx0, dy1 - dy0);
}
