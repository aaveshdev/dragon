#include <stdint.h>
#include <string.h>

extern uint32_t* dgn_bm_fb_ptr;
extern int dgn_bm_fb_w, dgn_bm_fb_h;

#define CWID 12
#define CHEI 19
static const char* sprite[CHEI] = {
    "X           ", "XX          ", "X.X         ", "X..X        ", "X...X       ", "X....X      ", "X.....X     ",
    "X......X    ", "X.......X   ", "X........X  ", "X.....XXXXX ", "X..X..X     ", "X.X X..X    ", "XX  X..X    ",
    "X    X..X   ", "     X..X   ", "      X..X  ", "      X..X  ", "       XX   " };

extern void dgn_bm_fb_sync(int x, int y, int w, int h);
extern void dgn_bm_fb_dirty(int x, int y, int w, int h);
extern void dgn_bm_fb_flush(void);
static int enabled, shown, lifted, px, py;
static uint32_t under[CWID * CHEI];

static void save_draw(void) {
    for (int j = 0; j < CHEI; j++) for (int i = 0; i < CWID; i++) {
        int x = px + i, y = py + j;
        if (x < 0 || y < 0 || x >= dgn_bm_fb_w || y >= dgn_bm_fb_h) continue;
        uint32_t* p = &dgn_bm_fb_ptr[(long)y * dgn_bm_fb_w + x];
        under[j * CWID + i] = *p;
        char c = sprite[j][i];
        if (c == 'X') *p = 0x000000; else if (c == '.') *p = 0xFFFFFF;
    }
    shown = 1;
    dgn_bm_fb_sync(px, py, CWID, CHEI);
}
static void restore(void) {
    for (int j = 0; j < CHEI; j++) for (int i = 0; i < CWID; i++) {
        int x = px + i, y = py + j;
        if (x < 0 || y < 0 || x >= dgn_bm_fb_w || y >= dgn_bm_fb_h) continue;
        if (sprite[j][i] != ' ') dgn_bm_fb_ptr[(long)y * dgn_bm_fb_w + x] = under[j * CWID + i];
    }
    shown = 0;
    dgn_bm_fb_sync(px, py, CWID, CHEI);
}

int dgn_bm_cur_begin(int x, int y, int w, int h) {
    dgn_bm_fb_dirty(x, y, w, h);
    if (!shown) return 0;
    if (x + w <= px || y + h <= py || x >= px + CWID || y >= py + CHEI) return 0;
    restore(); lifted = 1; return 1;
}
void dgn_bm_cur_end(int was) {
    /* draw the pointer into the shadow first so flushed rows already contain it (avoids blinking) */
    if (was && enabled && dgn_bm_fb_ptr) { save_draw(); }
    dgn_bm_fb_flush();
    if (was && enabled && dgn_bm_fb_ptr) dgn_bm_fb_sync(px, py, CWID, CHEI);
    if (was) lifted = 0;
}
void dgn_bm_cur_enable(int on) {
    if (!dgn_bm_fb_ptr) return;
    if (shown) restore();
    enabled = on;
    if (on) save_draw();
}
void dgn_bm_cur_move(int x, int y) {
    if (!enabled || !dgn_bm_fb_ptr) return;
    if (x == px && y == py && shown) return;
    if (shown) restore();
    px = x; py = y;
    save_draw();
}

void dgn_bm_cur_reset(void) { shown = 0; if (enabled && dgn_bm_fb_ptr) save_draw(); }
