#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdlib.h>

#define FWCFG_BASE   0x10100000UL
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
    __asm__ volatile("fence" ::: "memory");
    *(volatile uint64_t*)(FWCFG_BASE + 16) = bs64((uint64_t)(uintptr_t)&dma);
    for (int i = 0; i < 1000000; i++) { if ((bs32(dma.control) & ~1u) == 0) return (bs32(dma.control) & 1) ? -1 : 0; }
    return -1;
}

typedef struct { uint64_t addr; uint32_t fourcc, flags, width, height, stride; } __attribute__((packed)) RamfbCfg;

uint32_t* dgn_bm_fb_ptr;
int dgn_bm_fb_w, dgn_bm_fb_h;

long long dgn_bm_fb_vram(void) { return -1; }
int dgn_bm_fb_init(int w, int h) {
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
/* ramfb scans out dgn_bm_fb_ptr directly: nothing to copy */
void dgn_bm_fb_sync(int x, int y, int w, int h) { (void)x; (void)y; (void)w; (void)h; }
void dgn_bm_fb_dirty(int x, int y, int w, int h) { (void)x; (void)y; (void)w; (void)h; }
void dgn_bm_fb_flush(void) { }
