#include <stdint.h>
#include <string.h>

int dgn_bm_input_poll(int* kind, int* type, int* code, int* value);

#define INJ_N 128
static int inj[INJ_N][4];
static int inj_r, inj_w;
void dgn_bm_input_inject(int kind, int type, int code, int value) {
    int n = (inj_w + 1) % INJ_N;
    if (n == inj_r) return;
    inj[inj_w][0] = kind; inj[inj_w][1] = type; inj[inj_w][2] = code; inj[inj_w][3] = value; inj_w = n;
}
static int raw_poll(int* kind, int* type, int* code, int* value) {
    if (inj_r != inj_w) { *kind = inj[inj_r][0]; *type = inj[inj_r][1]; *code = inj[inj_r][2]; *value = inj[inj_r][3]; inj_r = (inj_r + 1) % INJ_N; return 1; }
    return dgn_bm_input_poll(kind, type, code, value);
}

extern int dgn_bm_fb_w, dgn_bm_fb_h;
static const char* base_row = "\0\0" "1234567890-=\0\tqwertyuiop[]\n\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
static const char* shift_row = "\0\0" "!@#$%^&*()_+\0\tQWERTYUIOP{}\n\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";
static int mods, caps, px, py, pbtn, have_ptr;

static const char* special(int code) {
    switch (code) {
    case 1: return "Escape"; case 14: return "Backspace"; case 15: return "Tab"; case 28: return "Enter"; case 57: return " ";
    case 103: return "Up"; case 108: return "Down"; case 105: return "Left"; case 106: return "Right";
    case 102: return "Home"; case 107: return "End"; case 104: return "PageUp"; case 109: return "PageDown";
    case 110: return "Insert"; case 111: return "Delete";
    case 59: return "F1"; case 60: return "F2"; case 61: return "F3"; case 62: return "F4"; case 63: return "F5";
    case 64: return "F6"; case 65: return "F7"; case 66: return "F8"; case 67: return "F9"; case 68: return "F10";
    }
    return 0;
}

int dgn_bm_input_event(char* name, int* a, int* b, int* c) {
    int kind, type, code, value;
    while (raw_poll(&kind, &type, &code, &value)) {
        if (kind == 1 && type == 1) {
            if (code == 42 || code == 54) { mods = value ? (mods | 1) : (mods & ~1); continue; }
            if (code == 29 || code == 97) { mods = value ? (mods | 2) : (mods & ~2); continue; }
            if (code == 56 || code == 100) { mods = value ? (mods | 4) : (mods & ~4); continue; }
            if (code == 58) { if (value == 1) caps ^= 1; continue; }
            const char* sp = special(code);
            char tmp[2] = { 0, 0 };
            if (!sp) {
                if (code < 2 || code > 57) continue;
                char ch = (mods & 1) ? shift_row[code] : base_row[code];
                if (ch == 0) continue;
                if (caps && ch >= 'a' && ch <= 'z') ch = (char)(ch - 32); else if (caps && (mods & 1) && ch >= 'A' && ch <= 'Z') ch = (char)(ch + 32);
                tmp[0] = ch; sp = tmp;
            }
            strcpy(name, sp); *a = value ? 1 : 0; *b = mods; *c = 0; return 1;
        }
        if (kind == 2 && type == 2) {
            if (code == 0) px += value; else if (code == 1) py += value;
            else if (code == 8) { *a = value; *b = *c = 0; strcpy(name, "wheel"); return 3; }
            if (px < 0) px = 0; if (py < 0) py = 0; if (px >= dgn_bm_fb_w) px = dgn_bm_fb_w - 1; if (py >= dgn_bm_fb_h) py = dgn_bm_fb_h - 1;
            have_ptr = 1; continue;
        }
        if (kind == 3 && type == 3) {
            if (code == 0) px = (int)((long)value * dgn_bm_fb_w / 32768); else if (code == 1) py = (int)((long)value * dgn_bm_fb_h / 32768);
            have_ptr = 1; continue;
        }
        if ((kind == 2 || kind == 3) && type == 1) {
            int bit = code == 0x110 ? 1 : code == 0x111 ? 2 : code == 0x112 ? 4 : 0;
            if (bit) { pbtn = value ? (pbtn | bit) : (pbtn & ~bit); have_ptr = 1; }
            continue;
        }
        if ((kind == 2 || kind == 3) && type == 0 && have_ptr) {
            have_ptr = 0; { extern void dgn_bm_cur_move(int,int); dgn_bm_cur_move(px, py); } *a = px; *b = py; *c = pbtn; strcpy(name, "mouse"); return 2;
        }
        if (kind == 3 && type == 3 && code == 8) { *a = value; strcpy(name, "wheel"); return 3; }
    }
    return 0;
}
