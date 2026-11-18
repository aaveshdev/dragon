#include <stdint.h>
#include <stddef.h>
#include "font8x13.h"

extern uint32_t* dgn_bm_fb_ptr;
extern int dgn_bm_fb_w, dgn_bm_fb_h;
extern int dgn_bm_cur_begin(int x, int y, int w, int h);
extern void dgn_bm_cur_end(int was);

static uint32_t* cust; static int cw, ch;
#define FW (cust ? cw : dgn_bm_fb_w)
#define FH (cust ? ch : dgn_bm_fb_h)
static int sc_on, sc_x0, sc_y0, sc_x1, sc_y1;
#define CLX0 ((cust || !sc_on) ? 0 : sc_x0)
#define CLY0 ((cust || !sc_on) ? 0 : sc_y0)
#define CLX1 ((cust || !sc_on) ? FW : sc_x1)
#define CLY1 ((cust || !sc_on) ? FH : sc_y1)
void dgn_gfx_clip(int x, int y, int w, int h) {
    int x1 = x + w, y1 = y + h;
    if (x < 0) x = 0; if (y < 0) y = 0; if (x1 > dgn_bm_fb_w) x1 = dgn_bm_fb_w; if (y1 > dgn_bm_fb_h) y1 = dgn_bm_fb_h;
    sc_x0 = x; sc_y0 = y; sc_x1 = x1 > x ? x1 : x; sc_y1 = y1 > y ? y1 : y; sc_on = 1;
}
void dgn_gfx_unclip(void) { sc_on = 0; }
static int scs_stack[8][5]; static int scs_n = 0;
void dgn_gfx_clip_push(int x, int y, int w, int h) {
    if (scs_n < 8) { scs_stack[scs_n][0] = sc_on; scs_stack[scs_n][1] = sc_x0; scs_stack[scs_n][2] = sc_y0; scs_stack[scs_n][3] = sc_x1; scs_stack[scs_n][4] = sc_y1; scs_n++; }
    int x1 = x + w, y1 = y + h;
    if (sc_on) { if (x < sc_x0) x = sc_x0; if (y < sc_y0) y = sc_y0; if (x1 > sc_x1) x1 = sc_x1; if (y1 > sc_y1) y1 = sc_y1; }
    dgn_gfx_clip(x, y, x1 - x, y1 - y);
}
void dgn_gfx_clip_pop(void) {
    if (scs_n > 0) { scs_n--; sc_on = scs_stack[scs_n][0]; sc_x0 = scs_stack[scs_n][1]; sc_y0 = scs_stack[scs_n][2]; sc_x1 = scs_stack[scs_n][3]; sc_y1 = scs_stack[scs_n][4]; }
}
void dgn_gfx_surface(uint32_t* p, int w, int h) { cust = p; cw = w; ch = h; }
int dgn_gfx_tw(void) { return FW; }
int dgn_gfx_custom(void) { return cust != 0; }
int dgn_gfx_th(void) { return FH; }

#include <stdlib.h>
#include <string.h>
double sqrt(double);
static float gx_sqrtf(float x) { return (float)sqrt((double)x); }

__attribute__((weak)) void dgn_smp_parallel(void (*f)(void*, int, int), void* ctx) { f(ctx, 0, 1); }
#define PAR_MIN 20000
static uint32_t* back;
static void gl_flush(void);
#define TP (cust ? cust : back ? back : dgn_bm_fb_ptr)
#define CB(x, y, w, h) ((cust || back) ? 0 : dgn_bm_cur_begin((x), (y), (w), (h)))
#define CE(was) do { if (!cust && !back) dgn_bm_cur_end(was); } while (0)
int dgn_gfx_buffer(int on) {
    if (!on) { if (back) { free(back); back = 0; } gl_flush(); return 1; }
    if (back || !dgn_bm_fb_ptr) return back != 0;
    back = (uint32_t*)malloc((size_t)dgn_bm_fb_w * dgn_bm_fb_h * 4);
    if (!back) return 0;
    memcpy(back, dgn_bm_fb_ptr, (size_t)dgn_bm_fb_w * dgn_bm_fb_h * 4);
    return 1;
}
#ifdef DGN_USER
extern void dgn_gfx_present_sys(void);
void dgn_gfx_present(void) {
    if (back && dgn_bm_fb_ptr) memcpy(dgn_bm_fb_ptr, back, (size_t)dgn_bm_fb_w * dgn_bm_fb_h * 4);
    dgn_gfx_present_sys();
}
void dgn_gfx_present_rect(int x, int y, int w, int h) { (void)x; (void)y; (void)w; (void)h; dgn_gfx_present(); }
#else
typedef struct { uint32_t *dst; const uint32_t* src; int x, y, x1, y1, fw; } PresentJob;
static void present_job(void* c, int part, int parts) {
    PresentJob* p = (PresentJob*)c;
    int rows = p->y1 - p->y, a = p->y + (int)((long)rows * part / parts), b = p->y + (int)((long)rows * (part + 1) / parts);
    for (int j = a; j < b; j++) memcpy(p->dst + (long)j * p->fw + p->x, p->src + (long)j * p->fw + p->x, (size_t)(p->x1 - p->x) * 4);
}
void dgn_gfx_present_rect(int x, int y, int w, int h) {
    if (!back || !dgn_bm_fb_ptr) return;
    int x1 = x + w, y1 = y + h;
    if (x < 0) x = 0; if (y < 0) y = 0; if (x1 > dgn_bm_fb_w) x1 = dgn_bm_fb_w; if (y1 > dgn_bm_fb_h) y1 = dgn_bm_fb_h;
    if (x1 <= x || y1 <= y) return;
    int wv = dgn_bm_cur_begin(x, y, x1 - x, y1 - y);
    PresentJob pj = { dgn_bm_fb_ptr, back, x, y, x1, y1, dgn_bm_fb_w };
    if ((long)(x1 - x) * (y1 - y) >= PAR_MIN) dgn_smp_parallel(present_job, &pj); else present_job(&pj, 0, 1);
    dgn_bm_cur_end(wv);
}
void dgn_gfx_present(void) {
    if (!back || !dgn_bm_fb_ptr) return;
    dgn_gfx_present_rect(0, 0, dgn_bm_fb_w, dgn_bm_fb_h);
}
#endif

