#include <stdint.h>
#include <string.h>
#include <stdlib.h>
#include "font8x13.h"

extern uint32_t* dgn_bm_fb_ptr;
extern int dgn_bm_fb_w, dgn_bm_fb_h;
extern int dgn_bm_cur_begin(int x, int y, int w, int h);
extern void dgn_bm_cur_end(int was);
extern void dgn_bm_cur_reset(void);

#define CW 8
#define CH 13

int dgn_bm_con_on, dgn_bm_con_fbonly, dgn_bm_con_seronly;
static int cols, rows, cx, cy, cursor_want, cursor_shown;
static uint32_t fg = 0xC8CCD4, bg = 0x0B0F1A;
static unsigned char* chr; static uint32_t* cfg; static uint32_t* cbg;

static void blit(int col, int row, unsigned char ch, uint32_t f, uint32_t b) {
    if (ch < 32 || ch > 126) ch = '?';
    const unsigned char* g = dgn_font8x13[ch - 32];
    uint32_t* p = dgn_bm_fb_ptr + (long)row * CH * dgn_bm_fb_w + (long)col * CW;
    for (int r = 0; r < CH; r++, p += dgn_bm_fb_w) {
        unsigned char bits = g[r];
        for (int c = 0; c < CW; c++) p[c] = (bits & (0x80 >> c)) ? f : b;
    }
}
static void draw_cell(int col, int row, int inverse) {
    long i = (long)row * cols + col;
    if (inverse) blit(col, row, chr[i], cbg[i], cfg[i]); else blit(col, row, chr[i], cfg[i], cbg[i]);
}
static void hide_cursor(void) { if (cursor_shown) { draw_cell(cx < cols ? cx : cols - 1, cy, 0); cursor_shown = 0; } }
static void show_cursor(void) { if (cursor_want && !cursor_shown) { draw_cell(cx < cols ? cx : cols - 1, cy, 1); cursor_shown = 1; } }

static void clear_cells(long from, long to) {
    for (long i = from; i < to; i++) { chr[i] = ' '; cfg[i] = fg; cbg[i] = bg; }
}
static void scroll_up(void) {
    memmove(chr, chr + cols, (size_t)(rows - 1) * cols);
    memmove(cfg, cfg + cols, (size_t)(rows - 1) * cols * 4);
    memmove(cbg, cbg + cols, (size_t)(rows - 1) * cols * 4);
    clear_cells((long)(rows - 1) * cols, (long)rows * cols);
    long w = dgn_bm_fb_w, keep = (long)(rows - 1) * CH * w;
    memmove(dgn_bm_fb_ptr, dgn_bm_fb_ptr + (long)CH * w, (size_t)keep * 4);
    for (long i = keep, n = (long)rows * CH * w; i < n; i++) dgn_bm_fb_ptr[i] = bg;
}
static void newline(void) { cx = 0; if (++cy >= rows) { cy = rows - 1; scroll_up(); } }

static void con_out_raw(char c);
void dgn_bm_con_out(char c) {
    if (!dgn_bm_con_on) return;
    int w = dgn_bm_cur_begin(0, 0, dgn_bm_fb_w, dgn_bm_fb_h);
    con_out_raw(c);
    dgn_bm_cur_end(w);
}
static void con_out_raw(char c) {
    hide_cursor();
    if (c == '\n') newline();
    else if (c == '\r') cx = 0;
    else if (c == '\b') { if (cx > 0) cx--; }
    else if (c == '\t') { int n = 8 - (cx & 7); while (n--) con_out_raw(' '); return; }
    else if ((unsigned char)c >= 32 && (unsigned char)c < 127) {
        if (cx >= cols) newline();
        long i = (long)cy * cols + cx; chr[i] = (unsigned char)c; cfg[i] = fg; cbg[i] = bg;
        draw_cell(cx, cy, 0); cx++;
    }
    show_cursor();
}

int dgn_bm_con_init(void) {
    if (!dgn_bm_fb_ptr) return 0;
    cols = dgn_bm_fb_w / CW; rows = dgn_bm_fb_h / CH;
    if (cols < 8 || rows < 4) return 0;
    free(chr); free(cfg); free(cbg);
    chr = malloc((size_t)cols * rows); cfg = malloc((size_t)cols * rows * 4); cbg = malloc((size_t)cols * rows * 4);
    if (!chr || !cfg || !cbg) return 0;
    clear_cells(0, (long)cols * rows);
    for (long i = 0, n = (long)dgn_bm_fb_w * dgn_bm_fb_h; i < n; i++) dgn_bm_fb_ptr[i] = bg;
    cx = cy = 0; cursor_shown = 0; dgn_bm_con_on = 1; dgn_bm_con_fbonly = 0;
    dgn_bm_cur_reset();
    return 1;
}
void dgn_bm_con_off(void) { dgn_bm_con_on = 0; }
void dgn_bm_con_clear(void) {
    if (!dgn_bm_con_on) return;
    int wv = dgn_bm_cur_begin(0, 0, dgn_bm_fb_w, dgn_bm_fb_h);
    cursor_shown = 0; clear_cells(0, (long)cols * rows);
    for (long i = 0, n = (long)dgn_bm_fb_w * dgn_bm_fb_h; i < n; i++) dgn_bm_fb_ptr[i] = bg;
    cx = cy = 0; show_cursor();
    dgn_bm_cur_end(wv);
}
void dgn_bm_con_redraw(void) {
    if (!dgn_bm_con_on) return;
    int wv = dgn_bm_cur_begin(0, 0, dgn_bm_fb_w, dgn_bm_fb_h);
    cursor_shown = 0;
    for (long i = 0, n = (long)dgn_bm_fb_w * dgn_bm_fb_h; i < n; i++) dgn_bm_fb_ptr[i] = bg;
    for (int r = 0; r < rows; r++) for (int c = 0; c < cols; c++) draw_cell(c, r, 0);
    show_cursor();
    dgn_bm_cur_end(wv);
}
void dgn_bm_con_goto(int x, int y) {
    if (!dgn_bm_con_on) return;
    int wv = dgn_bm_cur_begin(0, 0, dgn_bm_fb_w, dgn_bm_fb_h);
    hide_cursor();
    cx = x < 0 ? 0 : x >= cols ? cols - 1 : x; cy = y < 0 ? 0 : y >= rows ? rows - 1 : y;
    show_cursor();
    dgn_bm_cur_end(wv);
}
void dgn_bm_con_color(uint32_t f, long b) { fg = f; if (b >= 0) bg = (uint32_t)b; }
void dgn_bm_con_cursor(int on) { int wv = dgn_bm_cur_begin(0, 0, dgn_bm_fb_w, dgn_bm_fb_h); hide_cursor(); cursor_want = on; show_cursor(); dgn_bm_cur_end(wv); }
int dgn_bm_con_cols(void) { return cols; }
int dgn_bm_con_rows(void) { return rows; }
int dgn_bm_con_x(void) { return cx; }
int dgn_bm_con_y(void) { return cy; }