typedef struct { uint32_t* t; const uint32_t* src; int fw, stride, dx, dy, x0, y0, x1, y1; } BlitJob;
static void blit_job(void* c, int part, int parts) {
    BlitJob* b = (BlitJob*)c;
    int rows = b->y1 - b->y0, a = b->y0 + (int)((long)rows * part / parts), e = b->y0 + (int)((long)rows * (part + 1) / parts);
    for (int j = a; j < e; j++) memcpy(b->t + (long)(b->dy + j) * b->fw + b->dx + b->x0, b->src + (long)j * b->stride + b->x0, (size_t)(b->x1 - b->x0) * 4);
}
void dgn_gfx_blit(const uint32_t* src, int sw, int sh, int stride, int dx, int dy) {
    uint32_t* t = TP; if (!t) return;
    int x0 = dx < 0 ? -dx : 0, y0 = dy < 0 ? -dy : 0, x1 = sw, y1 = sh;
    if (dx + x0 < CLX0) x0 = CLX0 - dx; if (dy + y0 < CLY0) y0 = CLY0 - dy; if (dx + x1 > CLX1) x1 = CLX1 - dx; if (dy + y1 > CLY1) y1 = CLY1 - dy;
    if (x1 <= x0 || y1 <= y0) return;
    int was = CB(dx + x0, dy + y0, x1 - x0, y1 - y0);
    BlitJob bj = { t, src, FW, stride, dx, dy, x0, y0, x1, y1 };
    if ((long)(x1 - x0) * (y1 - y0) >= PAR_MIN) dgn_smp_parallel(blit_job, &bj); else blit_job(&bj, 0, 1);
    CE(was);
}
#define FWQ q->fw
typedef struct { uint32_t* t; const uint32_t* src; int sw, sh, dx, dy, dw, dh, x0, y0, x1, y1, avg; int fw; } ScaleJob;
static void scale_job(void* c, int part, int parts) {
    ScaleJob* q = (ScaleJob*)c;
    uint32_t* t = q->t; const uint32_t* src = q->src; int sw = q->sw, sh = q->sh, dx = q->dx, dy = q->dy, dw = q->dw, dh = q->dh, x0 = q->x0, x1 = q->x1, avg = q->avg;
    int rows = q->y1 - q->y0, ya = q->y0 + (int)((long)rows * part / parts), yb = q->y0 + (int)((long)rows * (part + 1) / parts);
    for (int j = ya; j < yb; j++) {
        long sy = ((long)j * sh) / dh; if (sy >= sh) sy = sh - 1;
        long sy2 = avg ? (((long)(j + 1) * sh) / dh) - 1 : sy; if (sy2 < sy) sy2 = sy; if (sy2 >= sh) sy2 = sh - 1;
        uint32_t* out = t + (long)(dy + j) * FWQ + dx;
        const uint32_t* r1 = src + sy * sw; const uint32_t* r2 = src + sy2 * sw;
        for (int i = x0; i < x1; i++) {
            long sx = ((long)i * sw) / dw; if (sx >= sw) sx = sw - 1;
            if (!avg) { out[i] = r1[sx]; continue; }
            long sx2 = (((long)(i + 1) * sw) / dw) - 1; if (sx2 < sx) sx2 = sx; if (sx2 >= sw) sx2 = sw - 1;
            uint32_t a = r1[sx], b = r1[sx2], c = r2[sx], d = r2[sx2];
            uint32_t rb = (((a & 0xFF00FF) + (b & 0xFF00FF) + (c & 0xFF00FF) + (d & 0xFF00FF)) >> 2) & 0xFF00FF;
            uint32_t g = (((a & 0x00FF00) + (b & 0x00FF00) + (c & 0x00FF00) + (d & 0x00FF00)) >> 2) & 0x00FF00;
            out[i] = 0xFF000000u | rb | g;
        }
    }
}
int dgn_gfx_alpha;   /* browser images: honour alpha (see kernel overlay) */
void dgn_gfx_blit_scaled(const uint32_t* src, int sw, int sh, int dx, int dy, int dw, int dh, int cx, int cy, int cw, int ch) {
    uint32_t* t = TP; if (!t || sw <= 0 || sh <= 0 || dw <= 0 || dh <= 0) return;
    int x0 = 0, y0 = 0, x1 = dw, y1 = dh;
    if (cw > 0 && ch > 0) { if (dx + x0 < cx) x0 = cx - dx; if (dy + y0 < cy) y0 = cy - dy; if (dx + x1 > cx + cw) x1 = cx + cw - dx; if (dy + y1 > cy + ch) y1 = cy + ch - dy; }
    if (dx + x0 < CLX0) x0 = CLX0 - dx; if (dy + y0 < CLY0) y0 = CLY0 - dy; if (dx + x1 > CLX1) x1 = CLX1 - dx; if (dy + y1 > CLY1) y1 = CLY1 - dy;
    if (x1 <= x0 || y1 <= y0) return;
    int was = CB(dx + x0, dy + y0, x1 - x0, y1 - y0);
    ScaleJob sj = { t, src, sw, sh, dx, dy, dw, dh, x0, y0, x1, y1, (sw >= dw * 2 || sh >= dh * 2), FW };
    if ((long)(x1 - x0) * (y1 - y0) >= PAR_MIN) dgn_smp_parallel(scale_job, &sj); else scale_job(&sj, 0, 1);
    CE(was);
}


void dgn_gfx_resize(uint32_t* dst, int dw, int dh, const uint32_t* src, int stride, int sx, int sy, int sw, int sh) {
    if (!dst || !src || dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0) return;
    if (sw == dw && sh == dh) { for (int j = 0; j < dh; j++) memcpy(dst + (long)j * dw, src + (long)(sy + j) * stride + sx, (size_t)dw * 4); return; }
    int box = (sw > dw || sh > dh);
    for (int j = 0; j < dh; j++) {
        long y0 = ((long)j * sh) / dh, y1 = (((long)(j + 1) * sh) + dh - 1) / dh; if (y1 <= y0) y1 = y0 + 1; if (y1 > sh) y1 = sh;
        long fy = (((long)(2 * j + 1) * sh) << 15) / dh - (1L << 15); if (fy < 0) fy = 0;
        long iy = fy >> 16, wy = fy & 0xFFFF; long iy2 = iy + 1 < sh ? iy + 1 : sh - 1; if (iy >= sh) iy = sh - 1;
        for (int i = 0; i < dw; i++) {
            long x0 = ((long)i * sw) / dw, x1 = (((long)(i + 1) * sw) + dw - 1) / dw; if (x1 <= x0) x1 = x0 + 1; if (x1 > sw) x1 = sw;
            uint32_t out;
            if (box) {
                long A = 0, R = 0, G = 0, B = 0, n = 0;
                long ys = (y1 - y0 > 4) ? (y1 - y0) / 4 : 1, xs = (x1 - x0 > 4) ? (x1 - x0) / 4 : 1;
                for (long y = y0; y < y1; y += ys) { const uint32_t* r = src + (long)(sy + y) * stride + sx;
                    for (long x = x0; x < x1; x += xs) { uint32_t p = r[x]; long a = p >> 24; A += a; R += ((p >> 16) & 255) * a; G += ((p >> 8) & 255) * a; B += (p & 255) * a; n++; } }
                if (A == 0) out = 0; else out = ((uint32_t)(A / n) << 24) | ((uint32_t)(R / A) << 16) | ((uint32_t)(G / A) << 8) | (uint32_t)(B / A);
            } else {
                long fx = (((long)(2 * i + 1) * sw) << 15) / dw - (1L << 15); if (fx < 0) fx = 0;
                long ix = fx >> 16, wx = fx & 0xFFFF; long ix2 = ix + 1 < sw ? ix + 1 : sw - 1; if (ix >= sw) ix = sw - 1;
                const uint32_t* r1 = src + (long)(sy + iy) * stride + sx; const uint32_t* r2 = src + (long)(sy + iy2) * stride + sx;
                uint32_t p[4] = { r1[ix], r1[ix2], r2[ix], r2[ix2] };
                long wgt[4] = { ((65536 - wx) * (65536 - wy)) >> 16, (wx * (65536 - wy)) >> 16, ((65536 - wx) * wy) >> 16, (wx * wy) >> 16 };
                long A = 0, R = 0, G = 0, B = 0;
                for (int k = 0; k < 4; k++) { long a = (p[k] >> 24) * wgt[k]; A += a; R += ((p[k] >> 16) & 255) * a; G += ((p[k] >> 8) & 255) * a; B += (p[k] & 255) * a; }
                if (A == 0) out = 0; else out = ((uint32_t)(A >> 16) << 24) | ((uint32_t)(R / A) << 16) | ((uint32_t)(G / A) << 8) | (uint32_t)(B / A);
            }
            dst[(long)j * dw + i] = out;
        }
    }
}
void dgn_gfx_rect(int x, int y, int w, int h, uint32_t c);
void dgn_gfx_clear(uint32_t c) { if (!TP) return; if (sc_on && !cust) { dgn_gfx_rect(sc_x0, sc_y0, sc_x1 - sc_x0, sc_y1 - sc_y0, c); return; } int w = CB(0, 0, FW, FH); uint32_t* t = TP; for (long i = 0, n = (long)FW * FH; i < n; i++) t[i] = c; CE(w); }
void dgn_gfx_pixel(int x, int y, uint32_t c) { if (TP && x >= CLX0 && y >= CLY0 && x < CLX1 && y < CLY1) { int w = CB(x, y, 1, 1); TP[(long)y * FW + x] = c; CE(w); } }
uint32_t dgn_gfx_get(int x, int y) { return (TP && x >= 0 && y >= 0 && x < FW && y < FH) ? TP[(long)y * FW + x] : 0; }
void dgn_gfx_rect(int x, int y, int w, int h, uint32_t c) {
    if (!TP) return;
    int x1 = x + w, y1 = y + h; if (x < CLX0) x = CLX0; if (y < CLY0) y = CLY0; if (x1 > CLX1) x1 = CLX1; if (y1 > CLY1) y1 = CLY1;
    if (x1 <= x || y1 <= y) return;
    int wv = CB(x, y, x1 - x, y1 - y);
    for (int j = y; j < y1; j++) { uint32_t* row = TP + (long)j * FW; for (int i = x; i < x1; i++) row[i] = c; }
    CE(wv);
}
void dgn_gfx_box(int x, int y, int w, int h, uint32_t c) {
    dgn_gfx_rect(x, y, w, 1, c); dgn_gfx_rect(x, y + h - 1, w, 1, c); dgn_gfx_rect(x, y, 1, h, c); dgn_gfx_rect(x + w - 1, y, 1, h, c);
}
void dgn_gfx_line(int x0, int y0, int x1, int y1, uint32_t c) {
    int dx = x1 > x0 ? x1 - x0 : x0 - x1, sx = x0 < x1 ? 1 : -1, dy = -(y1 > y0 ? y1 - y0 : y0 - y1), sy = y0 < y1 ? 1 : -1, err = dx + dy;
    for (;;) { dgn_gfx_pixel(x0, y0, c); if (x0 == x1 && y0 == y1) break; int e2 = 2 * err; if (e2 >= dy) { err += dy; x0 += sx; } if (e2 <= dx) { err += dx; y0 += sy; } }
}
void dgn_gfx_circle(int cx, int cy, int r, uint32_t c, int fill) {
    int x = r, y = 0, d = 1 - r;
    while (x >= y) {
        if (fill) { dgn_gfx_rect(cx - x, cy + y, 2 * x + 1, 1, c); dgn_gfx_rect(cx - x, cy - y, 2 * x + 1, 1, c); dgn_gfx_rect(cx - y, cy + x, 2 * y + 1, 1, c); dgn_gfx_rect(cx - y, cy - x, 2 * y + 1, 1, c); }
        else { dgn_gfx_pixel(cx + x, cy + y, c); dgn_gfx_pixel(cx - x, cy + y, c); dgn_gfx_pixel(cx + x, cy - y, c); dgn_gfx_pixel(cx - x, cy - y, c);
               dgn_gfx_pixel(cx + y, cy + x, c); dgn_gfx_pixel(cx - y, cy + x, c); dgn_gfx_pixel(cx + y, cy - x, c); dgn_gfx_pixel(cx - y, cy - x, c); }
        y++; if (d < 0) d += 2 * y + 1; else { x--; d += 2 * (y - x) + 1; }
    }
}

void dgn_gfx_char(int x, int y, int cc, uint32_t fg, long bg, int scale) {
    if (!TP) return;
    if (cc < 32 || cc > 126) cc = '?';
    if (scale < 1) scale = 1;
    if (x >= CLX1 || y >= CLY1 || x + 8 * scale <= CLX0 || y + 13 * scale <= CLY0) return;
    const unsigned char* g = dgn_font8x13[cc - 32];
    int was = CB(x, y, 8 * scale, 13 * scale);
    uint32_t* t = TP;
    for (int r = 0; r < 13; r++) for (int sy = 0; sy < scale; sy++) {
        int yy = y + r * scale + sy; if (yy < CLY0 || yy >= CLY1) continue;
        uint32_t* row = t + (long)yy * FW;
        for (int col = 0; col < 8; col++) {
            int on = g[r] & (0x80 >> col);
            if (!on && bg < 0) continue;
            uint32_t c = on ? fg : (uint32_t)bg;
            for (int sx = 0; sx < scale; sx++) { int xx = x + col * scale + sx; if (xx >= CLX0 && xx < CLX1) row[xx] = c; }
        }
    }
    CE(was);
}
void dgn_gfx_text(int x, int y, const char* s, uint32_t fg, long bg, int scale) {
    if (scale < 1) scale = 1;
    for (int i = 0; s[i]; i++) dgn_gfx_char(x + i * 8 * scale, y, (unsigned char)s[i], fg, bg, scale);
}

void dgn_gfx_scroll(int dy, uint32_t bg) {
    if (!TP || dy <= 0) return; if (dy > FH) dy = FH;
    long keep = (long)(FH - dy) * FW;
    int wv = CB(0, 0, FW, FH);
    uint32_t* t = TP;
    for (long i = 0; i < keep; i++) t[i] = t[i + (long)dy * FW];
    for (long i = keep, n = (long)FH * FW; i < n; i++) t[i] = bg;
    CE(wv);
}

static inline uint32_t gx_mix(uint32_t d, uint32_t s, int a) {
    a += a >> 7;
    uint32_t rb = (((d & 0xFF00FF) * (256 - a) + (s & 0xFF00FF) * a) >> 8) & 0xFF00FF;
    uint32_t g = (((d & 0x00FF00) * (256 - a) + (s & 0x00FF00) * a) >> 8) & 0x00FF00;
    return rb | g;
}

int dgn_gfx_fontadd(const void* d, int n, int s) { (void)d; (void)n; (void)s; return -1; }
int dgn_gfx_twidth(const char* str, int size, int flags) { (void)size; (void)flags; int n = 0; while (str[n]) n++; return n * 8; }
int dgn_gfx_tmetric(int size, int flags, int which) { (void)size; (void)flags; return which ? 11 : 13; }
int dgn_gfx_ttext(int x, int y, const char* str, uint32_t color, int size, int flags) {
    (void)size; (void)flags; dgn_gfx_text(x, y, str, color, -1, 1); return dgn_gfx_twidth(str, size, flags);
}
static inline int gx_clip(int* x, int* y, int* w, int* h) {
    int x1 = *x + *w, y1 = *y + *h;
    if (*x < CLX0) *x = CLX0; if (*y < CLY0) *y = CLY0; if (x1 > CLX1) x1 = CLX1; if (y1 > CLY1) y1 = CLY1;
    if (x1 <= *x || y1 <= *y) return 0;
    *w = x1 - *x; *h = y1 - *y; return 1;
}
void dgn_gfx_blend(int x, int y, int w, int h, uint32_t c, int a) {
    uint32_t* t = TP; if (!t || a <= 0) return;
    if (!gx_clip(&x, &y, &w, &h)) return;
    if (a > 255) a = 255;
    int wv = CB(x, y, w, h);
    for (int j = y; j < y + h; j++) { uint32_t* row = t + (long)j * FW; for (int i = x; i < x + w; i++) row[i] = gx_mix(row[i], c, a); }
    CE(wv);
}
static inline int gx_cover(int i, int j, int w, int h, int r) {
    int cx, cy;
    if (i < r) cx = r - i; else if (i >= w - r) cx = i - (w - r) + 1; else return 255;
    if (j < r) cy = r - j; else if (j >= h - r) cy = j - (h - r) + 1; else return 255;
    float dx = (float)cx - 0.5f, dy = (float)cy - 0.5f;
    float d = gx_sqrtf(dx * dx + dy * dy);
    float cv = (float)r - d + 0.5f;
    if (cv >= 1.0f) return 255; if (cv <= 0.0f) return 0;
    return (int)(cv * 255.0f);
}
void dgn_gfx_rrect(int x, int y, int w, int h, int r, uint32_t c, int a) {
    uint32_t* t = TP; if (!t || a <= 0 || w <= 0 || h <= 0) return;
    if (r * 2 > w) r = w / 2; if (r * 2 > h) r = h / 2; if (r < 0) r = 0;
    if (a > 255) a = 255;
    int ox = x, oy = y, ow = w, oh = h;
    if (!gx_clip(&x, &y, &w, &h)) return;
    int wv = CB(x, y, w, h);
    int xe = x + w;
    int ml = ox + r, mr = ox + ow - r;
    for (int j = y; j < y + h; j++) {
        uint32_t* row = t + (long)j * FW; int lj = j - oy;
        int corner = r > 0 && (lj < r || lj >= oh - r);
        int i = x;
        if (corner) {
            int e = xe < ml ? xe : ml;
            for (; i < e; i++) { int cv = gx_cover(i - ox, lj, ow, oh, r); if (cv) row[i] = gx_mix(row[i], c, a * cv / 255); }
        }
        int me = (corner && xe > mr) ? mr : xe;
        if (a >= 255) { for (; i < me; i++) row[i] = c; }
        else { for (; i < me; i++) row[i] = gx_mix(row[i], c, a); }
        if (corner) {
            for (; i < xe; i++) { int cv = gx_cover(i - ox, lj, ow, oh, r); if (cv) row[i] = gx_mix(row[i], c, a * cv / 255); }
        }
    }
    CE(wv);
}
void dgn_gfx_gradient(int x, int y, int w, int h, uint32_t c0, uint32_t c1, int dir) {
    uint32_t* t = TP; if (!t) return;
    int ox = x, oy = y, ow = w, oh = h;
    if (!gx_clip(&x, &y, &w, &h)) return;
    int wv = CB(x, y, w, h);
    int dw = ow > 1 ? ow - 1 : 1, dh = oh > 1 ? oh - 1 : 1;
    int* tx = 0;
    if (dir != 0) { tx = (int*)malloc((size_t)w * sizeof(int)); if (!tx) { CE(wv); return; } for (int i = 0; i < w; i++) tx[i] = (x + i - ox) * 255 / dw; }
    for (int j = y; j < y + h; j++) {
        uint32_t* row = t + (long)j * FW;
        if (dir == 0) { uint32_t c = gx_mix(c0, c1, (j - oy) * 255 / dh); for (int i = x; i < x + w; i++) row[i] = c; }
        else if (dir == 1) { for (int i = 0; i < w; i++) row[x + i] = gx_mix(c0, c1, tx[i]); }
        else { int ty = (j - oy) * 255 / dh; for (int i = 0; i < w; i++) row[x + i] = gx_mix(c0, c1, (tx[i] + ty) / 2); }
    }
    free(tx);
    CE(wv);
}
void dgn_gfx_glow(int cx, int cy, int r, uint32_t c, int a) {
    uint32_t* t = TP; if (!t || r <= 0) return;
    int x = cx - r, y = cy - r, w = 2 * r + 1, h = 2 * r + 1;
    if (!gx_clip(&x, &y, &w, &h)) return;
    long r2 = (long)r * r;
    int wv = CB(x, y, w, h);
    for (int j = y; j < y + h; j++) {
        uint32_t* row = t + (long)j * FW; long dy = j - cy;
        for (int i = x; i < x + w; i++) {
            long d2 = (long)(i - cx) * (i - cx) + dy * dy;
            if (d2 >= r2) continue;
            long f = (r2 - d2) * 256 / r2;
            int al = (int)((long)a * f * f >> 16);
            if (al > 0) row[i] = gx_mix(row[i], c, al);
        }
    }
    CE(wv);
}
void dgn_gfx_aaline(float x0, float y0, float x1, float y1, float w, uint32_t c, int a) {
    uint32_t* t = TP; if (!t) return;
    float hw = w * 0.5f;
    int bx = (int)((x0 < x1 ? x0 : x1) - hw - 1), by = (int)((y0 < y1 ? y0 : y1) - hw - 1);
    int bw = (int)((x0 < x1 ? x1 : x0) - (x0 < x1 ? x0 : x1) + w + 3), bh = (int)((y0 < y1 ? y1 : y0) - (y0 < y1 ? y0 : y1) + w + 3);
    int qx = bx, qy = by, qw = bw, qh = bh;
    if (!gx_clip(&qx, &qy, &qw, &qh)) return;
    int wv = CB(qx, qy, qw, qh);
    float dx = x1 - x0, dy = y1 - y0, len2 = dx * dx + dy * dy;
    for (int j = qy; j < qy + qh; j++) {
        uint32_t* row = t + (long)j * FW;
        for (int i = qx; i < qx + qw; i++) {
            float px = i + 0.5f - x0, py = j + 0.5f - y0;
            float u = len2 > 0 ? (px * dx + py * dy) / len2 : 0; if (u < 0) u = 0; if (u > 1) u = 1;
            float ex = px - u * dx, ey = py - u * dy;
            float d = gx_sqrtf(ex * ex + ey * ey);
            float cov = hw + 0.5f - d; if (cov <= 0) continue; if (cov > 1) cov = 1;
            int al = (int)(a * cov); if (al > 0) row[i] = gx_mix(row[i], c, al);
        }
    }
    CE(wv);
}
void dgn_gfx_aaring(float cxf, float cyf, float r, float th, uint32_t c, int a) {
    uint32_t* t = TP; if (!t) return;
    float ro = r + th * 0.5f, ri = r - th * 0.5f; if (ri < 0) ri = 0;
    int bx = (int)(cxf - ro - 1), by = (int)(cyf - ro - 1), bw = (int)(2 * ro + 3), bh = bw;
    int qx = bx, qy = by, qw = bw, qh = bh;
    if (!gx_clip(&qx, &qy, &qw, &qh)) return;
    int wv = CB(qx, qy, qw, qh);
    for (int j = qy; j < qy + qh; j++) {
        uint32_t* row = t + (long)j * FW;
        for (int i = qx; i < qx + qw; i++) {
            float ex = i + 0.5f - cxf, ey = j + 0.5f - cyf;
            float d = gx_sqrtf(ex * ex + ey * ey);
            float cov = ro + 0.5f - d, c2 = d - (ri - 0.5f);
            if (c2 < cov) cov = c2;
            if (cov <= 0) continue; if (cov > 1) cov = 1;
            int al = (int)(a * cov); if (al > 0) row[i] = gx_mix(row[i], c, al);
        }
    }
    CE(wv);
}
void dgn_gfx_aadisc(float cxf, float cyf, float r, uint32_t c, int a) {
    uint32_t* t = TP; if (!t) return;
    int bx = (int)(cxf - r - 1), by = (int)(cyf - r - 1), bw = (int)(2 * r + 3), bh = bw;
    int qx = bx, qy = by, qw = bw, qh = bh;
    if (!gx_clip(&qx, &qy, &qw, &qh)) return;
    int wv = CB(qx, qy, qw, qh);
    for (int j = qy; j < qy + qh; j++) {
        uint32_t* row = t + (long)j * FW;
        for (int i = qx; i < qx + qw; i++) {
            float ex = i + 0.5f - cxf, ey = j + 0.5f - cyf;
            float cov = r + 0.5f - gx_sqrtf(ex * ex + ey * ey);
            if (cov <= 0) continue; if (cov > 1) cov = 1;
            int al = (int)(a * cov); if (al > 0) row[i] = gx_mix(row[i], c, al);
        }
    }
    CE(wv);
}

void dgn_gfx_sprite_make(uint32_t* dst, const uint32_t* b, const uint32_t* w, int n) {
    for (int i = 0; i < n; i++) {
        uint32_t bb = b[i], ww = w[i];
        int a = 255 - (int)((ww >> 8) & 255) + (int)((bb >> 8) & 255);
        if (a < 0) a = 0; if (a > 255) a = 255;
        uint32_t r = (bb >> 16) & 255, g = (bb >> 8) & 255, bl = bb & 255;
        if (a > 0 && a < 255) { r = r * 255 / a; g = g * 255 / a; bl = bl * 255 / a; if (r > 255) r = 255; if (g > 255) g = 255; if (bl > 255) bl = 255; }
        dst[i] = ((uint32_t)a << 24) | (r << 16) | (g << 8) | bl;
    }
}
void dgn_gfx_sprite(const uint32_t* src, int sw, int sh, int dx, int dy) {
    uint32_t* t = TP; if (!t) return;
    int x0 = 0, y0 = 0, x1 = sw, y1 = sh;
    if (dx + x0 < CLX0) x0 = CLX0 - dx; if (dy + y0 < CLY0) y0 = CLY0 - dy; if (dx + x1 > CLX1) x1 = CLX1 - dx; if (dy + y1 > CLY1) y1 = CLY1 - dy;
    if (x1 <= x0 || y1 <= y0) return;
    int was = CB(dx + x0, dy + y0, x1 - x0, y1 - y0);
    for (int j = y0; j < y1; j++) {
        uint32_t* row = t + (long)(dy + j) * FW + dx; const uint32_t* sr = src + (long)j * sw;
        for (int i = x0; i < x1; i++) {
            uint32_t p = sr[i]; uint32_t a = p >> 24;
            if (a == 255) row[i] = p & 0xFFFFFF; else if (a) row[i] = gx_mix(row[i], p, (int)a);
        }
    }
    CE(was);
}
void dgn_gfx_shadow(int x, int y, int w, int h, int s, int a, int dy, int r) {
    uint32_t* t = TP; if (!t || s <= 0) return;
    if (r < 0) r = 0; if (r * 2 > w) r = w / 2; if (r * 2 > h) r = h / 2;
    int sy = y + dy;
    int ex = x - s, ey = (sy < y ? sy : y) - s, ew = w + 2 * s, eh = h + (dy < 0 ? -dy : dy) + 2 * s;
    int qx = ex, qy = ey, qw = ew, qh = eh;
    if (!gx_clip(&qx, &qy, &qw, &qh)) return;
    int lim = s + r, s2 = lim * lim;
    uint8_t* tab = (uint8_t*)malloc((size_t)s2 + 1); if (!tab) return;
    for (int k = 0; k < s2; k++) { float d = gx_sqrtf((float)k) - (float)r; if (d < 0) d = 0; float f = 1.0f - d / (float)s; int al = (int)((float)a * f * f * f); tab[k] = al > 255 ? 255 : al < 0 ? 0 : al; }
    int cxl = x + r, cxr = x + w - r - 1, cyt = sy + r, cyb = sy + h - r - 1;
    int wv = CB(qx, qy, qw, qh);
    for (int j = qy; j < qy + qh; j++) {
        uint32_t* row = t + (long)j * FW;
        int dyy = j < cyt ? cyt - j : j > cyb ? j - cyb : 0;
        int dy2 = dyy * dyy;
        int win_y = j >= y && j < y + h;
        int wcorner = win_y && (j - y < r || j - y >= h - r);
        for (int i = qx; i < qx + qw; i++) {
            if (win_y) {
                if (!wcorner) { if (i >= x && i < x + w) { i = x + w - 1; continue; } }
                else {
                    if (i >= x + r && i < x + w - r) { i = x + w - r - 1; continue; }
                    if (i >= x && i < x + w && gx_cover(i - x, j - y, w, h, r) >= 255) continue;
                }
            }
            int dx = i < cxl ? cxl - i : i > cxr ? i - cxr : 0;
            int k = dx * dx + dy2;
            if (k >= s2) continue;
            int al = tab[k];
            if (al > 0) row[i] = gx_mix(row[i], 0, al);
        }
    }
    free(tab);
    CE(wv);
}
void dgn_gfx_blur(int x, int y, int w, int h, int rad, int passes) {
    uint32_t* t = TP; if (!t || rad < 1 || w < 2 || h < 2) return;
    if (!gx_clip(&x, &y, &w, &h)) return;
    int m = w > h ? w : h;
    uint32_t* tmp = (uint32_t*)malloc((size_t)m * 4); if (!tmp) return;
    int wv = CB(x, y, w, h);
    for (int p = 0; p < passes; p++) {
        for (int j = y; j < y + h; j++) {
            uint32_t* row = t + (long)j * FW + x;
            long sr = 0, sg = 0, sb = 0; int n = 0;
            for (int k = 0; k <= rad && k < w; k++) { uint32_t v = row[k]; sr += (v >> 16) & 255; sg += (v >> 8) & 255; sb += v & 255; n++; }
            for (int i = 0; i < w; i++) {
                tmp[i] = ((uint32_t)(sr / n) << 16) | ((uint32_t)(sg / n) << 8) | (uint32_t)(sb / n);
                int add = i + rad + 1, rem = i - rad;
                if (add < w) { uint32_t v = row[add]; sr += (v >> 16) & 255; sg += (v >> 8) & 255; sb += v & 255; n++; }
                if (rem >= 0) { uint32_t v = row[rem]; sr -= (v >> 16) & 255; sg -= (v >> 8) & 255; sb -= v & 255; n--; }
            }
            memcpy(row, tmp, (size_t)w * 4);
        }
        for (int i = x; i < x + w; i++) {
            uint32_t* col = t + (long)y * FW + i;
            long sr = 0, sg = 0, sb = 0; int n = 0;
            for (int k = 0; k <= rad && k < h; k++) { uint32_t v = col[(long)k * FW]; sr += (v >> 16) & 255; sg += (v >> 8) & 255; sb += v & 255; n++; }
            for (int j = 0; j < h; j++) {
                tmp[j] = ((uint32_t)(sr / n) << 16) | ((uint32_t)(sg / n) << 8) | (uint32_t)(sb / n);
                int add = j + rad + 1, rem = j - rad;
                if (add < h) { uint32_t v = col[(long)add * FW]; sr += (v >> 16) & 255; sg += (v >> 8) & 255; sb += v & 255; n++; }
                if (rem >= 0) { uint32_t v = col[(long)rem * FW]; sr -= (v >> 16) & 255; sg -= (v >> 8) & 255; sb -= v & 255; n--; }
            }
            for (int j = 0; j < h; j++) col[(long)j * FW] = tmp[j];
        }
    }
    free(tmp);
    CE(wv);
}

void dgn_gfx_saturate(int x, int y, int w, int h, int pct) {
    uint32_t* t = TP; if (!t || pct == 100) return;
    if (!gx_clip(&x, &y, &w, &h)) return;
    int wv = CB(x, y, w, h);
    for (int j = y; j < y + h; j++) {
        uint32_t* row = t + (long)j * FW;
        for (int i = x; i < x + w; i++) {
            uint32_t v = row[i]; int r = (v >> 16) & 255, g = (v >> 8) & 255, b = v & 255;
            int gr = (r * 54 + g * 183 + b * 19) >> 8;
            r = gr + (r - gr) * pct / 100; g = gr + (g - gr) * pct / 100; b = gr + (b - gr) * pct / 100;
            if (r < 0) r = 0; if (r > 255) r = 255; if (g < 0) g = 0; if (g > 255) g = 255; if (b < 0) b = 0; if (b > 255) b = 255;
            row[i] = ((uint32_t)r << 16) | ((uint32_t)g << 8) | (uint32_t)b;
        }
    }
    CE(wv);
}
void dgn_gfx_gpill(int x, int y, int w, int h, int r, uint32_t c0, uint32_t c1, int a) {
    uint32_t* t = TP; if (!t || a <= 0 || w <= 0 || h <= 0) return;
    if (r * 2 > w) r = w / 2; if (r * 2 > h) r = h / 2; if (r < 0) r = 0;
    if (a > 255) a = 255;
    int ox = x, oy = y, ow = w, oh = h;
    if (!gx_clip(&x, &y, &w, &h)) return;
    int wv = CB(x, y, w, h);
    int dw = ow > 1 ? ow - 1 : 1;
    for (int j = y; j < y + h; j++) {
        uint32_t* row = t + (long)j * FW; int lj = j - oy;
        for (int i = x; i < x + w; i++) {
            int cv = (r > 0 && (lj < r || lj >= oh - r)) ? gx_cover(i - ox, lj, ow, oh, r) : ((r > 0 && ((i - ox) < r || (i - ox) >= ow - r)) ? gx_cover(i - ox, lj, ow, oh, r) : 255);
            if (!cv) continue;
            uint32_t col = gx_mix(c0, c1, (i - ox) * 255 / dw);
            row[i] = gx_mix(row[i], col, a * cv / 255);
        }
    }
    CE(wv);
}
static inline int gx_cover_c(int i, int j, int w, int h, int r, int corners) {
    if (i < 0 || j < 0 || i >= w || j >= h) return 0;
    int cx, cy, bit;
    if (i < r) { cx = r - i; bit = 0; } else if (i >= w - r) { cx = i - (w - r) + 1; bit = 1; } else return 255;
    if (j < r) cy = r - j; else if (j >= h - r) { cy = j - (h - r) + 1; bit += 2; } else return 255;
    if (!(corners & (1 << bit))) return 255;
    float dx = (float)cx - 0.5f, dy = (float)cy - 0.5f;
    float cv = (float)r - gx_sqrtf(dx * dx + dy * dy) + 0.5f;
    if (cv >= 1.0f) return 255; if (cv <= 0.0f) return 0;
    return (int)(cv * 255.0f);
}
typedef struct { const uint32_t* src; int stride, h; uint32_t tint; int ta; uint32_t* buf; int* lo; int* hi; unsigned age; } GlCache;
#define GL_N 5
static GlCache glc[GL_N];
static unsigned gl_age;
static GlCache* gl_get(const uint32_t* src, int stride, uint32_t tint, int ta) {
    int h = dgn_bm_fb_h, old = 0;
    for (int k = 0; k < GL_N; k++) {
        if (glc[k].buf && glc[k].src == src && glc[k].stride == stride && glc[k].tint == tint && glc[k].ta == ta && glc[k].h == h) { glc[k].age = ++gl_age; return &glc[k]; }
        if (!glc[k].buf) old = k; else if (glc[old].buf && glc[k].age < glc[old].age) old = k;
    }
    GlCache* g = &glc[old];
    if (g->buf) { free(g->buf); free(g->lo); free(g->hi); g->buf = 0; }
    g->buf = (uint32_t*)malloc((size_t)stride * h * 4); g->lo = (int*)malloc((size_t)h * sizeof(int)); g->hi = (int*)malloc((size_t)h * sizeof(int));
    if (!g->buf || !g->lo || !g->hi) { free(g->buf); free(g->lo); free(g->hi); g->buf = 0; return 0; }
    for (int j = 0; j < h; j++) { g->lo[j] = 0; g->hi[j] = 0; }
    g->src = src; g->stride = stride; g->h = h; g->tint = tint; g->ta = ta; g->age = ++gl_age;
    return g;
}
static void gl_fill(GlCache* g, int j, int a, int b) {
    int lo = g->hi[j] > g->lo[j] ? (a < g->lo[j] ? a : g->lo[j]) : a;
    int hi = g->hi[j] > g->lo[j] ? (b > g->hi[j] ? b : g->hi[j]) : b;
    if (g->hi[j] > g->lo[j] && lo >= g->lo[j] && hi <= g->hi[j]) return;
    const uint32_t* sr = g->src + (long)j * g->stride; uint32_t* d = g->buf + (long)j * g->stride;
    for (int i = lo; i < hi; i++) d[i] = gx_mix(sr[i], g->tint, g->ta);
    g->lo[j] = lo; g->hi[j] = hi;
}
static void gl_flush(void) { for (int k = 0; k < GL_N; k++) { if (glc[k].buf) { free(glc[k].buf); free(glc[k].lo); free(glc[k].hi); } glc[k].buf = 0; } }
void dgn_gfx_glass(const uint32_t* src, int stride, int x, int y, int w, int h, int r, uint32_t tint, int ta, int corners) {
    uint32_t* t = TP; if (!t || !src || w <= 0 || h <= 0) return;
    if (r * 2 > w) r = w / 2; if (r * 2 > h) r = h / 2; if (r < 0) r = 0;
    int ox = x, oy = y, fw = w, fh = h;
    if (!gx_clip(&x, &y, &w, &h)) return;
    int wv = CB(x, y, w, h);
    GlCache* gc = cust ? 0 : gl_get(src, stride, tint, ta);
    int xe = x + w;
    for (int j = y; j < y + h; j++) {
        uint32_t* row = t + (long)j * FW; const uint32_t* sr = src + (long)j * stride;
        int corner_row = r > 0 && (j - oy < r || j - oy >= fh - r);
        const uint32_t* cb = 0;
        if (gc) { gl_fill(gc, j, x, xe); cb = gc->buf + (long)j * gc->stride; }
        if (!corner_row) {
            if (cb) memcpy(row + x, cb + x, (size_t)w * 4);
            else for (int i = x; i < xe; i++) row[i] = gx_mix(sr[i], tint, ta);
            continue;
        }
        for (int i = x; i < xe; i++) {
            int cv = 255;
            int li = i - ox;
            if (li < r || li >= fw - r) cv = gx_cover_c(li, j - oy, fw, fh, r, corners);
            if (!cv) continue;
            uint32_t c = cb ? cb[i] : gx_mix(sr[i], tint, ta);
            row[i] = cv == 255 ? c : gx_mix(row[i], c, cv);
        }
    }
    CE(wv);
}
void dgn_gfx_glass_warm(const uint32_t* src, int stride, uint32_t tint, int ta) {
    if (cust) return;
    if (ta == 0) { gl_flush(); return; }
    GlCache* g = gl_get(src, stride, tint, ta); if (!g) return;
    for (int j = 0; j < g->h; j++) gl_fill(g, j, 0, dgn_bm_fb_w);
}
void dgn_gfx_rframe(int x, int y, int w, int h, int r, uint32_t c, int a, int corners) {
    uint32_t* t = TP; if (!t || w < 3 || h < 3 || a <= 0) return;
    if (r * 2 > w) r = w / 2; if (r * 2 > h) r = h / 2; if (r < 1) r = 1;
    if (a > 255) a = 255;
    int wv = CB(x, y, w, h);
    for (int j = 0; j < h; j++) {
        int yy = y + j; if (yy < CLY0 || yy >= CLY1) continue;
        uint32_t* row = t + (long)yy * FW;
        int edge = j == 0 || j == h - 1;
        int crow = j < r || j >= h - r;
        if (!crow) {
            int xa = x, xb = x + w - 1;
            if (xa >= CLX0 && xa < CLX1) row[xa] = gx_mix(row[xa], c, a);
            if (xb >= CLX0 && xb < CLX1) row[xb] = gx_mix(row[xb], c, a);
            continue;
        }
        if (edge) {
            int lo = x + r < CLX0 ? CLX0 : x + r, hi = x + w - r > CLX1 ? CLX1 : x + w - r;
            for (int k = lo; k < hi; k++) row[k] = gx_mix(row[k], c, a);
        }
        for (int s = 0; s < 2; s++) {
            int i0 = s ? w - r : 0, i1 = s ? w : r;
            for (int i = i0; i < i1; i++) {
                int xx = x + i; if (xx < CLX0 || xx >= CLX1) continue;
                int outer = gx_cover_c(i, j, w, h, r, corners);
                int inner = gx_cover_c(i - 1, j - 1, w - 2, h - 2, r > 1 ? r - 1 : 1, corners);
                int cv = outer - inner; if (cv <= 0) continue;
                row[xx] = gx_mix(row[xx], c, a * cv / 255);
            }
        }
    }
    CE(wv);
}

typedef struct { int fast[512]; int maxcode[18]; int mincode[17]; int valptr[17]; uint8_t val[256]; int ok; } JpHuff;
typedef struct { int id, h, v, tq, td, ta, pred, pw, ph; uint8_t* pl; } JpComp;
typedef struct {
    const uint8_t* d; int n, pos;
    int w, h, nc, prec, prog, ri, hmax, vmax, sos;
    uint16_t q[4][64]; int qok[4];
    JpHuff hdc[4], hac[4];
    JpComp c[3];
    uint32_t bb; int bc, hit;
} JpDec;

static const uint8_t jp_zz[64] = { 0,1,8,16,9,2,3,10,17,24,32,25,18,11,4,5,12,19,26,33,40,48,41,34,27,20,13,6,7,14,21,28,35,42,49,56,57,50,43,36,29,22,15,23,30,37,44,51,58,59,52,45,38,31,39,46,53,60,61,54,47,55,62,63 };

static int jp_be16(const uint8_t* p) { return (p[0] << 8) | p[1]; }

static void jp_build(JpHuff* h, const uint8_t* bits, const uint8_t* val, int nv) {
    int code = 0, k = 0, l, i;
    for (i = 0; i < 512; i++) h->fast[i] = 0;
    for (i = 0; i < nv && i < 256; i++) h->val[i] = val[i];
    for (l = 1; l <= 16; l++) {
        h->mincode[l] = code; h->valptr[l] = k;
        for (i = 0; i < bits[l - 1]; i++) {
            if (l <= 9 && k + i < 256) {
                int c = code + i, sh = 9 - l, f = c << sh, cnt = 1 << sh, j;
                for (j = 0; j < cnt && f + j < 512; j++) h->fast[f + j] = (l << 8) | val[k + i];
            }
        }
        code += bits[l - 1]; k += bits[l - 1];
        h->maxcode[l] = bits[l - 1] ? code - 1 : -1;
        code <<= 1;
    }
    h->ok = 1;
}

static int jp_parse(JpDec* d) {
    const uint8_t* p = d->d; int n = d->n, i = 2, k;
    if (n < 4 || p[0] != 0xFF || p[1] != 0xD8) return -1;
    d->w = d->h = d->nc = d->prog = d->ri = 0; d->sos = -1;
    for (k = 0; k < 4; k++) { d->qok[k] = 0; d->hdc[k].ok = 0; d->hac[k].ok = 0; }
    while (i + 4 <= n) {
        int m, len;
        if (p[i] != 0xFF) { i++; continue; }
        m = p[i + 1];
        if (m == 0xFF) { i++; continue; }
        if (m == 0xD8 || m == 0x01 || (m >= 0xD0 && m <= 0xD7)) { i += 2; continue; }
        if (m == 0xD9) break;
        len = jp_be16(p + i + 2);
        if (len < 2 || i + 2 + len > n) return -3;
        const uint8_t* s = p + i + 4; int sl = len - 2;
        if (m == 0xDB) {
            while (sl > 0) {
                int pq = s[0] >> 4, tq = s[0] & 15, j;
                if (tq > 3) return -3;
                s++; sl--;
                for (j = 0; j < 64; j++) {
                    if (pq) { if (sl < 2) return -3; d->q[tq][j] = jp_be16(s); s += 2; sl -= 2; }
                    else { if (sl < 1) return -3; d->q[tq][j] = s[0]; s++; sl--; }
                }
                d->qok[tq] = 1;
            }
        } else if (m == 0xC4) {
            while (sl >= 17) {
                int tc = s[0] >> 4, th = s[0] & 15, tot = 0, j;
                if (th > 3 || tc > 1) return -3;
                for (j = 0; j < 16; j++) tot += s[1 + j];
                if (tot > 256 || sl < 17 + tot) return -3;
                jp_build(tc ? &d->hac[th] : &d->hdc[th], s + 1, s + 17, tot);
                s += 17 + tot; sl -= 17 + tot;
            }
        } else if (m == 0xC0 || m == 0xC1 || m == 0xC2) {
            int j;
            d->prog = (m == 0xC2);
            d->prec = s[0]; d->h = jp_be16(s + 1); d->w = jp_be16(s + 3); d->nc = s[5];
            if (d->prec != 8 || d->w < 1 || d->h < 1 || (d->nc != 1 && d->nc != 3) || sl < 6 + 3 * d->nc) return -2;
            for (j = 0; j < d->nc; j++) {
                d->c[j].id = s[6 + 3 * j]; d->c[j].h = s[7 + 3 * j] >> 4; d->c[j].v = s[7 + 3 * j] & 15; d->c[j].tq = s[8 + 3 * j];
                if (d->c[j].h < 1 || d->c[j].h > 4 || d->c[j].v < 1 || d->c[j].v > 4 || d->c[j].tq > 3) return -2;
            }
        } else if (m == 0xC3 || (m >= 0xC5 && m <= 0xCF && m != 0xC8 && m != 0xCC)) {
            return -2;
        } else if (m == 0xDD) {
            d->ri = jp_be16(s);
        } else if (m == 0xDA) {
            d->sos = i; break;
        }
        i += 2 + len;
    }
    if (d->w < 1 || d->nc < 1) return -3;
    return 0;
}

int dgn_gfx_jpeg_info(const uint8_t* src, int n) {
    JpDec* d = (JpDec*)malloc(sizeof(JpDec)); int r;
    if (!d) return -4;
    d->d = src; d->n = n;
    r = jp_parse(d);
    if (r == 0 && d->prog) r = -2;
    if (r == 0 && (d->w > 16383 || d->h > 16383)) r = -2;
    if (r == 0) r = (d->w << 16) | d->h;
    free(d);
    return r;
}

static void jp_fill(JpDec* d) {
    while (d->bc <= 24) {
        int b = 0;
        if (!d->hit && d->pos < d->n) {
            b = d->d[d->pos];
            if (b == 0xFF) {
                int b2 = d->pos + 1 < d->n ? d->d[d->pos + 1] : 0xD9;
                if (b2 == 0) d->pos += 2; else { d->hit = 1; b = 0; }
            } else d->pos++;
        } else d->hit = 1;
        d->bb = (d->bb << 8) | (uint32_t)b; d->bc += 8;
    }
}

static int jp_sym(JpDec* d, const JpHuff* h) {
    int l, c;
    jp_fill(d);
    c = h->fast[(d->bb >> (d->bc - 9)) & 511];
    if (c) { d->bc -= c >> 8; return c & 255; }
    for (l = 10; l <= 16; l++) {
        int code = (d->bb >> (d->bc - l)) & ((1 << l) - 1);
        if (h->maxcode[l] >= 0 && code <= h->maxcode[l] && code >= h->mincode[l]) {
            d->bc -= l;
            return h->val[h->valptr[l] + code - h->mincode[l]];
        }
    }
    d->bc -= 16;
    return 0;
}

static int jp_recv(JpDec* d, int n) {
    int v;
    if (n == 0) return 0;
    jp_fill(d);
    v = (d->bb >> (d->bc - n)) & ((1 << n) - 1);
    d->bc -= n;
    return v;
}

static int jp_ext(JpDec* d, int t) {
    int v = jp_recv(d, t);
    if (v < (1 << (t - 1))) v += (-1 << t) + 1;
    return v;
}

#define JP_F2F(x) ((int)((x) * 4096 + 0.5))
#define JP_FSH(x) ((x) * 4096)
#define JP_IDCT1D(s0,s1,s2,s3,s4,s5,s6,s7) \
    int t0, t1, t2, t3, p1, p2, p3, p4, p5, x0, x1, x2, x3; \
    p2 = s2; p3 = s6; p1 = (p2 + p3) * JP_F2F(0.5411961f); \
    t2 = p1 + p3 * JP_F2F(-1.847759065f); t3 = p1 + p2 * JP_F2F(0.765366865f); \
    p2 = s0; p3 = s4; t0 = JP_FSH(p2 + p3); t1 = JP_FSH(p2 - p3); \
    x0 = t0 + t3; x3 = t0 - t3; x1 = t1 + t2; x2 = t1 - t2; \
    t0 = s7; t1 = s5; t2 = s3; t3 = s1; \
    p3 = t0 + t2; p4 = t1 + t3; p1 = t0 + t3; p2 = t1 + t2; \
    p5 = (p3 + p4) * JP_F2F(1.175875602f); \
    t0 = t0 * JP_F2F(0.298631336f); t1 = t1 * JP_F2F(2.053119869f); t2 = t2 * JP_F2F(3.072711026f); t3 = t3 * JP_F2F(1.501321110f); \
    p1 = p5 + p1 * JP_F2F(-0.899976223f); p2 = p5 + p2 * JP_F2F(-2.562915447f); \
    p3 = p3 * JP_F2F(-1.961570560f); p4 = p4 * JP_F2F(-0.390180644f); \
    t3 += p1 + p4; t2 += p2 + p3; t1 += p2 + p4; t0 += p1 + p3;

static uint8_t jp_clamp(int v) { return v < 0 ? 0 : v > 255 ? 255 : (uint8_t)v; }

static void jp_idct(const int* in, uint8_t* out, int stride) {
    int i, v[64];
    for (i = 0; i < 8; i++) {
        JP_IDCT1D(in[i], in[i + 8], in[i + 16], in[i + 24], in[i + 32], in[i + 40], in[i + 48], in[i + 56])
        x0 += 512; x1 += 512; x2 += 512; x3 += 512;
        v[i] = (x0 + t3) >> 10; v[i + 56] = (x0 - t3) >> 10; v[i + 8] = (x1 + t2) >> 10; v[i + 48] = (x1 - t2) >> 10;
        v[i + 16] = (x2 + t1) >> 10; v[i + 40] = (x2 - t1) >> 10; v[i + 24] = (x3 + t0) >> 10; v[i + 32] = (x3 - t0) >> 10;
    }
    for (i = 0; i < 8; i++) {
        uint8_t* o = out + i * stride;
        JP_IDCT1D(v[i * 8], v[i * 8 + 1], v[i * 8 + 2], v[i * 8 + 3], v[i * 8 + 4], v[i * 8 + 5], v[i * 8 + 6], v[i * 8 + 7])
        x0 += 65536 + (128 << 17); x1 += 65536 + (128 << 17); x2 += 65536 + (128 << 17); x3 += 65536 + (128 << 17);
        o[0] = jp_clamp((x0 + t3) >> 17); o[7] = jp_clamp((x0 - t3) >> 17); o[1] = jp_clamp((x1 + t2) >> 17); o[6] = jp_clamp((x1 - t2) >> 17);
        o[2] = jp_clamp((x2 + t1) >> 17); o[5] = jp_clamp((x2 - t1) >> 17); o[3] = jp_clamp((x3 + t0) >> 17); o[4] = jp_clamp((x3 - t0) >> 17);
    }
}

static void jp_restart(JpDec* d) {
    int i;
    d->bb = 0; d->bc = 0; d->hit = 0;
    while (d->pos + 1 < d->n) {
        if (d->d[d->pos] == 0xFF && d->d[d->pos + 1] >= 0xD0 && d->d[d->pos + 1] <= 0xD7) { d->pos += 2; break; }
        d->pos++;
    }
    for (i = 0; i < d->nc; i++) d->c[i].pred = 0;
}

int dgn_gfx_jpeg(const uint8_t* src, int n, uint32_t* dst) {
    JpDec* d = (JpDec*)malloc(sizeof(JpDec));
    int r, i, mcux, mcuy, mx, my, mcu = 0, ret = 0, x, y;
    if (!d) return -4;
    d->d = src; d->n = n;
    r = jp_parse(d);
    if (r == 0 && d->prog) r = -2;
    if (r != 0 || d->sos < 0) { free(d); return r ? r : -3; }
    {
        const uint8_t* s = src + d->sos + 4; int ns = s[0];
        if (ns != d->nc) { free(d); return -2; }
        for (i = 0; i < ns; i++) {
            int cid = s[1 + 2 * i], j;
            for (j = 0; j < d->nc; j++) if (d->c[j].id == cid) { d->c[j].td = s[2 + 2 * i] >> 4; d->c[j].ta = s[2 + 2 * i] & 15; }
        }
        d->pos = d->sos + 2 + jp_be16(src + d->sos + 2);
    }
    d->hmax = d->vmax = 1;
    for (i = 0; i < d->nc; i++) { if (d->c[i].h > d->hmax) d->hmax = d->c[i].h; if (d->c[i].v > d->vmax) d->vmax = d->c[i].v; }
    mcux = (d->w + 8 * d->hmax - 1) / (8 * d->hmax); mcuy = (d->h + 8 * d->vmax - 1) / (8 * d->vmax);
    for (i = 0; i < d->nc; i++) {
        JpComp* c = &d->c[i];
        if (c->td > 3 || c->ta > 3 || !d->hdc[c->td].ok || !d->hac[c->ta].ok || !d->qok[c->tq]) { ret = -3; break; }
        c->pw = mcux * c->h * 8; c->ph = mcuy * c->v * 8; c->pred = 0;
        c->pl = (uint8_t*)malloc((size_t)c->pw * c->ph);
        if (!c->pl) { ret = -4; break; }
    }
    d->bb = 0; d->bc = 0; d->hit = 0;
    for (my = 0; my < mcuy && !ret; my++) {
        for (mx = 0; mx < mcux; mx++) {
            if (d->ri && mcu && mcu % d->ri == 0) jp_restart(d);
            for (i = 0; i < d->nc; i++) {
                JpComp* c = &d->c[i]; int bv, bh;
                for (bv = 0; bv < c->v; bv++) for (bh = 0; bh < c->h; bh++) {
                    int coef[64], k = 1, t;
                    const uint16_t* q = d->q[c->tq];
                    memset(coef, 0, sizeof(coef));
                    t = jp_sym(d, &d->hdc[c->td]);
                    c->pred += t ? jp_ext(d, t) : 0;
                    coef[0] = c->pred * q[0];
                    while (k < 64) {
                        int rs = jp_sym(d, &d->hac[c->ta]), rr = rs >> 4, ss = rs & 15;
                        if (ss == 0) { if (rr == 15) { k += 16; continue; } break; }
                        k += rr;
                        if (k > 63) break;
                        coef[jp_zz[k]] = jp_ext(d, ss) * q[k];
                        k++;
                    }
                    jp_idct(coef, c->pl + (size_t)((my * c->v + bv) * 8) * c->pw + (mx * c->h + bh) * 8, c->pw);
                }
            }
            mcu++;
        }
    }
    if (!ret) {
        int* xa[3] = { 0, 0, 0 }; int* xb[3] = { 0, 0, 0 }; int* xf[3] = { 0, 0, 0 }; int cwv[3], chv[3];
        for (i = 0; i < d->nc && !ret; i++) {
            JpComp* c = &d->c[i];
            cwv[i] = (d->w * c->h + d->hmax - 1) / d->hmax; chv[i] = (d->h * c->v + d->vmax - 1) / d->vmax;
            xa[i] = (int*)malloc(sizeof(int) * d->w * 3);
            if (!xa[i]) { ret = -4; break; }
            xb[i] = xa[i] + d->w; xf[i] = xb[i] + d->w;
            for (x = 0; x < d->w; x++) {
                int pos, i0, fr;
                if (c->h == d->hmax) { xa[i][x] = xb[i][x] = x; xf[i][x] = 0; continue; }
                pos = ((2 * x + 1) * c->h * 128) / d->hmax - 128;
                i0 = pos >> 8; fr = pos & 255;
                xa[i][x] = i0 < 0 ? 0 : i0 >= cwv[i] ? cwv[i] - 1 : i0;
                xb[i][x] = i0 + 1 < 0 ? 0 : i0 + 1 >= cwv[i] ? cwv[i] - 1 : i0 + 1;
                xf[i][x] = fr;
            }
        }
        if (!ret) {
            for (y = 0; y < d->h; y++) {
                uint32_t* o = dst + (size_t)y * d->w;
                const uint8_t* r0[3]; const uint8_t* r1[3]; int fy[3], k;
                for (k = 0; k < d->nc; k++) {
                    JpComp* c = &d->c[k]; int i0 = y, i1 = y, fr = 0;
                    if (c->v != d->vmax) {
                        int pos = ((2 * y + 1) * c->v * 128) / d->vmax - 128;
                        i0 = pos >> 8; fr = pos & 255; i1 = i0 + 1;
                        if (i0 < 0) i0 = 0;
                        if (i0 >= chv[k]) i0 = chv[k] - 1;
                        if (i1 < 0) i1 = 0;
                        if (i1 >= chv[k]) i1 = chv[k] - 1;
                    }
                    r0[k] = c->pl + (size_t)i0 * c->pw; r1[k] = c->pl + (size_t)i1 * c->pw; fy[k] = fr;
                }
                if (d->nc == 1) {
                    for (x = 0; x < d->w; x++) { uint32_t v = r0[0][x]; o[x] = (v << 16) | (v << 8) | v; }
                } else {
                    for (x = 0; x < d->w; x++) {
                        int v3[3];
                        for (k = 0; k < 3; k++) {
                            int a = xa[k][x], b2 = xb[k][x], fx = xf[k][x];
                            if (fx == 0 && fy[k] == 0) v3[k] = r0[k][a];
                            else {
                                int top = r0[k][a] * (256 - fx) + r0[k][b2] * fx, bot = r1[k][a] * (256 - fx) + r1[k][b2] * fx;
                                v3[k] = (top * (256 - fy[k]) + bot * fy[k]) >> 16;
                            }
                        }
                        {
                            int Y = v3[0] << 16, cb = v3[1] - 128, cr = v3[2] - 128;
                            int R = (Y + 91881 * cr + 32768) >> 16, G = (Y - 22554 * cb - 46802 * cr + 32768) >> 16, B = (Y + 116130 * cb + 32768) >> 16;
                            o[x] = ((uint32_t)jp_clamp(R) << 16) | ((uint32_t)jp_clamp(G) << 8) | jp_clamp(B);
                        }
                    }
                }
            }
        }
        for (i = 0; i < d->nc; i++) free(xa[i]);
    }
    for (i = 0; i < d->nc; i++) free(d->c[i].pl);
    free(d);
    return ret;
}
