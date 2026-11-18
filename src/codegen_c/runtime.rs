#![allow(dead_code)]

pub(crate) const RUNTIME_HEADER: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdint.h>
#include <setjmp.h>
#include <ctype.h>
#include <time.h>
#include <errno.h>
#include <sys/stat.h>
#ifndef _WIN32
#  include <dirent.h>
#endif
#ifdef _WIN32
#  include <winsock2.h>
#  include <ws2tcpip.h>
#  pragma comment(lib, "ws2_32.lib")
#else
#  include <sys/socket.h>
#  include <netinet/in.h>
#  include <netdb.h>
#  include <arpa/inet.h>
#  include <pthread.h>
#  include <dirent.h>
#endif

#ifdef _WIN32
#  include <windows.h>
#  define DGN_SLEEP(ms) Sleep(ms)
#else
#  include <unistd.h>
#  define DGN_SLEEP(ms) usleep((unsigned int)((ms)*1000))
#endif

typedef enum {
    DGN_NULL = 0, DGN_BOOL, DGN_INT, DGN_FLOAT, DGN_STR,
    DGN_LIST, DGN_MAP, DGN_FUNC, DGN_CLASS, DGN_INSTANCE,
} DgnTag;

typedef struct DgnVal   DgnVal;
typedef struct DgnList  DgnList;
typedef struct DgnMap   DgnMap;
typedef struct DgnFunc  DgnFunc;
typedef struct DgnEnv   DgnEnv;
typedef DgnVal (*DgnFuncPtr)(DgnVal*, int, DgnEnv*);

static DgnVal dgn_func(DgnFuncPtr fn, int arity);
static DgnVal dgn_closure(DgnFuncPtr fn, int arity, DgnMap* env);
static DgnVal dgn_call_val(DgnVal fv, DgnVal* args, int n);
static DgnVal dgn_map_get_raw(DgnMap* m, const char* key);
static void   dgn_map_set_raw(DgnMap* m, const char* key, DgnVal val);

struct DgnVal {
    DgnTag tag;
    union { int64_t i; double f; char* s; DgnList* list; DgnMap* map; DgnFunc* func; };
};

struct DgnList { DgnVal* data; int len; int cap; };
typedef struct { char* key; DgnVal val; int used; } DgnMapEntry;
/* Maps keep their entries in insertion order (removing leaves a hole that compaction closes later); lookups use a
 * hash index once the map has more than 8 slots, and a plain scan below that.  `n` = slots used so far (live + holes). */
struct DgnMap  { DgnMapEntry* entries; int cap; int len; int n; int icap; int* idx; };
struct DgnFunc { DgnFuncPtr fn; int arity; DgnMap* env; const char* pnames; int skip; };
typedef struct { jmp_buf buf; DgnVal thrown; } DgnTryFrame;
struct DgnEnv { DgnMap* map; };

typedef enum { DGN_PROMISE_PENDING, DGN_PROMISE_RESOLVED } DgnPromiseStatus;

typedef struct DgnPromise DgnPromise;
struct DgnPromise {
    DgnPromiseStatus status;
    DgnVal           result;
    int              done;
#ifdef _WIN32
    HANDLE           thread;
    CRITICAL_SECTION lock;
#else
    pthread_t        thread;
    pthread_mutex_t  lock;
#endif
};

#ifdef _WIN32
typedef SOCKET DgnSocket;
#else
typedef int    DgnSocket;
#endif

#undef  DGN_INSTANCE

typedef struct GcHeader GcHeader;
struct GcHeader { int marked; DgnTag tag; GcHeader* next; GcHeader* hnext; };

static GcHeader* _dgn_gc_head = NULL;
static void* _dgn_stack_bottom = NULL;
#if defined(DGN_BAREMETAL) && !defined(DGN_USER)
#define DGN_PERCPU 1
#include "dgn_pcpu.h"
#endif
#ifdef DGN_BAREMETAL
extern void  dgn_bm_crit_enter(void);
extern void  dgn_bm_crit_exit(void);
extern void* dgn_bm_stack_bottom(void* main_bottom);
extern void  dgn_bm_scan_stacks(void (*mark)(void*));
extern void  dgn_bm_safepoint(void);
#ifdef DGN_PERCPU
extern int   dgn_bm_stw_begin(void);
extern void  dgn_bm_stw_end(void);
#define DGN_SAFEPOINT() do { if (PC(resched)) dgn_bm_safepoint(); } while (0)
#else
extern volatile int dgn_bm_resched;
#define DGN_SAFEPOINT() do { if (dgn_bm_resched) dgn_bm_safepoint(); } while (0)
#endif
#else
#define DGN_SAFEPOINT() ((void)0)
#endif

#define DGN_GC_HTAB_BITS 16
#define DGN_GC_HTAB_SIZE (1u << DGN_GC_HTAB_BITS)
static GcHeader* _dgn_gc_htab[DGN_GC_HTAB_SIZE];

static unsigned _dgn_gc_hash(void* p) {
    uintptr_t x = (uintptr_t)p >> 4;
    x ^= x >> 15; x *= 0x2545F4914F6CDD1DULL; x ^= x >> 13;
    return (unsigned)(x & (DGN_GC_HTAB_SIZE - 1));
}

static void dgn_gc_htab_insert(GcHeader* hdr) {
    void* payload = (void*)(hdr + 1);
    unsigned b = _dgn_gc_hash(payload);
    hdr->hnext = _dgn_gc_htab[b];
    _dgn_gc_htab[b] = hdr;
}

static void dgn_gc_htab_remove(GcHeader* hdr) {
    void* payload = (void*)(hdr + 1);
    unsigned b = _dgn_gc_hash(payload);
    GcHeader** slot = &_dgn_gc_htab[b];
    while(*slot) {
        if(*slot == hdr) { *slot = hdr->hnext; return; }
        slot = &(*slot)->hnext;
    }
}

static GcHeader* dgn_gc_htab_find(void* payload) {
    unsigned b = _dgn_gc_hash(payload);
    GcHeader* c = _dgn_gc_htab[b];
    while(c) {
        if((void*)(c + 1) == payload) return c;
        c = c->hnext;
    }
    return NULL;
}

static void dgn_gc_mark_ptr(void* ptr) {
    if(!ptr) return;
    GcHeader* curr = dgn_gc_htab_find(ptr);
    if(!curr || curr->marked) return;
    curr->marked = 1;
    void* payload = ptr;
    if(curr->tag == DGN_LIST) {
        DgnList* l = (DgnList*)payload;
        for(int i=0; i<l->len; i++) {
            DgnVal v = l->data[i];
            if(v.tag==DGN_STR) dgn_gc_mark_ptr(v.s);
            else if(v.tag==DGN_LIST) dgn_gc_mark_ptr(v.list);
            else if(v.tag==DGN_MAP||v.tag==DGN_CLASS||v.tag==DGN_INSTANCE) dgn_gc_mark_ptr(v.map);
            else if(v.tag==DGN_FUNC) dgn_gc_mark_ptr(v.func);
        }
    } else if(curr->tag == DGN_MAP || curr->tag == DGN_CLASS || curr->tag == DGN_INSTANCE) {
        DgnMap* m = (DgnMap*)payload;
        for(int i=0; i<m->cap; i++) {
            if(m->entries[i].used) {
                DgnVal v = m->entries[i].val;
                if(v.tag==DGN_STR) dgn_gc_mark_ptr(v.s);
                else if(v.tag==DGN_LIST) dgn_gc_mark_ptr(v.list);
                else if(v.tag==DGN_MAP||v.tag==DGN_CLASS||v.tag==DGN_INSTANCE) dgn_gc_mark_ptr(v.map);
                else if(v.tag==DGN_FUNC) dgn_gc_mark_ptr(v.func);
            }
        }
    } else if(curr->tag == DGN_FUNC) {
        DgnFunc* f = (DgnFunc*)payload;
        if(f->env) dgn_gc_mark_ptr(f->env);
    }
}

extern DgnVal _dgn_aug_str;
extern DgnVal _dgn_aug_list;
extern int _dgn_aug_init;
extern DgnVal _dgn_ffi_sigs;

extern DgnVal* _dgn_global_roots[];

static void dgn_gc_mark_val(DgnVal v) {
    if(v.tag==DGN_MAP||v.tag==DGN_CLASS||v.tag==DGN_INSTANCE) dgn_gc_mark_ptr(v.map);
    else if(v.tag==DGN_LIST) dgn_gc_mark_ptr(v.list);
    else if(v.tag==DGN_STR)  dgn_gc_mark_ptr(v.s);
    else if(v.tag==DGN_FUNC) dgn_gc_mark_ptr(v.func);
}

#ifdef _MSC_VER
static __declspec(noinline) void dgn_gc_collect(void);
#else
static void dgn_gc_collect(void) __attribute__((noinline));
#endif
static long _dgn_gc_threshold = 0;
#ifdef DGN_PERCPU
#define _dgn_sl_p PC(sl_p)
#define _dgn_sl_n PC(sl_n)
#else
static const char* _dgn_sl_p = NULL;   /* one-entry cache for the length of a long string (see dgn_strlen_fast) */
static size_t _dgn_sl_n = 0;
#endif
static void dgn_gc_collect(void) {
#ifdef DGN_PERCPU
    if(!dgn_bm_stw_begin()) return;
    for(int _ci = 0; _ci < DGN_MAXCPU; _ci++) dgn_pcpu[_ci].sl_p = NULL;
#elif defined(DGN_BAREMETAL)
    dgn_bm_crit_enter();
#endif
    long _survivors = 0;
    _dgn_sl_p = NULL;
    if(_dgn_aug_init) {
        dgn_gc_mark_ptr(_dgn_aug_str.map);
        dgn_gc_mark_ptr(_dgn_aug_list.map);
    }
    if(_dgn_ffi_sigs.tag == DGN_MAP) dgn_gc_mark_ptr(_dgn_ffi_sigs.map);
    for(int _gi = 0; _dgn_global_roots[_gi] != NULL; _gi++)
        dgn_gc_mark_val(*_dgn_global_roots[_gi]);

    jmp_buf _dgn_gc_regspill;
    setjmp(_dgn_gc_regspill);

    void* top;
    void** curr = (void**)&top;
#ifdef DGN_BAREMETAL
    void** bottom = (void**)dgn_bm_stack_bottom(_dgn_stack_bottom);
#else
    void** bottom = (void**)_dgn_stack_bottom;
#endif
    if(curr > bottom) { void** tmp = curr; curr = bottom; bottom = tmp; }
    while(curr < bottom) { dgn_gc_mark_ptr(*curr); curr++; }
    { void** jb = (void**)&_dgn_gc_regspill; size_t jn = sizeof(_dgn_gc_regspill)/sizeof(void*);
      for(size_t _ji=0; _ji<jn; _ji++) dgn_gc_mark_ptr(jb[_ji]); }
#ifdef DGN_BAREMETAL
    dgn_bm_scan_stacks(dgn_gc_mark_ptr);
#endif

    GcHeader** node = &_dgn_gc_head;
    while(*node) {
        if(!(*node)->marked) {
            GcHeader* unreached = *node;
            *node = unreached->next;
            dgn_gc_htab_remove(unreached);
            if(unreached->tag == DGN_LIST) {
                DgnList* l = (DgnList*)(unreached + 1);
                if(l->data) free(l->data);
            } else if(unreached->tag == DGN_MAP || unreached->tag == DGN_CLASS || unreached->tag == DGN_INSTANCE) {
                DgnMap* m = (DgnMap*)(unreached + 1);
                if(m->entries) {
                    for(int i=0; i<m->cap; i++) if(m->entries[i].used && m->entries[i].key) free(m->entries[i].key);
                    free(m->entries);
                }
                free(m->idx);
            }
            free(unreached);
        } else {
            (*node)->marked = 0;
            _survivors++;
            node = &(*node)->next;
        }
    }
    _dgn_gc_threshold = _survivors * 2;
#ifdef DGN_PERCPU
    dgn_bm_stw_end();
#elif defined(DGN_BAREMETAL)
    dgn_bm_crit_exit();
#endif
}

static long _dgn_gc_alloc_count = 0;
#ifdef DGN_USER
#define DGN_GC_THRESHOLD 4000   /* user programs have a small fixed heap: collect often */
#else
#define DGN_GC_THRESHOLD 50000
#endif

/* malloc/realloc that collect garbage and retry when memory runs out (a fixed-size heap on bare metal) */
static void* dgn_heap_alloc(void* old, size_t size) {
    void* p = old ? realloc(old, size) : malloc(size);
#ifdef DGN_BAREMETAL
    if(!p) {
        dgn_gc_collect();
        _dgn_gc_alloc_count = 0;
        p = old ? realloc(old, size) : malloc(size);
        if(!p) { fputs("[dragon] out of memory\n", stderr); exit(137); }
    }
#endif
    return p;
}

static volatile int _dgn_gc_lock;
static void* dgn_gc_alloc(size_t size, DgnTag tag) {
#ifdef DGN_PERCPU
    if(__sync_add_and_fetch(&_dgn_gc_alloc_count, 1) >= (_dgn_gc_threshold > DGN_GC_THRESHOLD ? _dgn_gc_threshold : DGN_GC_THRESHOLD)) {
        dgn_gc_collect();
        _dgn_gc_alloc_count = 0;
    }
    GcHeader* hdr = (GcHeader*)dgn_heap_alloc(NULL, sizeof(GcHeader) + size);
    memset((void*)(hdr + 1), 0, size);
    hdr->marked = 0;
    hdr->tag = tag;
    dgn_spin_lock(&_dgn_gc_lock);
    hdr->next = _dgn_gc_head;
    _dgn_gc_head = hdr;
    dgn_gc_htab_insert(hdr);
    dgn_spin_unlock(&_dgn_gc_lock);
    return (void*)(hdr + 1);
#else
#ifdef DGN_BAREMETAL
    dgn_bm_crit_enter();
#endif
    _dgn_gc_alloc_count++;
    if(_dgn_gc_alloc_count >= (_dgn_gc_threshold > DGN_GC_THRESHOLD ? _dgn_gc_threshold : DGN_GC_THRESHOLD)) {
        dgn_gc_collect();
        _dgn_gc_alloc_count = 0;
    }
    GcHeader* hdr = (GcHeader*)dgn_heap_alloc(NULL, sizeof(GcHeader) + size);
    memset((void*)(hdr + 1), 0, size);
    hdr->marked = 0;
    hdr->tag = tag;
    hdr->next = _dgn_gc_head;
    _dgn_gc_head = hdr;
    dgn_gc_htab_insert(hdr);
#ifdef DGN_BAREMETAL
    dgn_bm_crit_exit();
#endif
    return (void*)(hdr + 1);
#endif
}

static DgnVal dgn_null(void)      { DgnVal v; v.tag=DGN_NULL;  v.i=0;    return v; }
static DgnVal dgn_bool(int b)     { DgnVal v; v.tag=DGN_BOOL;  v.i=b?1:0;return v; }
static DgnVal dgn_int(int64_t n)  { DgnVal v; v.tag=DGN_INT;   v.i=n;    return v; }
static DgnVal dgn_float(double d) { DgnVal v; v.tag=DGN_FLOAT; v.f=d;    return v; }

/* a string literal: points at the constant in the program, nothing is allocated (strings are never changed in place; the garbage
   collector ignores pointers it does not own) */
static inline DgnVal dgn_lit(const char* s) { DgnVal v; v.tag=DGN_STR; v.s=(char*)s; return v; }

static DgnVal dgn_str(const char* s) {
    DgnVal v; v.tag=DGN_STR;
    v.s=(char*)dgn_gc_alloc(strlen(s)+1, DGN_STR); strcpy(v.s,s); return v;
}

static DgnVal dgn_list_new(int hint) {
    DgnVal v; v.tag=DGN_LIST;
    v.list=(DgnList*)dgn_gc_alloc(sizeof(DgnList), DGN_LIST);
    int cap=hint>4?hint:4;
    v.list->data=(DgnVal*)dgn_heap_alloc(NULL, (size_t)cap*sizeof(DgnVal));
    v.list->cap=cap; v.list->len=0;
    return v;
}

static void dgn_list_push(DgnVal* lv, DgnVal item) {
    DgnList* l=lv->list;
    if(l->len>=l->cap){ l->cap*=2; l->data=(DgnVal*)dgn_heap_alloc(l->data, (size_t)l->cap*sizeof(DgnVal)); }
    l->data[l->len++]=item;
}

static DgnVal dgn_box_new(DgnVal v) { DgnVal b = dgn_list_new(1); dgn_list_push(&b, v); return b; }
static DgnVal dgn_box_get(DgnVal b) { return (b.tag == DGN_LIST && b.list && b.list->len > 0) ? b.list->data[0] : b; }
static void dgn_box_set(DgnVal b, DgnVal v) { if (b.tag == DGN_LIST && b.list && b.list->len > 0) b.list->data[0] = v; }

static DgnVal dgn_bytes(DgnVal* args, int n) {
    DgnVal list = dgn_list_new(n > 0 ? 16 : 0);
    if (n > 0 && args[0].tag == DGN_STR) {
        for (int i = 0; args[0].s[i]; i++) {
            dgn_list_push(&list, dgn_int((unsigned char)args[0].s[i]));
        }
    } else if (n > 0 && args[0].tag == DGN_INT) {
        for(int i = 0; i < args[0].i; i++) {
            dgn_list_push(&list, dgn_int(0));
        }
    }
    return list;
}

static DgnVal dgn_list_slice(DgnVal v, int start) {
    if (v.tag != DGN_LIST) return dgn_null();
    int len = v.list->len;
    if (start >= len) return dgn_list_new(0);
    DgnVal sub = dgn_list_new(len - start);
    for (int i = start; i < len; i++) dgn_list_push(&sub, v.list->data[i]);
    return sub;
}

static DgnVal dgn_map_new(void) {
    DgnVal v; v.tag=DGN_MAP;
    v.map=(DgnMap*)dgn_gc_alloc(sizeof(DgnMap), DGN_MAP);
    v.map->cap=8; v.map->len=0; v.map->n=0; v.map->icap=0; v.map->idx=NULL;
    v.map->entries=(DgnMapEntry*)calloc(8,sizeof(DgnMapEntry));
    return v;
}
static unsigned dgn_hash_str(const char* k) { unsigned h = 2166136261u; while (*k) { h ^= (unsigned char)*k++; h *= 16777619u; } return h; }
static void dgn_map_reindex(DgnMap* m) {
    free(m->idx); m->idx = NULL; m->icap = 0;
    if (m->cap <= 8) return;
    int ic = 16; while (ic < m->cap * 2) ic <<= 1;
    m->idx = (int*)calloc((size_t)ic, sizeof(int)); m->icap = ic;
    for (int i = 0; i < m->n; i++) if (m->entries[i].used) {
        unsigned h = dgn_hash_str(m->entries[i].key) & (unsigned)(ic - 1);
        while (m->idx[h]) h = (h + 1) & (unsigned)(ic - 1);
        m->idx[h] = i + 1;
    }
}
/* key comparison without a library call: keys are short and mostly differ early */
static inline int dgn_keyeq(const char* a, const char* b) { while(*a && *a == *b) { a++; b++; } return *a == *b; }
/* slot of `key`, or -1 */
static int dgn_map_find(DgnMap* m, const char* key) {
    if (!m->idx) {
        char k0 = key[0];
        for (int i = 0; i < m->n; i++) { const char* ek = m->entries[i].key; if (m->entries[i].used && ek[0] == k0 && dgn_keyeq(ek, key)) return i; }
        return -1;
    }
    unsigned mask = (unsigned)(m->icap - 1), h = dgn_hash_str(key) & mask;
    while (m->idx[h]) {
        int s = m->idx[h];
        if (s > 0 && m->entries[s - 1].used && m->entries[s - 1].key[0] == key[0] && dgn_keyeq(m->entries[s - 1].key, key)) return s - 1;
        h = (h + 1) & mask;
    }
    return -1;
}
static int dgn_map_remove_key(DgnMap* m, const char* key) {
    int i = dgn_map_find(m, key);
    if (i < 0) return 0;
    m->entries[i].used = 0; m->len--;
    if (m->idx) {                       /* leave a tombstone (-1) so probe chains stay intact */
        unsigned mask = (unsigned)(m->icap - 1), h = dgn_hash_str(m->entries[i].key) & mask;
        while (m->idx[h] != i + 1) h = (h + 1) & mask;
        m->idx[h] = -1;
    }
    return 1;
}
static void dgn_map_clear_all(DgnMap* m) {
    for (int i = 0; i < m->n; i++) m->entries[i].used = 0;
    m->len = 0; m->n = 0;
    if (m->idx) memset(m->idx, 0, (size_t)m->icap * sizeof(int));
}
static void dgn_map_set(DgnVal* mv, const char* key, DgnVal val) {
    DgnMap* m=mv->map;
    int f = dgn_map_find(m, key);
    if (f >= 0) { m->entries[f].val = val; return; }
    if (m->n >= m->cap) {
        if (m->len < m->n - m->n / 4 && m->n > 0) {               /* many holes: close them up instead of growing */
            int w = 0;
            for (int i = 0; i < m->n; i++) if (m->entries[i].used) { if (w != i) m->entries[w] = m->entries[i]; w++; }
            for (int i = w; i < m->n; i++) memset(&m->entries[i], 0, sizeof(DgnMapEntry));
            m->n = w;
        } else {
            int nc = m->cap * 2;
            m->entries = (DgnMapEntry*)realloc(m->entries, (size_t)nc * sizeof(DgnMapEntry));
            memset(m->entries + m->cap, 0, (size_t)(nc - m->cap) * sizeof(DgnMapEntry));
            m->cap = nc;
        }
        dgn_map_reindex(m);
    }
    int i = m->n++;
    m->entries[i].key = strdup(key); m->entries[i].val = val; m->entries[i].used = 1; m->len++;
    if (m->idx) {
        unsigned mask = (unsigned)(m->icap - 1), h = dgn_hash_str(key) & mask;
        while (m->idx[h] > 0) h = (h + 1) & mask;
        m->idx[h] = i + 1;
    }
}
static DgnVal dgn_map_get(DgnVal mv, const char* key) {
    DgnMap* m=mv.map;
    int i = dgn_map_find(m, key);
    return i >= 0 ? m->entries[i].val : dgn_null();
}

static void dgn_fmt_float(double d, char* buf, size_t n) {
    if(isnan(d)) { snprintf(buf,n,"NaN"); return; }
    if(isinf(d)) { snprintf(buf,n,d<0?"-inf":"inf"); return; }
    for(int prec=1; prec<=17; prec++) {
        snprintf(buf,n,"%.*g",prec,d);
        if(strtod(buf,NULL)==d) return;
    }
}

static char* _dgn_val_to_cstr(DgnVal v, int depth) {
    char buf[8192]; buf[0]=0;
    if(depth > 10) return strdup("...");
    switch(v.tag){
        case DGN_NULL:  return strdup("null");
        case DGN_BOOL:  return strdup(v.i?"True":"False");
        case DGN_INT:   snprintf(buf,sizeof(buf),"%lld",(long long)v.i); return strdup(buf);
        case DGN_FLOAT: dgn_fmt_float(v.f,buf,sizeof(buf)); return strdup(buf);
        case DGN_STR:   if(depth==0) return strdup(v.s); snprintf(buf,sizeof(buf),"\"%s\"", v.s); return strdup(buf);
        case DGN_LIST: {
            strcpy(buf, "[");
            for(int i=0; i<v.list->len; i++) {
                if(i>0) strcat(buf, ", ");
                char* s = _dgn_val_to_cstr(v.list->data[i], depth+1);
                if(strlen(buf)+strlen(s) < sizeof(buf)-4) strcat(buf, s);
                free(s);
            }
            strcat(buf, "]"); return strdup(buf);
        }
        case DGN_MAP:
        case DGN_INSTANCE: {
            strcpy(buf, "{"); int first = 1;
            for(int i=0; i<v.map->cap; i++) {
                if(v.map->entries[i].used) {
                    if(!first) strcat(buf, ", "); else strcat(buf, " ");
                    strcat(buf, v.map->entries[i].key); strcat(buf, ": ");
                    char* s = _dgn_val_to_cstr(v.map->entries[i].val, depth+1);
                    if(strlen(buf)+strlen(s) < sizeof(buf)-4) strcat(buf, s);
                    free(s); first = 0;
                }
            }
            if(!first) strcat(buf, " }"); else strcat(buf, "}");
            return strdup(buf);
        }
        case DGN_FUNC:  return strdup("<func>");
        case DGN_CLASS: return strdup("<class>");
        default:        return strdup("?");
    }
}
static char* dgn_val_to_cstr(DgnVal v) { return _dgn_val_to_cstr(v, 0); }

static DgnVal dgn_showln(DgnVal* args, int n) {
    for(int i=0;i<n;i++){ if(i) printf(" "); char* s=dgn_val_to_cstr(args[i]); printf("%s",s); free(s); }
    printf("\n"); fflush(stdout); return dgn_null();
}
static DgnVal dgn_show(DgnVal* args, int n) {
    for(int i=0;i<n;i++){ if(i) printf(" "); char* s=dgn_val_to_cstr(args[i]); printf("%s",s); free(s); }
    fflush(stdout); return dgn_null();
}
static DgnVal dgn_readln(DgnVal prompt) {
    if(prompt.tag!=DGN_NULL){ char* s=dgn_val_to_cstr(prompt); printf("%s",s); free(s); fflush(stdout); }
    size_t cap=256, len=0; int c;
    char* buf=(char*)malloc(cap);
    while((c=getchar())!=EOF && c!='\n'){
        if(len+1>=cap){ cap*=2; buf=(char*)realloc(buf,cap); }
        buf[len++]=(char)c;
    }
    buf[len]='\0';
    DgnVal v=dgn_str(buf); free(buf); return v;
}

static int dgn_truthy(DgnVal v) {
    DGN_SAFEPOINT();
    switch(v.tag){
        case DGN_NULL:  return 0;
        case DGN_BOOL:  return (int)v.i;
        case DGN_INT:   return v.i!=0;
        case DGN_FLOAT: return v.f!=0.0;
        case DGN_STR:   return v.s&&v.s[0]!='\0';
        case DGN_LIST:  return v.list->len>0;
        case DGN_MAP:
        case DGN_INSTANCE: return v.map->len>0;
        default: return 1;
    }
}

static DgnVal dgn_add(DgnVal a, DgnVal b) {
    if(a.tag==DGN_STR||b.tag==DGN_STR){
        char* sa=dgn_val_to_cstr(a); char* sb=dgn_val_to_cstr(b);
        char* r=(char*)dgn_gc_alloc(strlen(sa)+strlen(sb)+1, DGN_STR);
        strcpy(r,sa); strcat(r,sb); free(sa); free(sb);
        DgnVal v; v.tag=DGN_STR; v.s=r; return v;
    }
    if(a.tag==DGN_LIST&&b.tag==DGN_LIST){
        DgnVal out=dgn_list_new(a.list->len+b.list->len);
        for(int i=0;i<a.list->len;i++) dgn_list_push(&out,a.list->data[i]);
        for(int i=0;i<b.list->len;i++) dgn_list_push(&out,b.list->data[i]);
        return out;
    }
    if(a.tag==DGN_FLOAT||b.tag==DGN_FLOAT) return dgn_float((a.tag==DGN_FLOAT?a.f:(double)a.i)+(b.tag==DGN_FLOAT?b.f:(double)b.i));
    return dgn_int(a.i+b.i);
}
static DgnVal dgn_sub(DgnVal a, DgnVal b) {
    if(a.tag==DGN_FLOAT||b.tag==DGN_FLOAT) return dgn_float((a.tag==DGN_FLOAT?a.f:(double)a.i)-(b.tag==DGN_FLOAT?b.f:(double)b.i));
    return dgn_int(a.i-b.i);
}
static DgnVal dgn_mul(DgnVal a, DgnVal b) {
    if(a.tag==DGN_FLOAT||b.tag==DGN_FLOAT) return dgn_float((a.tag==DGN_FLOAT?a.f:(double)a.i)*(b.tag==DGN_FLOAT?b.f:(double)b.i));
    if(a.tag==DGN_STR&&b.tag==DGN_INT){
        int n=(int)b.i; if(n<=0) return dgn_str("");
        char* r=(char*)dgn_gc_alloc((size_t)(strlen(a.s)*n+1), DGN_STR);
        r[0]=0; for(int i=0;i<n;i++) strcat(r,a.s);
        DgnVal v; v.tag=DGN_STR; v.s=r; return v;
    }
    return dgn_int(a.i*b.i);
}
static DgnVal dgn_div(DgnVal a, DgnVal b) {
    double fb=(b.tag==DGN_FLOAT)?b.f:(double)b.i;
    if(fb==0.0){ fprintf(stderr,"Dragon: Division by zero\n"); exit(1); }
    double fa=(a.tag==DGN_FLOAT)?a.f:(double)a.i;
    if(a.tag!=DGN_FLOAT&&b.tag!=DGN_FLOAT&&b.i!=0&&a.i%b.i==0) return dgn_int(a.i/b.i);
    return dgn_float(fa/fb);
}
static DgnVal dgn_mod(DgnVal a, DgnVal b) {
    if(a.tag==DGN_FLOAT||b.tag==DGN_FLOAT) return dgn_float(fmod(a.tag==DGN_FLOAT?a.f:(double)a.i,b.tag==DGN_FLOAT?b.f:(double)b.i));
    if(b.i==0){ fprintf(stderr,"Dragon: Modulo by zero\n"); exit(1); }
    return dgn_int(a.i%b.i);
}
static DgnVal dgn_neg(DgnVal a)    { return a.tag==DGN_FLOAT?dgn_float(-a.f):dgn_int(-a.i); }
static DgnVal dgn_bitnot(DgnVal a) { return dgn_int(~a.i); }
static DgnVal dgn_band(DgnVal a, DgnVal b) { return dgn_int(a.i&b.i); }
static DgnVal dgn_bor(DgnVal a, DgnVal b)  { return dgn_int(a.i|b.i); }
static DgnVal dgn_bxor(DgnVal a, DgnVal b) { return dgn_int(a.i^b.i); }
static DgnVal dgn_shl(DgnVal a, DgnVal b)  { return dgn_int(a.i<<b.i); }
static DgnVal dgn_shr(DgnVal a, DgnVal b)  { return dgn_int(a.i>>b.i); }

static int _dgn_num_cmp(DgnVal a, DgnVal b) {
    double fa=(a.tag==DGN_FLOAT)?a.f:(double)a.i, fb=(b.tag==DGN_FLOAT)?b.f:(double)b.i;
    return (fa>fb)-(fa<fb);
}
static DgnVal dgn_eq(DgnVal a, DgnVal b) {
    if(a.tag==DGN_NULL&&b.tag==DGN_NULL) return dgn_bool(1);
    if(a.tag==DGN_BOOL&&b.tag==DGN_BOOL) return dgn_bool(a.i==b.i);
    if((a.tag==DGN_INT||a.tag==DGN_FLOAT)&&(b.tag==DGN_INT||b.tag==DGN_FLOAT)) return dgn_bool(_dgn_num_cmp(a,b)==0);
    if(a.tag==DGN_STR&&b.tag==DGN_STR) return dgn_bool(a.s==b.s || (a.s[0]==b.s[0] && strcmp(a.s,b.s)==0));
    if(a.tag==DGN_LIST&&b.tag==DGN_LIST) return dgn_bool(a.list==b.list);
    if((a.tag==DGN_MAP||a.tag==DGN_INSTANCE||a.tag==DGN_CLASS)&&a.tag==b.tag) return dgn_bool(a.map==b.map);
    return dgn_bool(0);
}
static DgnVal dgn_neq(DgnVal a, DgnVal b) { return dgn_bool(!dgn_truthy(dgn_eq(a,b))); }
static DgnVal dgn_lt(DgnVal a, DgnVal b)  { if(a.tag==DGN_STR&&b.tag==DGN_STR) return dgn_bool(strcmp(a.s,b.s)<0); return dgn_bool(_dgn_num_cmp(a,b)<0); }
static DgnVal dgn_lte(DgnVal a, DgnVal b) { if(a.tag==DGN_STR&&b.tag==DGN_STR) return dgn_bool(strcmp(a.s,b.s)<=0); return dgn_bool(_dgn_num_cmp(a,b)<=0); }
static DgnVal dgn_gt(DgnVal a, DgnVal b)  { return dgn_lt(b,a); }
static DgnVal dgn_gte(DgnVal a, DgnVal b) { return dgn_lte(b,a); }
static DgnVal dgn_not(DgnVal a)           { return dgn_bool(!dgn_truthy(a)); }

static DgnVal dgn_in(DgnVal needle, DgnVal haystack) {
    if(haystack.tag==DGN_LIST){ for(int i=0;i<haystack.list->len;i++) if(dgn_truthy(dgn_eq(needle,haystack.list->data[i]))) return dgn_bool(1); return dgn_bool(0); }
    if(haystack.tag==DGN_MAP||haystack.tag==DGN_INSTANCE){ if(needle.tag!=DGN_STR) return dgn_bool(0); return dgn_bool(dgn_map_get(haystack,needle.s).tag!=DGN_NULL); }
    if(haystack.tag==DGN_STR&&needle.tag==DGN_STR) return dgn_bool(strstr(haystack.s,needle.s)!=NULL);
    return dgn_bool(0);
}

/* strings are C strings, so len(s) and s[i] would scan the whole string every time: remember the length of the last long string
   (strings are never changed in place; the memory of a collected string may be reused, so the garbage collector clears this) */
static size_t dgn_strlen_fast(const char* s) {
    if(s == _dgn_sl_p) return _dgn_sl_n;
    size_t n = strlen(s);
    if(n >= 256) { _dgn_sl_p = s; _dgn_sl_n = n; }
    return n;
}
static int64_t dgn_len_raw(DgnVal v) {
    if(v.tag==DGN_STR)  return (int64_t)dgn_strlen_fast(v.s);
    if(v.tag==DGN_LIST) return (int64_t)v.list->len;
    if(v.tag==DGN_MAP||v.tag==DGN_INSTANCE) return (int64_t)v.map->len;
    if(v.tag==DGN_INT)  return v.i;
    if(v.tag==DGN_FLOAT) return (int64_t)v.f;
    return 0;
}
static DgnVal dgn_len(DgnVal v) { return dgn_int(dgn_len_raw(v)); }

static DgnVal dgn_typeof(DgnVal v) {
    switch(v.tag){
        case DGN_NULL:     return dgn_str("null");
        case DGN_BOOL:     return dgn_str("bool");
        case DGN_INT:      return dgn_str("int");
        case DGN_FLOAT:    return dgn_str("number");
        case DGN_STR:      return dgn_str("string");
        case DGN_LIST:     return dgn_str("list");
        case DGN_MAP:      return dgn_str("map");
        case DGN_FUNC:     return dgn_str("func");
        case DGN_CLASS:    return dgn_str("class");
        case DGN_INSTANCE: return dgn_str("instance");
        default:           return dgn_str("unknown");
    }
}

static DgnVal dgn_to_str(DgnVal v) {
    char* sa = dgn_val_to_cstr(v);
    char* r = (char*)dgn_gc_alloc(strlen(sa)+1, DGN_STR);
    strcpy(r, sa); free(sa);
    DgnVal res; res.tag=DGN_STR; res.s=r; return res;
}
static DgnVal dgn_to_int(DgnVal v)   { if(v.tag==DGN_INT) return v; if(v.tag==DGN_FLOAT) return dgn_int((int64_t)v.f); if(v.tag==DGN_BOOL) return dgn_int(v.i); if(v.tag==DGN_STR) return dgn_int(atoll(v.s)); return dgn_int(0); }
static DgnVal dgn_to_float(DgnVal v) { if(v.tag==DGN_FLOAT) return v; if(v.tag==DGN_INT) return dgn_float((double)v.i); if(v.tag==DGN_BOOL) return dgn_float((double)v.i); if(v.tag==DGN_STR) return dgn_float(atof(v.s)); return dgn_float(0.0); }
static DgnVal dgn_chr(DgnVal v)      { char buf[2]; buf[0]=(char)v.i; buf[1]=0; return dgn_str(buf); }
static DgnVal dgn_ord(DgnVal v)      { if(v.tag!=DGN_STR||!v.s) return dgn_int(0); return dgn_int((unsigned char)v.s[0]); }

static DgnVal dgn_sleep(DgnVal v)  { DGN_SLEEP((int)((v.tag==DGN_FLOAT?v.f:(double)v.i)*1000)); return dgn_null(); }
static DgnVal dgn_exit(DgnVal v)   { exit((int)v.i); return dgn_null(); }

static DgnVal dgn_format(DgnVal* args, int n) {
    if(n<1||args[0].tag!=DGN_STR) return dgn_str("");
    const char* fmt=args[0].s;
    char out[8192]; out[0]=0; int oi=0,ai=1;
    for(const char* p=fmt;*p&&oi<(int)sizeof(out)-128;p++){
        if(*p=='{'&&*(p+1)=='}'){
            if(ai<n){ char* s=dgn_val_to_cstr(args[ai++]); int l=(int)strlen(s); if(oi+l<(int)sizeof(out)-1){strcpy(out+oi,s);oi+=l;} free(s); } p++;
        } else { out[oi++]=*p; }
    }
    out[oi]=0; return dgn_str(out);
}

static DgnVal dgn_range(DgnVal start, DgnVal stop, DgnVal step) {
    int64_t s=(start.tag==DGN_INT)?start.i:(int64_t)start.f;
    int64_t e=(stop.tag==DGN_INT)?stop.i:(int64_t)stop.f;
    int64_t st=(step.tag==DGN_INT)?step.i:(int64_t)step.f;
    if(st==0) st=1;
    DgnVal list=dgn_list_new(16);
    if(st>0){ for(int64_t i=s;i<e;i+=st) dgn_list_push(&list,dgn_int(i)); }
    else    { for(int64_t i=s;i>e;i+=st) dgn_list_push(&list,dgn_int(i)); }
    return list;
}

static DgnVal dgn_list_map_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2 || args[0].tag != DGN_LIST || args[1].tag != DGN_FUNC) return dgn_null();
    DgnList* l = args[0].list; DgnVal cb = args[1];
    DgnVal result = dgn_list_new(l->len);
    for(int i = 0; i < l->len; i++) {
        DgnVal call_args[2] = { l->data[i], dgn_int(i) };
        dgn_list_push(&result, dgn_call_val(cb, call_args, 2));
    }
    return result;
}

static DgnVal dgn_list_filter_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2 || args[0].tag != DGN_LIST || args[1].tag != DGN_FUNC) return dgn_list_new(0);
    DgnList* l = args[0].list; DgnVal cb = args[1];
    DgnVal result = dgn_list_new(0);
    for(int i = 0; i < l->len; i++) {
        DgnVal call_args[2] = { l->data[i], dgn_int(i) };
        if (dgn_truthy(dgn_call_val(cb, call_args, 2))) dgn_list_push(&result, l->data[i]);
    }
    return result;
}

static DgnVal dgn_list_reduce_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2 || args[0].tag != DGN_LIST || args[1].tag != DGN_FUNC) return dgn_null();
    DgnList* l = args[0].list; DgnVal cb = args[1];
    DgnVal acc = dgn_null(); int start = 0;
    if (n > 2) { acc = args[2]; } else if (l->len > 0) { acc = l->data[0]; start = 1; }
    for(int i = start; i < l->len; i++) {
        DgnVal call_args[3] = { acc, l->data[i], dgn_int(i) };
        acc = dgn_call_val(cb, call_args, 3);
    }
    return acc;
}

static DgnVal dgn_list_join(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 1 || args[0].tag != DGN_LIST) return dgn_str("");
    const char* sep = (n > 1 && args[1].tag == DGN_STR) ? args[1].s : "";
    size_t seplen = strlen(sep);
    DgnList* l = args[0].list;
    size_t cap = 256, len = 0;
    char* buf = (char*)malloc(cap);
    buf[0] = 0;
    for(int i = 0; i < l->len; i++) {
        char* s = dgn_val_to_cstr(l->data[i]);
        size_t sl = strlen(s);
        size_t need = len + sl + (i > 0 ? seplen : 0) + 1;
        if(need > cap) { while(cap < need) cap *= 2; buf = (char*)realloc(buf, cap); }
        if(i > 0) { memcpy(buf + len, sep, seplen); len += seplen; }
        memcpy(buf + len, s, sl); len += sl;
        buf[len] = 0;
        free(s);
    }
    DgnVal r = dgn_str(buf);
    free(buf);
    return r;
}

static DgnVal dgn_list_slice_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 1 || args[0].tag != DGN_LIST) return dgn_null();
    int len = args[0].list->len;
    int start = 0; int end = len;
    if (n > 1) start = (args[1].tag == DGN_INT) ? (int)args[1].i : (int)args[1].f;
    if (n > 2) end = (args[2].tag == DGN_INT) ? (int)args[2].i : (int)args[2].f;
    if (start < 0) start = (len + start > 0) ? len + start : 0; else if (start > len) start = len;
    if (end < 0) end = (len + end > 0) ? len + end : 0; else if (end > len) end = len;

    DgnVal sub = dgn_list_new(end > start ? end - start : 0);
    for (int i = start; i < end; i++) dgn_list_push(&sub, args[0].list->data[i]);
    return sub;
}

static DgnVal dgn_list_push_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2 || args[0].tag != DGN_LIST) return dgn_null();
    dgn_list_push(&args[0], args[1]);
    return args[0];
}

static DgnVal dgn_str_upper(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 1 || args[0].tag != DGN_STR) return dgn_str("");
    char* r = (char*)dgn_gc_alloc(strlen(args[0].s)+1, DGN_STR);
    strcpy(r, args[0].s);
    for(int i = 0; r[i]; i++) r[i] = toupper((unsigned char)r[i]);
    DgnVal v; v.tag=DGN_STR; v.s=r; return v;
}

static DgnVal dgn_str_lower(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 1 || args[0].tag != DGN_STR) return dgn_str("");
    char* r = (char*)dgn_gc_alloc(strlen(args[0].s)+1, DGN_STR);
    strcpy(r, args[0].s);
    for(int i = 0; r[i]; i++) r[i] = tolower((unsigned char)r[i]);
    DgnVal v; v.tag=DGN_STR; v.s=r; return v;
}

static DgnVal dgn_str_trim(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 1 || args[0].tag != DGN_STR) return dgn_str("");
    const char* s = args[0].s;
    while(*s && isspace((unsigned char)*s)) s++;
    if(*s == 0) return dgn_str("");
    const char* e = s + strlen(s) - 1;
    while(e > s && isspace((unsigned char)*e)) e--;
    int len = (int)(e - s + 1);
    char* r = (char*)dgn_gc_alloc(len+1, DGN_STR);
    strncpy(r, s, len); r[len] = 0;
    DgnVal v; v.tag=DGN_STR; v.s=r; return v;
}

static DgnVal dgn_str_split(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 1 || args[0].tag != DGN_STR) return dgn_list_new(0);
    const char* s = args[0].s;
    const char* sep = (n > 1 && args[1].tag == DGN_STR) ? args[1].s : "";
    DgnVal list = dgn_list_new(4);

    if(sep[0] == '\0') {
        for(int i=0; s[i]; i++) {
            char buf[2] = {s[i], 0};
            dgn_list_push(&list, dgn_str(buf));
        }
    } else {
        int seplen = (int)strlen(sep);
        const char* p = s;
        const char* match;
        while((match = strstr(p, sep)) != NULL) {
            int len = (int)(match - p);
            char* buf = (char*)dgn_gc_alloc(len+1, DGN_STR);
            strncpy(buf, p, len); buf[len] = 0;
            dgn_list_push(&list, dgn_str(buf));
            p = match + seplen;
        }
        dgn_list_push(&list, dgn_str(p));
    }
    return list;
}

static DgnVal dgn_str_replace(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 3 || args[0].tag != DGN_STR || args[1].tag != DGN_STR || args[2].tag != DGN_STR) return n>0?args[0]:dgn_null();
    const char* s = args[0].s; const char* old_s = args[1].s; const char* new_s = args[2].s;
    if(old_s[0] == '\0') return args[0];
    char out[65536]; out[0]=0; int oi=0;
    int old_len = (int)strlen(old_s); int new_len = (int)strlen(new_s);
    const char* p = s; const char* match;
    while((match = strstr(p, old_s)) != NULL) {
        int copy_len = (int)(match - p);
        if(oi + copy_len < 65530) { memcpy(out+oi, p, copy_len); oi += copy_len; }
        if(oi + new_len < 65530) { memcpy(out+oi, new_s, new_len); oi += new_len; }
        p = match + old_len;
    }
    int rem = (int)strlen(p);
    if(oi + rem < 65530) { memcpy(out+oi, p, rem); oi += rem; }
    out[oi] = 0;
    return dgn_str(out);
}

static DgnVal dgn_str_contains(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR || args[1].tag != DGN_STR) return dgn_bool(0);
    return dgn_bool(strstr(args[0].s, args[1].s) != NULL);
}

static DgnVal dgn_str_starts_with(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR || args[1].tag != DGN_STR) return dgn_bool(0);
    size_t off = 0;
    if(n > 2 && args[2].tag == DGN_INT && args[2].i > 0) { off = (size_t)args[2].i; if(off > strlen(args[0].s)) return dgn_bool(0); }
    return dgn_bool(strncmp(args[0].s + off, args[1].s, strlen(args[1].s)) == 0);
}

static DgnVal dgn_str_ends_with(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR || args[1].tag != DGN_STR) return dgn_bool(0);
    int len1 = (int)strlen(args[0].s); int len2 = (int)strlen(args[1].s);
    if(len2 > len1) return dgn_bool(0);
    return dgn_bool(strcmp(args[0].s + len1 - len2, args[1].s) == 0);
}

static DgnVal dgn_str_index_of(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR || args[1].tag != DGN_STR) return dgn_int(-1);
    size_t off = 0;
    if(n > 2 && args[2].tag == DGN_INT && args[2].i > 0) { off = (size_t)args[2].i; if(off > strlen(args[0].s)) return dgn_int(-1); }
    const char* match = strstr(args[0].s + off, args[1].s);
    if(match) return dgn_int((int64_t)(match - args[0].s));
    return dgn_int(-1);
}

static DgnVal dgn_str_substring(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR) return dgn_str("");
    int len = (int)strlen(args[0].s);
    int start = (args[1].tag == DGN_INT) ? (int)args[1].i : (int)args[1].f;
    int end = len;
    if(n > 2) end = (args[2].tag == DGN_INT) ? (int)args[2].i : (int)args[2].f;

    if(start < 0) start = 0; if(start > len) start = len;
    if(end < 0) end = 0; if(end > len) end = len;
    if(start > end) return dgn_str("");

    int sub_len = end - start;
    char* r = (char*)dgn_gc_alloc(sub_len + 1, DGN_STR);
    strncpy(r, args[0].s + start, sub_len); r[sub_len] = 0;
    return dgn_str(r);
}

static DgnVal dgn_map_set_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<3 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_null(); dgn_map_set(&args[0], args[1].s, args[2]); return dgn_null(); }
static DgnVal dgn_map_get_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_null(); return dgn_map_get(args[0], args[1].s); }
static DgnVal dgn_map_remove_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_bool(0); return dgn_bool(dgn_map_remove_key(args[0].map, args[1].s)); }
static DgnVal dgn_map_contains_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_bool(0); return dgn_bool(dgn_map_get(args[0], args[1].s).tag != DGN_NULL); }
static DgnVal dgn_map_clear_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_MAP) return dgn_null(); dgn_map_clear_all(args[0].map); return dgn_null(); }
static DgnVal dgn_map_keys_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_MAP) return dgn_list_new(0); DgnMap* m = args[0].map; DgnVal l = dgn_list_new(m->len); for(int i=0; i<m->cap; i++) if(m->entries[i].used) dgn_list_push(&l, dgn_str(m->entries[i].key)); return l; }
static DgnVal dgn_map_values_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_MAP) return dgn_list_new(0); DgnMap* m = args[0].map; DgnVal l = dgn_list_new(m->len); for(int i=0; i<m->cap; i++) if(m->entries[i].used) dgn_list_push(&l, m->entries[i].val); return l; }

static DgnVal dgn_list_pop_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST || args[0].list->len==0) return dgn_null(); return args[0].list->data[--args[0].list->len]; }
static DgnVal dgn_list_insert_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<3 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list; int idx = (int)(args[1].tag==DGN_INT?args[1].i:0); if(idx<0||idx>l->len) return dgn_null(); dgn_list_push(&args[0], dgn_null()); for(int i=l->len-1; i>idx; i--) l->data[i] = l->data[i-1]; l->data[idx] = args[2]; return dgn_null(); }
static DgnVal dgn_list_remove_at_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list; int idx = (int)(args[1].tag==DGN_INT?args[1].i:0); if(idx<0||idx>=l->len) return dgn_null(); DgnVal v = l->data[idx]; for(int i=idx; i<l->len-1; i++) l->data[i] = l->data[i+1]; l->len--; return v; }
static DgnVal dgn_list_reverse_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list; for(int i=0; i<l->len/2; i++) { DgnVal tmp = l->data[i]; l->data[i] = l->data[l->len-1-i]; l->data[l->len-1-i] = tmp; } return args[0]; }
static DgnVal dgn_list_first_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST || args[0].list->len==0) return dgn_null(); return args[0].list->data[0]; }
static DgnVal dgn_list_last_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST || args[0].list->len==0) return dgn_null(); return args[0].list->data[args[0].list->len-1]; }
static DgnVal dgn_list_contains_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_LIST) return dgn_bool(0); for(int i=0; i<args[0].list->len; i++) if(dgn_truthy(dgn_eq(args[0].list->data[i], args[1]))) return dgn_bool(1); return dgn_bool(0); }
static DgnVal dgn_list_sum_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_int(0); double sum=0; int is_float=0; for(int i=0; i<args[0].list->len; i++) { if(args[0].list->data[i].tag==DGN_FLOAT) { is_float=1; sum+=args[0].list->data[i].f; } else if(args[0].list->data[i].tag==DGN_INT) sum+=(double)args[0].list->data[i].i; } return is_float ? dgn_float(sum) : dgn_int((int64_t)sum); }

static DgnVal dgn_list_to_string_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_str("");
    DgnList* l = args[0].list;
    char* buf = (char*)dgn_gc_alloc(l->len + 1, DGN_STR);
    for(int i=0; i<l->len; i++) { buf[i] = (char)((args[0].list->data[i].tag == DGN_INT) ? args[0].list->data[i].i : 0); }
    buf[l->len] = '\0';
    DgnVal v; v.tag = DGN_STR; v.s = buf; return v;
}

static DgnVal dgn_list_find_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_LIST || args[1].tag != DGN_FUNC) return dgn_null();
    DgnList* l = args[0].list; DgnVal cb = args[1];
    for(int i = 0; i < l->len; i++) { DgnVal call_args[2] = { l->data[i], dgn_int(i) }; if (dgn_truthy(dgn_call_val(cb, call_args, 2))) return l->data[i]; }
    return dgn_null();
}
static DgnVal dgn_list_any_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_LIST || args[1].tag != DGN_FUNC) return dgn_bool(0);
    DgnList* l = args[0].list; DgnVal cb = args[1];
    for(int i = 0; i < l->len; i++) { DgnVal call_args[2] = { l->data[i], dgn_int(i) }; if (dgn_truthy(dgn_call_val(cb, call_args, 2))) return dgn_bool(1); }
    return dgn_bool(0);
}
static DgnVal dgn_list_all_method(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_LIST || args[1].tag != DGN_FUNC) return dgn_bool(0);
    DgnList* l = args[0].list; DgnVal cb = args[1];
    for(int i = 0; i < l->len; i++) { DgnVal call_args[2] = { l->data[i], dgn_int(i) }; if (!dgn_truthy(dgn_call_val(cb, call_args, 2))) return dgn_bool(0); }
    return dgn_bool(1);
}

static DgnVal dgn_list_unique_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_null();
    DgnList* l = args[0].list; DgnVal res = dgn_list_new(l->len);
    for(int i=0; i<l->len; i++) { int found = 0; for(int j=0; j<res.list->len; j++) if(dgn_truthy(dgn_eq(l->data[i], res.list->data[j]))) { found = 1; break; }
    if(!found) dgn_list_push(&res, l->data[i]); } return res;
}
static DgnVal dgn_list_flat_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_null();
    DgnList* l = args[0].list; DgnVal res = dgn_list_new(l->len);
    for(int i=0; i<l->len; i++) { if(l->data[i].tag == DGN_LIST) { for(int j=0; j<l->data[i].list->len; j++) dgn_list_push(&res, l->data[i].list->data[j]); } else dgn_list_push(&res, l->data[i]); } return res;
}
static DgnVal dgn_list_zip_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<2 || args[0].tag!=DGN_LIST || args[1].tag!=DGN_LIST) return dgn_list_new(0);
    DgnList* l1 = args[0].list; DgnList* l2 = args[1].list; int min_len = l1->len < l2->len ? l1->len : l2->len;
    DgnVal res = dgn_list_new(min_len);
    for(int i=0; i<min_len; i++) { DgnVal pair = dgn_list_new(2); dgn_list_push(&pair, l1->data[i]); dgn_list_push(&pair, l2->data[i]); dgn_list_push(&res, pair); } return res;
}
static void dgn_msort_rec(DgnVal* k, DgnVal* v, DgnVal* tk, DgnVal* tv, int lo, int hi) {
    if (hi - lo < 2) return;
    int mid = lo + (hi - lo) / 2;
    dgn_msort_rec(k, v, tk, tv, lo, mid);
    dgn_msort_rec(k, v, tk, tv, mid, hi);
    int a = lo, b = mid, o = lo;
    while (a < mid && b < hi) {
        if (dgn_truthy(dgn_gt(k[a], k[b]))) { tk[o] = k[b]; if (v) tv[o] = v[b]; b++; }
        else { tk[o] = k[a]; if (v) tv[o] = v[a]; a++; }
        o++;
    }
    while (a < mid) { tk[o] = k[a]; if (v) tv[o] = v[a]; a++; o++; }
    while (b < hi) { tk[o] = k[b]; if (v) tv[o] = v[b]; b++; o++; }
    for (int i = lo; i < hi; i++) { k[i] = tk[i]; if (v) v[i] = tv[i]; }
}
static void dgn_msort(DgnVal* k, DgnVal* v, int n) {
    if (n < 2) return;
    DgnVal* tk = (DgnVal*)malloc(n * sizeof(DgnVal));
    DgnVal* tv = v ? (DgnVal*)malloc(n * sizeof(DgnVal)) : NULL;
    dgn_msort_rec(k, v, tk, tv, 0, n);
    free(tk); if (tv) free(tv);
}
static DgnVal dgn_list_sort_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list;
    dgn_msort(l->data, NULL, l->len);
    return args[0];
}
static DgnVal dgn_list_sort_by_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<2 || args[0].tag!=DGN_LIST || args[1].tag!=DGN_FUNC) return dgn_null(); DgnList* l = args[0].list; DgnVal cb = args[1];
    DgnVal keyl = dgn_list_new(l->len);
    for(int i=0; i<l->len; i++) { DgnVal call_args[1] = { l->data[i] }; DgnVal kv = dgn_call_val(cb, call_args, 1); dgn_list_push(&keyl, kv); }
    dgn_msort(keyl.list->data, l->data, l->len);
    return args[0];
}

DgnVal _dgn_aug_str;
DgnVal _dgn_aug_list;
int _dgn_aug_init = 0;
static void _init_augments(void);

static DgnVal dgn_get_attr(DgnVal obj, const char* key) {
    _init_augments();

    if(obj.tag==DGN_MAP || obj.tag==DGN_CLASS || obj.tag==DGN_INSTANCE) {
        DgnVal v = dgn_map_get(obj, key);
        if (v.tag != DGN_NULL) return v;
        if (obj.tag==DGN_INSTANCE || obj.tag==DGN_CLASS) {
            DgnVal par = dgn_map_get(obj, "__parent__"); int g = 0;
            while ((par.tag==DGN_CLASS||par.tag==DGN_MAP) && g++ < 64) {
                DgnVal pv = dgn_map_get(par, key);
                if (pv.tag != DGN_NULL) return pv;
                par = dgn_map_get(par, "__parent__");
            }
        }

        if(strcmp(key,"set")==0) return dgn_func(dgn_map_set_method, 3);
        if(strcmp(key,"get")==0) return dgn_func(dgn_map_get_method, 2);
        if(strcmp(key,"remove")==0) return dgn_func(dgn_map_remove_method, 2);
        if(strcmp(key,"contains")==0) return dgn_func(dgn_map_contains_method, 2);
        if(strcmp(key,"clear")==0) return dgn_func(dgn_map_clear_method, 1);
        if(strcmp(key,"keys")==0) return dgn_func(dgn_map_keys_method, 1);
        if(strcmp(key,"values")==0) return dgn_func(dgn_map_values_method, 1);

        return dgn_null();
    }

    if(obj.tag==DGN_STR){
        if(strcmp(key,"length")==0) return dgn_int((int64_t)strlen(obj.s));
        if(strcmp(key,"upper")==0) return dgn_func(dgn_str_upper, 1);
        if(strcmp(key,"lower")==0) return dgn_func(dgn_str_lower, 1);
        if(strcmp(key,"trim")==0) return dgn_func(dgn_str_trim, 1);
        if(strcmp(key,"split")==0) return dgn_func(dgn_str_split, -1);
        if(strcmp(key,"replace")==0) return dgn_func(dgn_str_replace, 3);
        if(strcmp(key,"contains")==0) return dgn_func(dgn_str_contains, 2);
        if(strcmp(key,"starts_with")==0) return dgn_func(dgn_str_starts_with, -1);
        if(strcmp(key,"ends_with")==0) return dgn_func(dgn_str_ends_with, 2);
        if(strcmp(key,"index_of")==0) return dgn_func(dgn_str_index_of, -1);
        if(strcmp(key,"substring")==0) return dgn_func(dgn_str_substring, 3);

        DgnVal aug = dgn_map_get(_dgn_aug_str, key);
        if(aug.tag != DGN_NULL) return aug;
    }

    if(obj.tag==DGN_LIST){
        if(strcmp(key,"length")==0||strcmp(key,"len")==0) return dgn_int((int64_t)obj.list->len);
        if(strcmp(key,"join")==0) return dgn_func(dgn_list_join, 1);
        if(strcmp(key,"slice")==0) return dgn_func(dgn_list_slice_method, -1);
        if(strcmp(key,"push")==0) return dgn_func(dgn_list_push_method, 1);
        if(strcmp(key,"map")==0) return dgn_func(dgn_list_map_method, 1);
        if(strcmp(key,"filter")==0) return dgn_func(dgn_list_filter_method, 1);
        if(strcmp(key,"reduce")==0) return dgn_func(dgn_list_reduce_method, 2);

        if(strcmp(key,"pop")==0) return dgn_func(dgn_list_pop_method, 1);
        if(strcmp(key,"insert")==0) return dgn_func(dgn_list_insert_method, 3);
        if(strcmp(key,"remove_at")==0) return dgn_func(dgn_list_remove_at_method, 2);
        if(strcmp(key,"reverse")==0) return dgn_func(dgn_list_reverse_method, 1);
        if(strcmp(key,"first")==0) return dgn_func(dgn_list_first_method, 1);
        if(strcmp(key,"last")==0) return dgn_func(dgn_list_last_method, 1);
        if(strcmp(key,"contains")==0) return dgn_func(dgn_list_contains_method, 2);
        if(strcmp(key,"sum")==0) return dgn_func(dgn_list_sum_method, 1);
        if(strcmp(key,"to_string")==0) return dgn_func(dgn_list_to_string_method, 1);
        if(strcmp(key,"find")==0) return dgn_func(dgn_list_find_method, 2);
        if(strcmp(key,"any")==0) return dgn_func(dgn_list_any_method, 2);
        if(strcmp(key,"all")==0) return dgn_func(dgn_list_all_method, 2);

        if(strcmp(key,"unique")==0) return dgn_func(dgn_list_unique_method, 1);
        if(strcmp(key,"flat")==0) return dgn_func(dgn_list_flat_method, 1);
        if(strcmp(key,"zip")==0) return dgn_func(dgn_list_zip_method, 2);
        if(strcmp(key,"sort")==0) return dgn_func(dgn_list_sort_method, 1);
        if(strcmp(key,"sort_by")==0) return dgn_func(dgn_list_sort_by_method, 2);

        DgnVal aug = dgn_map_get(_dgn_aug_list, key);
        if(aug.tag != DGN_NULL) return aug;
    }
    return dgn_null();
}

static void dgn_set_attr(DgnVal* obj, const char* key, DgnVal val) {
    if(obj->tag==DGN_MAP||obj->tag==DGN_CLASS||obj->tag==DGN_INSTANCE) dgn_map_set(obj,key,val);
}
static DgnVal dgn_get_index(DgnVal obj, DgnVal idx) {
    if(obj.tag==DGN_LIST){ int64_t i=(idx.tag==DGN_INT)?idx.i:(int64_t)idx.f; if(i<0)i+=obj.list->len; if(i>=0&&i<obj.list->len) return obj.list->data[i]; return dgn_null(); }
    if(obj.tag==DGN_STR&&(idx.tag==DGN_INT||idx.tag==DGN_FLOAT)){ int64_t i=(idx.tag==DGN_INT)?idx.i:(int64_t)idx.f; size_t sl_=dgn_strlen_fast(obj.s); if(i<0)i+=(int64_t)sl_; if(i>=0&&(size_t)i<sl_){ char buf[2]; buf[0]=obj.s[i]; buf[1]=0; return dgn_str(buf); } return dgn_null(); }
    if((obj.tag==DGN_MAP||obj.tag==DGN_INSTANCE)&&idx.tag==DGN_STR) return dgn_map_get(obj,idx.s);
    return dgn_null();
}
static void dgn_set_index(DgnVal* obj, DgnVal idx, DgnVal val) {
    if(obj->tag==DGN_LIST){ int64_t i=(idx.tag==DGN_INT)?idx.i:(int64_t)idx.f; if(i>=0&&i<obj->list->len) obj->list->data[i]=val; return; }
    if((obj->tag==DGN_MAP||obj->tag==DGN_INSTANCE)&&idx.tag==DGN_STR) dgn_map_set(obj,idx.s,val);
}

static DgnVal dgn_iter_get(DgnVal obj, int i) {
    if(obj.tag==DGN_LIST){ if(i>=0&&i<obj.list->len) return obj.list->data[i]; return dgn_null(); }
    if(obj.tag==DGN_STR){ int l=(int)dgn_strlen_fast(obj.s); if(i>=0&&i<l){ char buf[2]; buf[0]=obj.s[i]; buf[1]=0; return dgn_str(buf); } return dgn_null(); }
    if(obj.tag==DGN_MAP||obj.tag==DGN_INSTANCE){ int seen=0,k; for(k=0;k<obj.map->cap;k++) if(obj.map->entries[k].used){ if(seen==i) return dgn_str(obj.map->entries[k].key); seen++; } return dgn_null(); }
    if(obj.tag==DGN_INT||obj.tag==DGN_FLOAT){ int64_t max=(obj.tag==DGN_INT)?obj.i:(int64_t)obj.f; if(i>=0&&i<max) return dgn_int(i); return dgn_null(); }
    return dgn_null();
}
static DgnVal dgn_iter_pair(DgnVal obj, int i) {
    DgnVal pair=dgn_list_new(2);
    if(obj.tag==DGN_MAP||obj.tag==DGN_INSTANCE){
        int seen=0,k;
        for(k=0;k<obj.map->cap;k++) {
            if(obj.map->entries[k].used){
                if(seen==i){ dgn_list_push(&pair,dgn_str(obj.map->entries[k].key)); dgn_list_push(&pair,obj.map->entries[k].val); return pair; }
                seen++;
            }
        }
    }
    else if(obj.tag==DGN_LIST){
        dgn_list_push(&pair, dgn_int(i));
        if(i>=0 && i<obj.list->len) dgn_list_push(&pair, obj.list->data[i]);
        else dgn_list_push(&pair, dgn_null());
    }
    else if(obj.tag==DGN_STR){
        dgn_list_push(&pair, dgn_int(i));
        int l=(int)strlen(obj.s);
        if(i>=0 && i<l) { char buf[2]; buf[0]=obj.s[i]; buf[1]=0; dgn_list_push(&pair, dgn_str(buf)); }
        else dgn_list_push(&pair, dgn_null());
    }
    else if(obj.tag==DGN_INT || obj.tag==DGN_FLOAT){
        dgn_list_push(&pair, dgn_int(i));
        int64_t max = (obj.tag==DGN_INT) ? obj.i : (int64_t)obj.f;
        if(i>=0 && i<max) dgn_list_push(&pair, dgn_int(i));
        else dgn_list_push(&pair, dgn_null());
    }
    else { dgn_list_push(&pair, dgn_null()); dgn_list_push(&pair, dgn_null()); }
    return pair;
}

static DgnVal dgn_func(DgnFuncPtr fn, int arity) {
    DgnVal v; v.tag=DGN_FUNC;
    v.func=(DgnFunc*)dgn_gc_alloc(sizeof(DgnFunc), DGN_FUNC);
    v.func->fn=fn; v.func->arity=arity; v.func->env=NULL; v.func->pnames=NULL; v.func->skip=0;
    return v;
}
static DgnVal dgn_closure(DgnFuncPtr fn, int arity, DgnMap* env) {
    DgnVal v; v.tag=DGN_FUNC;
    v.func=(DgnFunc*)dgn_gc_alloc(sizeof(DgnFunc), DGN_FUNC);
    v.func->fn=fn; v.func->arity=arity; v.func->env=env; v.func->pnames=NULL; v.func->skip=0;
    return v;
}
static DgnVal dgn_map_get_raw(DgnMap* m, const char* key) {
    if(!m || !key) return dgn_null();
    DgnVal v; v.tag = DGN_MAP; v.map = m;
    return dgn_map_get(v, key);
}

static void dgn_map_set_raw(DgnMap* m, const char* key, DgnVal val) {
    if(!m || !key) return;
    DgnVal v; v.tag = DGN_MAP; v.map = m;
    dgn_map_set(&v, key, val);
}
static DgnVal dgn_func_n(DgnFuncPtr fn, int arity, const char* pn){ DgnVal v=dgn_func(fn,arity); v.func->pnames=pn; v.func->skip=-1; return v; }
static DgnVal dgn_closure_n(DgnFuncPtr fn, int arity, DgnMap* env, const char* pn){ DgnVal v=dgn_closure(fn,arity,env); v.func->pnames=pn; v.func->skip=-1; return v; }
static DgnVal dgn_new_instance(DgnVal cls, DgnVal* args, int n);
static int dgn_is_named_pack(DgnVal v){
    if(v.tag!=DGN_MAP) return 0;
    for(int i=0;i<v.map->cap;i++) if(v.map->entries[i].used && strcmp(v.map->entries[i].key,"__named__")==0) return 1;
    return 0;
}
static DgnVal dgn_call_fn_bound(DgnVal fv, DgnVal* args, int n){
    DgnFunc* f=fv.func;
    if(n>0 && f->pnames && dgn_is_named_pack(args[n-1])){
        DgnVal pack=args[n-1];
        int cnt=1; for(const char* c=f->pnames;*c;c++) if(*c==',') cnt++;
        if(!*f->pnames) cnt=0;
        int skip=f->skip>0?f->skip:0;
        int total=skip+cnt; int pos=n-1; if(pos>total) total=pos;
        DgnVal* na=(DgnVal*)malloc((size_t)(total>0?total:1)*sizeof(DgnVal));
        for(int i=0;i<total;i++) na[i]=dgn_null();
        for(int i=0;i<pos;i++) na[i]=args[i];
        for(int e=0;e<pack.map->cap;e++){
            if(!pack.map->entries[e].used) continue;
            const char* k=pack.map->entries[e].key; if(strcmp(k,"__named__")==0) continue;
            int idx=-1, j=0; const char* p=f->pnames; size_t kl=strlen(k);
            while(*p){ const char* q=strchr(p,','); size_t L=q?(size_t)(q-p):strlen(p); if(L==kl && strncmp(p,k,L)==0){ idx=j; break; } j++; if(!q) break; p=q+1; }
            if(idx<0){ fprintf(stderr,"Dragon: function has no parameter named '%s' (parameters: %s)\n",k,f->pnames); continue; }
            na[skip+idx]=pack.map->entries[e].val;
        }
        DgnEnv env_s; env_s.map=f->env;
        DgnVal r=f->fn(na,total,f->env?&env_s:NULL); free(na); return r;
    }
    DgnEnv env_s; env_s.map=f->env;
    return f->fn(args,n,f->env?&env_s:NULL);
}
static DgnVal dgn_call_val(DgnVal fv, DgnVal* args, int n) {
    if(fv.tag==DGN_CLASS) return dgn_new_instance(fv,args,n);
    if(fv.tag==DGN_FUNC&&fv.func&&fv.func->fn) { return dgn_call_fn_bound(fv,args,n); }
    if(0) {
        DgnEnv env_s; env_s.map=fv.func->env;
        return fv.func->fn(args, n, fv.func->env ? &env_s : NULL);
    }
    fprintf(stderr,"Dragon: attempt to call non-function value (tag=%d)\n",fv.tag);
    return dgn_null();
}
static DgnVal dgn_call_method(DgnVal obj, const char* method, DgnVal* args, int n) {
    DgnVal fn=dgn_get_attr(obj,method);
    if(fn.tag==DGN_NULL){ DgnVal parent=dgn_get_attr(obj,"__parent__"); while(parent.tag==DGN_MAP||parent.tag==DGN_CLASS||parent.tag==DGN_INSTANCE){ fn=dgn_get_attr(parent,method); if(fn.tag!=DGN_NULL) break; parent=dgn_get_attr(parent,"__parent__"); } }
    if(fn.tag!=DGN_FUNC) return dgn_null();
    if(fn.func->skip<0) return dgn_call_val(fn,args,n);
    DgnVal* nargs=(DgnVal*)malloc((size_t)(n+1)*sizeof(DgnVal)); nargs[0]=obj; memcpy(nargs+1,args,(size_t)n*sizeof(DgnVal));
    DgnVal result=dgn_call_val(fn,nargs,n+1); free(nargs); return result;
}

static DgnVal dgn_class_new(const char* name, const char* parent_name) {
    DgnVal v=dgn_map_new(); v.tag=DGN_CLASS;
    if(name) dgn_map_set(&v,"__name__",dgn_str(name));
    (void)parent_name; return v;
}
static void dgn_class_add_method(DgnVal* cls, const char* name, DgnFuncPtr fn, int arity) { dgn_map_set(cls,name,dgn_func(fn,arity)); }
static void dgn_class_add_method_n(DgnVal* cls, const char* name, DgnFuncPtr fn, int arity, const char* pn) { DgnVal f=dgn_func(fn,arity); f.func->pnames=pn; f.func->skip=1; dgn_map_set(cls,name,f); }
static void dgn_class_set_parent(DgnVal* cls, DgnVal parent) { dgn_map_set(cls,"__parent__",parent); }
static void dgn_class_init_statics(DgnVal* cls) {
    DgnVal fi = dgn_map_get(*cls, "__field_init__");
    if(fi.tag == DGN_FUNC) { DgnVal a[1]; a[0] = *cls; dgn_call_val(fi, a, 1); }
}
static DgnVal dgn_new_instance(DgnVal cls, DgnVal* args, int n) {
    DgnVal inst; inst.tag=DGN_INSTANCE;
    inst.map=(DgnMap*)dgn_gc_alloc(sizeof(DgnMap), DGN_INSTANCE); inst.map->cap=8; inst.map->n=0; inst.map->idx=NULL; inst.map->icap=0; inst.map->entries=(DgnMapEntry*)calloc(8,sizeof(DgnMapEntry));
    if(cls.tag==DGN_CLASS||cls.tag==DGN_MAP){ for(int i=0;i<cls.map->cap;i++) if(cls.map->entries[i].used) dgn_map_set(&inst,cls.map->entries[i].key,cls.map->entries[i].val); }
    { DgnVal chain[64]; int cn=0; DgnVal cur=cls;
      while((cur.tag==DGN_CLASS||cur.tag==DGN_MAP) && cn<64){ chain[cn++]=cur; cur=dgn_map_get(cur,"__parent__"); }
      for(int ci=cn-1; ci>=0; ci--){
          DgnVal fi=dgn_map_get(chain[ci],"__field_init__");
          if(fi.tag==DGN_FUNC){ DgnVal fi_args[1]; fi_args[0]=inst; dgn_call_val(fi,fi_args,1); }
      }
      DgnVal init_fn=dgn_null();
      for(int ci=0; ci<cn; ci++){ init_fn=dgn_map_get(chain[ci],"init"); if(init_fn.tag==DGN_FUNC) break; }
      if(init_fn.tag==DGN_FUNC){ DgnVal* nargs=(DgnVal*)malloc((size_t)(n+1)*sizeof(DgnVal)); nargs[0]=inst; if(n>0) memcpy(nargs+1,args,(size_t)n*sizeof(DgnVal)); dgn_call_val(init_fn,nargs,n+1); free(nargs); }
    }
    return inst;
}
static DgnVal dgn_super_call(DgnVal inst, const char* method, DgnVal* args, int n) {
    DgnVal parent = dgn_map_get(inst, "__parent__"); DgnVal fn = dgn_null();
    while(parent.tag == DGN_MAP || parent.tag == DGN_CLASS || parent.tag == DGN_INSTANCE) {
        fn = dgn_map_get(parent, method);
        if(fn.tag == DGN_FUNC) break;
        parent = dgn_map_get(parent, "__parent__");
    }
    if(fn.tag != DGN_FUNC) return dgn_null();
    DgnVal* nargs = (DgnVal*)malloc((size_t)(n+1)*sizeof(DgnVal));
    nargs[0] = inst; if(n > 0) memcpy(nargs+1, args, (size_t)n*sizeof(DgnVal));
    DgnVal result = dgn_call_val(fn, nargs, n+1); free(nargs); return result;
}

static DgnVal dgn_super_call_from(DgnVal inst, DgnVal defcls, const char* method, DgnVal* args, int n) {
    DgnVal parent = dgn_map_get(defcls, "__parent__"); DgnVal fn = dgn_null(); int g = 0;
    while((parent.tag == DGN_MAP || parent.tag == DGN_CLASS) && g++ < 64) {
        fn = dgn_map_get(parent, method);
        if(fn.tag == DGN_FUNC) break;
        parent = dgn_map_get(parent, "__parent__");
    }
    if(fn.tag != DGN_FUNC) return dgn_null();
    DgnVal* nargs = (DgnVal*)malloc((size_t)(n+1)*sizeof(DgnVal));
    nargs[0] = inst; if(n > 0) memcpy(nargs+1, args, (size_t)n*sizeof(DgnVal));
    DgnVal result = dgn_call_val(fn, nargs, n+1); free(nargs); return result;
}

#define MAX_TRY_DEPTH 256
#ifdef DGN_PERCPU
#define _dgn_try_stack PC(try_stack)
#define _dgn_try_top PC(try_top)
#else
static DgnTryFrame* _dgn_try_stack[MAX_TRY_DEPTH];
static int _dgn_try_top = 0;
#endif
static void dgn_try_push(DgnTryFrame* f) { if (_dgn_try_top >= MAX_TRY_DEPTH) { fprintf(stderr, "Dragon Error: Try/Catch overflow\n"); exit(1); } _dgn_try_stack[_dgn_try_top++] = f; }
#ifdef DGN_BAREMETAL
int  dgn_bm_try_cap(void) { return MAX_TRY_DEPTH; }
void dgn_bm_try_save(void** dst, int* top) { int n = _dgn_try_top; for (int i = 0; i < n; i++) dst[i] = (void*)_dgn_try_stack[i]; *top = n; }
void dgn_bm_try_load(void** src, int top) { for (int i = 0; i < top; i++) _dgn_try_stack[i] = (DgnTryFrame*)src[i]; _dgn_try_top = top; }
void* dgn_bm_boot_stack_bottom(void) { return _dgn_stack_bottom; }
#endif
static void dgn_try_pop(DgnTryFrame* f) { if (_dgn_try_top > 0 && _dgn_try_stack[_dgn_try_top - 1] == f) _dgn_try_top--; }
static void dgn_raise(DgnVal v) {
    if (_dgn_try_top > 0) { DgnTryFrame* f = _dgn_try_stack[--_dgn_try_top]; f->thrown = v; longjmp(f->buf, 1); }
    char* s = dgn_val_to_cstr(v); fprintf(stderr, "Dragon Error: %s\n", s); free(s); exit(1);
}

static void _init_augments(void) {
    if(!_dgn_aug_init) { _dgn_aug_str = dgn_map_new(); _dgn_aug_list = dgn_map_new(); _dgn_aug_init = 1; }
}
static void dgn_augment_register(const char* type, const char* method, DgnFuncPtr fn) {
    _init_augments();
    if(strcmp(type, "string") == 0) dgn_map_set(&_dgn_aug_str, method, dgn_func(fn, -1));
    else if(strcmp(type, "list") == 0) dgn_map_set(&_dgn_aug_list, method, dgn_func(fn, -1));
}

static DgnVal dgn_range_wrap(DgnVal* args, int n, DgnEnv* env) {
    (void)env; int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    int argc = n - off; DgnVal* argv = args + off;
    if(argc == 1) return dgn_range(dgn_int(0), argv[0], dgn_int(1));
    if(argc == 2) return dgn_range(argv[0], argv[1], dgn_int(1));
    if(argc >= 3) return dgn_range(argv[0], argv[1], argv[2]);
    return dgn_list_new(0);
}

#define MATH_WRAP1(name, fnc) static DgnVal dgn_math_##name(DgnVal* args, int n, DgnEnv* env) { (void)env; int off = (n>0 && args[0].tag==DGN_MAP) ? 1 : 0; if(n - off < 1) return dgn_float(0.0); double v = (args[off].tag==DGN_FLOAT) ? args[off].f : (double)args[off].i; return dgn_float(fnc(v)); }
#define MATH_WRAP2(name, fnc) static DgnVal dgn_math_##name(DgnVal* args, int n, DgnEnv* env) { (void)env; int off = (n>0 && args[0].tag==DGN_MAP) ? 1 : 0; if(n - off < 2) return dgn_float(0.0); double v1 = (args[off].tag==DGN_FLOAT) ? args[off].f : (double)args[off].i; double v2 = (args[off+1].tag==DGN_FLOAT) ? args[off+1].f : (double)args[off+1].i; return dgn_float(fnc(v1, v2)); }

MATH_WRAP1(sqrt, sqrt) MATH_WRAP1(floor, floor) MATH_WRAP1(ceil, ceil) MATH_WRAP1(abs, fabs)
MATH_WRAP1(sin, sin)   MATH_WRAP1(cos, cos)     MATH_WRAP1(tan, tan)   MATH_WRAP1(log, log)
MATH_WRAP1(exp, exp)   MATH_WRAP1(round, round)
MATH_WRAP2(pow, pow)   MATH_WRAP2(atan2, atan2)

static DgnVal dgn_file_read_text(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();

    FILE* f = fopen(args[off].s, "rb");
    if(!f) return dgn_null();

    fseek(f, 0, SEEK_END);
    long length = ftell(f);
    fseek(f, 0, SEEK_SET);

    if(length < 0) { fclose(f); return dgn_null(); }

    char* buffer = (char*)dgn_gc_alloc(length + 1, DGN_STR);
    size_t read_bytes = fread(buffer, 1, length, f);
    buffer[read_bytes] = '\0';
    fclose(f);

    DgnVal v; v.tag = DGN_STR; v.s = buffer;
    return v;
}

static DgnVal dgn_file_read_bytes(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();

    FILE* f = fopen(args[off].s, "rb");
    if(!f) return dgn_null();

    fseek(f, 0, SEEK_END);
    long length = ftell(f);
    fseek(f, 0, SEEK_SET);

    if(length < 0) { fclose(f); return dgn_null(); }

    unsigned char* buf = (unsigned char*)malloc(length);
    if(!buf) { fclose(f); return dgn_null(); }

    size_t got = fread(buf, 1, (size_t)length, f);
    fclose(f);

    DgnVal list = dgn_list_new((int)got);
    for(size_t i = 0; i < got; i++)
        dgn_list_push(&list, dgn_int((int64_t)buf[i]));
    free(buf);

    return list;
}

static DgnVal dgn_file_write_text(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2 || args[off].tag != DGN_STR) return dgn_bool(0);

    FILE* f = fopen(args[off].s, "w");
    if(!f) return dgn_bool(0);

    char* content = dgn_val_to_cstr(args[off+1]);
    fputs(content, f);
    free(content);
    fclose(f);

    return dgn_bool(1);
}

static DgnVal dgn_file_append_text(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2 || args[off].tag != DGN_STR) return dgn_bool(0);

    FILE* f = fopen(args[off].s, "a");
    if(!f) return dgn_bool(0);

    char* content = dgn_val_to_cstr(args[off+1]);
    fputs(content, f);
    free(content);
    fclose(f);

    return dgn_bool(1);
}

static DgnVal dgn_file_exists(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);

    FILE* f = fopen(args[off].s, "r");
    if(f) { fclose(f); return dgn_bool(1); }
    return dgn_bool(0);
}

static DgnVal dgn_file_delete(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);

    return dgn_bool(remove(args[off].s) == 0);
}

static DgnVal dgn_file_mkdir(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);
#ifdef _WIN32
    return dgn_bool(CreateDirectoryA(args[off].s, NULL) != 0 || GetLastError() == ERROR_ALREADY_EXISTS);
#else
    return dgn_bool(mkdir(args[off].s, 0755) == 0 || errno == EEXIST);
#endif
}

static DgnVal dgn_file_rmdir(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);
#ifdef _WIN32
    return dgn_bool(RemoveDirectoryA(args[off].s) != 0);
#else
    return dgn_bool(rmdir(args[off].s) == 0);
#endif
}

static DgnVal dgn_file_rename(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2 || args[off].tag != DGN_STR || args[off+1].tag != DGN_STR) return dgn_bool(0);
    return dgn_bool(rename(args[off].s, args[off+1].s) == 0);
}

static DgnVal dgn_file_size(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_float(-1.0);
    FILE* f = fopen(args[off].s, "rb");
    if(!f) return dgn_float(-1.0);
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fclose(f);
    return dgn_float((double)sz);
}

static DgnVal dgn_file_is_dir(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);
#ifdef _WIN32
    DWORD attr = GetFileAttributesA(args[off].s);
    return dgn_bool(attr != INVALID_FILE_ATTRIBUTES && (attr & FILE_ATTRIBUTE_DIRECTORY));
#else
    struct stat st;
    return dgn_bool(stat(args[off].s, &st) == 0 && S_ISDIR(st.st_mode));
#endif
}

   typedef struct { char url[4096]; char method[8]; char body[65536]; char headers[4096]; } DgnHttpCtx;
typedef struct { char host[256]; int port; } DgnConnectCtx;
typedef struct { int64_t fd; char msg[65536]; } DgnSocketCtx;
typedef struct { int port; DgnVal handler; } DgnServerCtx;

static DgnVal dgn_net_do_http(void* arg);
static DgnVal dgn_make_promise(DgnVal (*fn)(void*), void* ctx);
static DgnVal dgn_net_http_server_task(void* arg);
static DgnVal dgn_socket_make(int64_t fd);
static int64_t dgn_socket_fd(DgnVal v);

static DgnVal dgn_net_http_get_task(void* arg) {
    DgnHttpCtx* ctx = (DgnHttpCtx*)arg;
    strcpy(ctx->method, "GET"); ctx->body[0] = 0;
    return dgn_net_do_http(arg);
}
static DgnVal dgn_net_http_get(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    DgnHttpCtx* ctx = (DgnHttpCtx*)malloc(sizeof(DgnHttpCtx));
    strncpy(ctx->url, args[off].s, sizeof(ctx->url)-1);
    strcpy(ctx->method, "GET"); ctx->body[0]=0; ctx->headers[0]=0;
    return dgn_make_promise(dgn_net_http_get_task, ctx);
}

static DgnVal dgn_net_http_post_task(void* arg) { return dgn_net_do_http(arg); }
static DgnVal dgn_net_http_post(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    DgnHttpCtx* ctx = (DgnHttpCtx*)malloc(sizeof(DgnHttpCtx));
    strncpy(ctx->url, args[off].s, sizeof(ctx->url)-1);
    strcpy(ctx->method, "POST");
    if(n - off >= 2) { char* b = dgn_val_to_cstr(args[off+1]); strncpy(ctx->body, b, sizeof(ctx->body)-1); free(b); }
    else ctx->body[0]=0;
    ctx->headers[0]=0;
    if(n - off >= 3 && args[off+2].tag == DGN_MAP) {
        for(int i=0; i<args[off+2].map->cap; i++) {
            if(args[off+2].map->entries[i].used) {
                char* v = dgn_val_to_cstr(args[off+2].map->entries[i].val);
                char tmp[256]; snprintf(tmp,sizeof(tmp),"%s: %s\r\n",args[off+2].map->entries[i].key,v);
                strncat(ctx->headers,tmp,sizeof(ctx->headers)-strlen(ctx->headers)-1);
                free(v);
            }
        }
    }
    return dgn_make_promise(dgn_net_http_post_task, ctx);
}

static DgnVal dgn_net_http_put(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    DgnHttpCtx* ctx = (DgnHttpCtx*)malloc(sizeof(DgnHttpCtx));
    strncpy(ctx->url, args[off].s, sizeof(ctx->url)-1);
    strcpy(ctx->method, "PUT");
    if(n - off >= 2) { char* b=dgn_val_to_cstr(args[off+1]); strncpy(ctx->body,b,sizeof(ctx->body)-1); free(b); }
    else ctx->body[0]=0; ctx->headers[0]=0;
    return dgn_make_promise(dgn_net_http_post_task, ctx);
}

static DgnVal dgn_net_http_delete(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    DgnHttpCtx* ctx = (DgnHttpCtx*)malloc(sizeof(DgnHttpCtx));
    strncpy(ctx->url, args[off].s, sizeof(ctx->url)-1);
    strcpy(ctx->method, "DELETE"); ctx->body[0]=0; ctx->headers[0]=0;
    return dgn_make_promise(dgn_net_http_get_task, ctx);
}

static DgnVal dgn_net_socket_connect_task(void* arg) {
    DgnConnectCtx* ctx = (DgnConnectCtx*)arg;
    struct addrinfo hints, *ai;
    memset(&hints,0,sizeof(hints)); hints.ai_socktype=SOCK_STREAM;
    char port_str[8]; snprintf(port_str,sizeof(port_str),"%d",ctx->port);
    if(getaddrinfo(ctx->host, port_str, &hints, &ai) != 0) { free(ctx); return dgn_null(); }
#ifdef _WIN32
    SOCKET fd = socket(ai->ai_family,SOCK_STREAM,0);
    if(connect(fd,ai->ai_addr,(int)ai->ai_addrlen)!=0){freeaddrinfo(ai);closesocket(fd);free(ctx);return dgn_null();}
#else
    int fd = socket(ai->ai_family,SOCK_STREAM,0);
    if(connect(fd,ai->ai_addr,(int)ai->ai_addrlen)!=0){freeaddrinfo(ai);close(fd);free(ctx);return dgn_null();}
#endif
    freeaddrinfo(ai); free(ctx);
    return dgn_socket_make((int64_t)fd);
}
static DgnVal dgn_net_socket_connect(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2) return dgn_null();
    DgnConnectCtx* ctx = (DgnConnectCtx*)malloc(sizeof(DgnConnectCtx));
    char* h = dgn_val_to_cstr(args[off]); strncpy(ctx->host,h,255); free(h);
    ctx->port = (int)((args[off+1].tag==DGN_INT)?args[off+1].i:(int64_t)args[off+1].f);
    return dgn_make_promise(dgn_net_socket_connect_task, ctx);
}

static DgnVal dgn_net_socket_send(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_socket_fd(args[0]) >= 0) ? 0 : 0;
    if(n - off < 2) return dgn_bool(0);
    int64_t fd = dgn_socket_fd(args[off]);
    if(fd < 0) return dgn_bool(0);
    char* msg = dgn_val_to_cstr(args[off+1]);
    int sent = (int)send((int)fd, msg, (int)strlen(msg), 0);
    free(msg);
    return dgn_bool(sent >= 0);
}

static DgnVal dgn_net_socket_read(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 1) return dgn_null();
    int64_t fd = dgn_socket_fd(args[0]);
    if(fd < 0) return dgn_null();
    char buf[8192] = {0};
    int r = (int)recv((int)fd, buf, sizeof(buf)-1, 0);
    if(r <= 0) return dgn_null();
    buf[r] = 0;
    return dgn_str(buf);
}

static DgnVal dgn_net_socket_close(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 1) return dgn_bool(0);
    int64_t fd = dgn_socket_fd(args[0]);
    if(fd < 0) return dgn_bool(0);
#ifdef _WIN32
    closesocket((SOCKET)fd);
#else
    close((int)fd);
#endif
    return dgn_bool(1);
}

static DgnVal dgn_net_start_server(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2) return dgn_bool(0);
    DgnServerCtx* ctx = (DgnServerCtx*)malloc(sizeof(DgnServerCtx));
    ctx->port    = (int)((args[off].tag==DGN_INT)?args[off].i:(int64_t)args[off].f);
    ctx->handler = args[off+1];
    dgn_make_promise(dgn_net_http_server_task, ctx);
    return dgn_bool(1);
}

#if defined(_WIN32)
#  define DGN_OS_NAME "windows"
#  define DGN_ARCH_STR "x86_64"
#elif defined(__APPLE__)
#  define DGN_OS_NAME "macos"
#  define DGN_ARCH_STR "aarch64"
#else
#  define DGN_OS_NAME "linux"
#  define DGN_ARCH_STR "x86_64"
#endif

static DgnVal dgn_os_name(int x) { (void)x; return dgn_str(DGN_OS_NAME); }

static DgnVal dgn_os_name_fn(DgnVal* args, int n, DgnEnv* env) {
    (void)args; (void)n; (void)env;
    return dgn_str(DGN_OS_NAME);
}

static DgnVal dgn_os_arch_fn(DgnVal* args, int n, DgnEnv* env) {
    (void)args; (void)n; (void)env;
    return dgn_str(DGN_ARCH_STR);
}

static DgnVal dgn_os_cwd(DgnVal* args, int n, DgnEnv* env) {
    (void)args; (void)n; (void)env;
    char buf[4096]; buf[0] = 0;
#ifdef _WIN32
    GetCurrentDirectoryA(sizeof(buf), buf);
#else
    if(!getcwd(buf, sizeof(buf))) buf[0] = 0;
#endif
    return dgn_str(buf);
}

static DgnVal dgn_os_chdir(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);
#ifdef _WIN32
    return dgn_bool(SetCurrentDirectoryA(args[off].s) != 0);
#else
    return dgn_bool(chdir(args[off].s) == 0);
#endif
}

static DgnVal dgn_os_getenv(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    const char* v = getenv(args[off].s);
    return v ? dgn_str(v) : dgn_null();
}

static DgnVal dgn_os_setenv(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2 || args[off].tag != DGN_STR || args[off+1].tag != DGN_STR) return dgn_bool(0);
#ifdef _WIN32
    char buf[4096]; snprintf(buf, sizeof(buf), "%s=%s", args[off].s, args[off+1].s);
    return dgn_bool(_putenv(buf) == 0);
#else
    return dgn_bool(setenv(args[off].s, args[off+1].s, 1) == 0);
#endif
}

static DgnVal dgn_os_system(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_int(-1);
    return dgn_int(system(args[off].s));
}

static DgnVal dgn_os_listdir(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    const char* path = (n - off >= 1 && args[off].tag == DGN_STR) ? args[off].s : ".";
    DgnVal list = dgn_list_new(8);
#ifdef _WIN32
    char pattern[4096]; snprintf(pattern, sizeof(pattern), "%s\\*", path);
    WIN32_FIND_DATAA fd; HANDLE h = FindFirstFileA(pattern, &fd);
    if(h != INVALID_HANDLE_VALUE) {
        do { if(strcmp(fd.cFileName,".") && strcmp(fd.cFileName,"..")) dgn_list_push(&list, dgn_str(fd.cFileName)); } while(FindNextFileA(h, &fd));
        FindClose(h);
    }
#else
    DIR* d = opendir(path);
    if(d) {
        struct dirent* e;
        while((e = readdir(d))) { if(strcmp(e->d_name,".") && strcmp(e->d_name,"..")) dgn_list_push(&list, dgn_str(e->d_name)); }
        closedir(d);
    }
#endif
    return list;
}

#ifdef _WIN32
#  define DGN_PATH_SEP '\\'
#else
#  define DGN_PATH_SEP '/'
#endif

static DgnVal dgn_path_join(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    char buf[4096]; buf[0] = 0; int first = 1;
    for(int i = 0; i < n; i++) {
        if(args[i].tag != DGN_STR) continue;
        if(!first && buf[strlen(buf)-1] != DGN_PATH_SEP) { int l=(int)strlen(buf); buf[l]=(char)DGN_PATH_SEP; buf[l+1]=0; }
        strncat(buf, args[i].s, sizeof(buf)-strlen(buf)-1);
        first = 0;
    }
    return dgn_str(buf);
}

static DgnVal dgn_path_basename(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_str("");
    const char* p = args[off].s; int len = (int)strlen(p);
    int i = len - 1;
    while(i > 0 && p[i] != '/' && p[i] != '\\') i--;
    return dgn_str(p[i] == '/' || p[i] == '\\' ? p + i + 1 : p);
}

static DgnVal dgn_path_dirname(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_str(".");
    const char* p = args[off].s; int len = (int)strlen(p);
    int i = len - 1;
    while(i > 0 && p[i] != '/' && p[i] != '\\') i--;
    if(i == 0) return dgn_str(".");
    char buf[4096]; memcpy(buf, p, i); buf[i] = 0;
    return dgn_str(buf);
}

static DgnVal dgn_path_extname(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_str("");
    const char* p = args[off].s;
    const char* dot = strrchr(p, '.');
    return dot ? dgn_str(dot) : dgn_str("");
}

static DgnVal dgn_path_is_absolute(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_bool(0);
    const char* p = args[off].s;
#ifdef _WIN32
    return dgn_bool((p[0] && p[1] == ':') || p[0] == '\\' || p[0] == '/');
#else
    return dgn_bool(p[0] == '/');
#endif
}

static DgnVal dgn_time_now(DgnVal* args, int n, DgnEnv* env) {
    (void)args; (void)n; (void)env;
    return dgn_float((double)time(NULL));
}

static DgnVal dgn_time_sleep_ms(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    int ms = (n - off >= 1) ? (int)((args[off].tag == DGN_FLOAT) ? args[off].f : (double)args[off].i) : 0;
    DGN_SLEEP(ms);
    return dgn_null();
}

static DgnVal dgn_time_date_now(DgnVal* args, int n, DgnEnv* env) {
    (void)args; (void)n; (void)env;
    time_t now = time(NULL);
    struct tm* t = localtime(&now);
    DgnVal m = dgn_map_new();
    dgn_map_set(&m, "year",   dgn_int(t->tm_year + 1900));
    dgn_map_set(&m, "month",  dgn_int(t->tm_mon + 1));
    dgn_map_set(&m, "day",    dgn_int(t->tm_mday));
    dgn_map_set(&m, "hour",   dgn_int(t->tm_hour));
    dgn_map_set(&m, "minute", dgn_int(t->tm_min));
    dgn_map_set(&m, "second", dgn_int(t->tm_sec));
    char buf[32]; strftime(buf, sizeof(buf), "%Y-%m-%d %H:%M:%S", t);
    dgn_map_set(&m, "str", dgn_str(buf));
    return m;
}

static char* dgn_json_encode_val(DgnVal v, int depth);
static char* dgn_json_encode_val(DgnVal v, int depth) {
    if(depth > 20) return strdup("null");
    char buf[256];
    switch(v.tag) {
        case DGN_NULL:  return strdup("null");
        case DGN_BOOL:  return strdup(v.i ? "true" : "false");
        case DGN_INT:   snprintf(buf, sizeof(buf), "%lld", (long long)v.i); return strdup(buf);
        case DGN_FLOAT: snprintf(buf, sizeof(buf), "%.17g", v.f); return strdup(buf);
        case DGN_STR: {
            size_t len = strlen(v.s) * 6 + 3;
            char* out = (char*)malloc(len); out[0] = '"'; int oi = 1;
            for(const char* p = v.s; *p; p++) {
                if(*p == '"')  { out[oi++]='\\'; out[oi++]='"'; }
                else if(*p=='\\') { out[oi++]='\\'; out[oi++]='\\'; }
                else if(*p=='\n') { out[oi++]='\\'; out[oi++]='n'; }
                else if(*p=='\r') { out[oi++]='\\'; out[oi++]='r'; }
                else if(*p=='\t') { out[oi++]='\\'; out[oi++]='t'; }
                else out[oi++] = *p;
            }
            out[oi++] = '"'; out[oi] = 0; return out;
        }
        case DGN_LIST: {
            size_t cap = 1024, len = 0; char* out = (char*)malloc(cap); out[len++] = '['; int first = 1;
            for(int i = 0; i < v.list->len; i++) {
                if(!first) { if(len + 2 > cap) { cap *= 2; out = (char*)realloc(out, cap); } out[len++] = ','; } first = 0;
                char* s = dgn_json_encode_val(v.list->data[i], depth+1);
                size_t sl = strlen(s);
                while(len + sl + 2 > cap) cap *= 2;
                out = (char*)realloc(out, cap);
                memcpy(out + len, s, sl); len += sl; free(s);
            }
            out[len++] = ']'; out[len] = 0; return out;
        }
        case DGN_MAP: case DGN_INSTANCE: {
            size_t cap = 1024, len = 0; char* out = (char*)malloc(cap); out[len++] = '{'; int first = 1;
            for(int i = 0; i < v.map->cap; i++) {
                if(!v.map->entries[i].used) continue;
                if(!first) { if(len + 2 > cap) { cap *= 2; out = (char*)realloc(out, cap); } out[len++] = ','; } first = 0;
                const char* key = v.map->entries[i].key;
                size_t kl = strlen(key);
                while(len + kl + 8 > cap) cap *= 2;
                out = (char*)realloc(out, cap);
                out[len++] = '"'; memcpy(out + len, key, kl); len += kl; out[len++] = '"'; out[len++] = ':';
                char* s = dgn_json_encode_val(v.map->entries[i].val, depth+1);
                size_t sl = strlen(s);
                while(len + sl + 2 > cap) cap *= 2;
                out = (char*)realloc(out, cap);
                memcpy(out + len, s, sl); len += sl; free(s);
            }
            out[len++] = '}'; out[len] = 0; return out;
        }
        default: return strdup("null");
    }
}

static DgnVal dgn_json_encode(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1) return dgn_str("null");
    char* s = dgn_json_encode_val(args[off], 0);
    DgnVal r = dgn_str(s); free(s); return r;
}

static DgnVal dgn_json_parse_val(const char** p);
static void dgn_json_skip_ws(const char** p) { while(**p && **p <= ' ') (*p)++; }
static DgnVal dgn_json_parse_val(const char** p) {
    dgn_json_skip_ws(p);
    char c = **p;

    if(c == '"') {
        (*p)++;
        int cap = 256; char* buf = (char*)malloc(cap); int bi = 0;
        while(**p && **p != '"') {
            if(bi >= cap - 4) { cap *= 2; buf = (char*)realloc(buf, cap); }
            if(**p == '\\') {
                (*p)++;
                switch(**p) {
                    case 'n': buf[bi++]='\n'; break; case 't': buf[bi++]='\t'; break;
                    case 'r': buf[bi++]='\r'; break; case '"': buf[bi++]='"';  break;
                    case '\\':buf[bi++]='\\'; break;
                    case 'b': buf[bi++]='\b'; break; case 'f': buf[bi++]='\f'; break;
                    case 'u': {
                        unsigned cp = 0; int ok = 1;
                        for(int k = 1; k <= 4; k++) {
                            char h = (*p)[k];
                            if(h >= '0' && h <= '9') cp = cp*16 + (h-'0');
                            else if(h >= 'a' && h <= 'f') cp = cp*16 + (h-'a'+10);
                            else if(h >= 'A' && h <= 'F') cp = cp*16 + (h-'A'+10);
                            else { ok = 0; break; }
                        }
                        if(!ok) { buf[bi++]='u'; break; }
                        *p += 4;
                        if(cp >= 0xD800 && cp <= 0xDBFF && (*p)[1] == '\\' && (*p)[2] == 'u') {
                            unsigned lo = 0; int ok2 = 1;
                            for(int k = 3; k <= 6; k++) {
                                char h = (*p)[k];
                                if(h >= '0' && h <= '9') lo = lo*16 + (h-'0');
                                else if(h >= 'a' && h <= 'f') lo = lo*16 + (h-'a'+10);
                                else if(h >= 'A' && h <= 'F') lo = lo*16 + (h-'A'+10);
                                else { ok2 = 0; break; }
                            }
                            if(ok2 && lo >= 0xDC00 && lo <= 0xDFFF) { cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00); *p += 6; }
                        }
                        if(cp < 0x80) buf[bi++] = (char)cp;
                        else if(cp < 0x800) { buf[bi++] = (char)(0xC0 | (cp >> 6)); buf[bi++] = (char)(0x80 | (cp & 0x3F)); }
                        else if(cp < 0x10000) { buf[bi++] = (char)(0xE0 | (cp >> 12)); buf[bi++] = (char)(0x80 | ((cp >> 6) & 0x3F)); buf[bi++] = (char)(0x80 | (cp & 0x3F)); }
                        else { buf[bi++] = (char)(0xF0 | (cp >> 18)); buf[bi++] = (char)(0x80 | ((cp >> 12) & 0x3F)); buf[bi++] = (char)(0x80 | ((cp >> 6) & 0x3F)); buf[bi++] = (char)(0x80 | (cp & 0x3F)); }
                        break;
                    }
                    default:  buf[bi++]=**p; break;
                }
            } else { buf[bi++] = **p; }
            (*p)++;
        }
        if(**p == '"') (*p)++;
        buf[bi] = 0;
        DgnVal r = dgn_str(buf); free(buf); return r;
    }

    if(c == '{') {
        (*p)++; DgnVal m = dgn_map_new();
        dgn_json_skip_ws(p);
        if(**p == '}') { (*p)++; return m; }
        while(**p) {
            dgn_json_skip_ws(p);
            if(**p != '"') break;
            DgnVal key = dgn_json_parse_val(p);
            dgn_json_skip_ws(p); if(**p == ':') (*p)++;
            DgnVal val = dgn_json_parse_val(p);
            if(key.tag == DGN_STR) dgn_map_set(&m, key.s, val);
            dgn_json_skip_ws(p);
            if(**p == ',') (*p)++; else break;
        }
        dgn_json_skip_ws(p); if(**p == '}') (*p)++;
        return m;
    }

    if(c == '[') {
        (*p)++; DgnVal list = dgn_list_new(4);
        dgn_json_skip_ws(p);
        if(**p == ']') { (*p)++; return list; }
        while(**p) {
            dgn_list_push(&list, dgn_json_parse_val(p));
            dgn_json_skip_ws(p);
            if(**p == ',') (*p)++; else break;
        }
        dgn_json_skip_ws(p); if(**p == ']') (*p)++;
        return list;
    }

    if(strncmp(*p, "true",  4)==0) { (*p)+=4; return dgn_bool(1); }
    if(strncmp(*p, "false", 5)==0) { (*p)+=5; return dgn_bool(0); }
    if(strncmp(*p, "null",  4)==0) { (*p)+=4; return dgn_null(); }

    char nbuf[64]; int ni = 0; int is_float = 0;
    if(**p == '-') { nbuf[ni++] = **p; (*p)++; }
    while(**p && (isdigit((unsigned char)**p) || **p=='.' || **p=='e' || **p=='E' || **p=='+' || **p=='-')) {
        if(**p=='.' || **p=='e' || **p=='E') is_float = 1;
        if(ni < 62) nbuf[ni++] = **p; (*p)++;
    }
    nbuf[ni] = 0;
    if(ni == 0) { (*p)++; return dgn_null(); }
    return is_float ? dgn_float(atof(nbuf)) : dgn_int(atoll(nbuf));
}

static DgnVal dgn_json_decode(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    const char* p = args[off].s;
    return dgn_json_parse_val(&p);
}

static const char _b64chars[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
static DgnVal dgn_b64_encode(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_str("");
    const unsigned char* in = (const unsigned char*)args[off].s;
    int inlen = (int)strlen(args[off].s);
    int outlen = ((inlen + 2) / 3) * 4 + 1;
    char* out = (char*)malloc(outlen); int oi = 0;
    for(int i = 0; i < inlen; i += 3) {
        unsigned int b = (in[i] << 16) | (i+1 < inlen ? in[i+1] << 8 : 0) | (i+2 < inlen ? in[i+2] : 0);
        out[oi++] = _b64chars[(b>>18)&63]; out[oi++] = _b64chars[(b>>12)&63];
        out[oi++] = (i+1 < inlen) ? _b64chars[(b>>6)&63] : '=';
        out[oi++] = (i+2 < inlen) ? _b64chars[b&63] : '=';
    }
    out[oi] = 0;
    DgnVal r = dgn_str(out); free(out); return r;
}

static DgnVal dgn_b64_decode(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_str("");
    const char* in = args[off].s; int inlen = (int)strlen(in);
    char* out = (char*)malloc(inlen + 1); int oi = 0;
    static const int dec[256] = {
        ['A']=0,['B']=1,['C']=2,['D']=3,['E']=4,['F']=5,['G']=6,['H']=7,
        ['I']=8,['J']=9,['K']=10,['L']=11,['M']=12,['N']=13,['O']=14,['P']=15,
        ['Q']=16,['R']=17,['S']=18,['T']=19,['U']=20,['V']=21,['W']=22,['X']=23,
        ['Y']=24,['Z']=25,['a']=26,['b']=27,['c']=28,['d']=29,['e']=30,['f']=31,
        ['g']=32,['h']=33,['i']=34,['j']=35,['k']=36,['l']=37,['m']=38,['n']=39,
        ['o']=40,['p']=41,['q']=42,['r']=43,['s']=44,['t']=45,['u']=46,['v']=47,
        ['w']=48,['x']=49,['y']=50,['z']=51,['0']=52,['1']=53,['2']=54,['3']=55,
        ['4']=56,['5']=57,['6']=58,['7']=59,['8']=60,['9']=61,['+']=62,['/']=63
    };
    for(int i = 0; i+3 < inlen; i += 4) {
        unsigned int b = (dec[(unsigned char)in[i]]<<18)|(dec[(unsigned char)in[i+1]]<<12)|
                         (in[i+2]!='='?dec[(unsigned char)in[i+2]]<<6:0)|
                         (in[i+3]!='='?dec[(unsigned char)in[i+3]]:0);
        out[oi++]=(char)(b>>16); if(in[i+2]!='=') out[oi++]=(char)(b>>8); if(in[i+3]!='=') out[oi++]=(char)b;
    }
    out[oi] = 0;
    DgnVal r = dgn_str(out); free(out); return r;
}

static DgnVal dgn_uuid_v4(DgnVal* args, int n, DgnEnv* env) {
    (void)args; (void)n; (void)env;
    unsigned int u1 = (unsigned int)rand() ^ ((unsigned int)rand() << 15);
    unsigned int u2 = (unsigned int)rand() ^ ((unsigned int)rand() << 15);
    unsigned int u3 = (unsigned int)rand() ^ ((unsigned int)rand() << 15);
    unsigned int u4 = (unsigned int)rand() ^ ((unsigned int)rand() << 15);
    char buf[37];
    snprintf(buf, sizeof(buf), "%08x-%04x-4%03x-%04x-%08x%04x",
        u1, (u2>>16)&0xFFFF, u2&0xFFF,
        ((u3>>16)&0x3FFF)|0x8000,
        u3&0xFFFF, u4&0xFFFF);
    snprintf(buf, sizeof(buf), "%08x-%04x-4%03x-%04x-%04x%08x",
        u1, (u2>>16)&0xFFFF, u2&0xFFF,
        ((u3>>16)&0x3FFF)|0x8000,
        u3&0xFFFF, u4);
    return dgn_str(buf);
}

typedef struct {
    DgnVal (*fn)(void* ctx);
    void*  ctx;
    DgnVal result;
    int    done;
#ifdef _WIN32
    HANDLE thread;
#else
    pthread_t thread;
#endif
} DgnTask;

#ifdef _WIN32
static DWORD WINAPI dgn_task_run(LPVOID arg) {
    DgnTask* t = (DgnTask*)arg;
    t->result = t->fn(t->ctx);
    t->done = 1;
    return 0;
}
#else
static void* dgn_task_run(void* arg) {
    DgnTask* t = (DgnTask*)arg;
    t->result = t->fn(t->ctx);
    t->done = 1;
    return NULL;
}
#endif

static DgnVal dgn_make_promise(DgnVal (*fn)(void*), void* ctx) {
    DgnTask* task = (DgnTask*)malloc(sizeof(DgnTask));
    task->fn     = fn;
    task->ctx    = ctx;
    task->result = dgn_null();
    task->done   = 0;

#ifdef _WIN32
    task->thread = CreateThread(NULL, 0, dgn_task_run, task, 0, NULL);
#else
    pthread_create(&task->thread, NULL, dgn_task_run, task);
    pthread_detach(task->thread);
#endif

    DgnVal promise = dgn_map_new();
    dgn_map_set(&promise, "__promise__", dgn_int(1));
    dgn_map_set(&promise, "__task_ptr__", dgn_int((int64_t)(uintptr_t)task));
    return promise;
}

static DgnVal dgn_promise_await(DgnVal v) {
    if(v.tag != DGN_MAP) return v;
    DgnVal sentinel = dgn_map_get(v, "__promise__");
    if(sentinel.tag != DGN_INT || sentinel.i != 1) return v;
    DgnVal ptr_val = dgn_map_get(v, "__task_ptr__");
    if(ptr_val.tag != DGN_INT) return dgn_null();
    DgnTask* task = (DgnTask*)(uintptr_t)(uint64_t)ptr_val.i;
    while(!task->done) { DGN_SLEEP(1); }
#ifdef _WIN32
    if(task->thread) { WaitForSingleObject(task->thread, INFINITE); CloseHandle(task->thread); }
#endif
    DgnVal result = task->result;
    free(task);
    return result;
}

static DgnVal dgn_socket_make(int64_t fd) {
    DgnVal m = dgn_map_new();
    dgn_map_set(&m, "__socket__", dgn_int(fd));
    return m;
}
static int64_t dgn_socket_fd(DgnVal v) {
    if(v.tag != DGN_MAP) return -1;
    DgnVal s = dgn_map_get(v, "__socket__");
    return (s.tag == DGN_INT) ? s.i : -1;
}

static DgnVal dgn_net_do_http(void* arg) {
    DgnHttpCtx* ctx = (DgnHttpCtx*)arg;
    DgnVal res = dgn_map_new();

    const char* url = ctx->url;
    int use_port = 80;
    char host[256] = {0}; char path[2048] = "/";
    const char* p = url;
    if(strncmp(p, "http://",  7)==0) p += 7;
    else if(strncmp(p, "https://", 8)==0) { p += 8; use_port = 443; }
    int hi = 0;
    while(*p && *p != '/' && *p != ':' && hi < 255) host[hi++] = *p++;
    host[hi] = 0;
    if(*p == ':') { p++; use_port = atoi(p); while(*p && *p != '/') p++; }
    if(*p == '/') { strncpy(path, p, sizeof(path)-1); }

    struct addrinfo hints, *ai;
    memset(&hints, 0, sizeof(hints));
    hints.ai_socktype = SOCK_STREAM;
    char port_str[8]; snprintf(port_str, sizeof(port_str), "%d", use_port);
    if(getaddrinfo(host, port_str, &hints, &ai) != 0) {
        dgn_map_set(&res, "status", dgn_int(-1));
        dgn_map_set(&res, "body",   dgn_str("DNS resolution failed"));
        free(ctx); return res;
    }
#ifdef _WIN32
    SOCKET fd = socket(ai->ai_family, SOCK_STREAM, 0);
#else
    int fd = socket(ai->ai_family, SOCK_STREAM, 0);
#endif
    if(connect(fd, ai->ai_addr, (int)ai->ai_addrlen) != 0) {
        freeaddrinfo(ai);
        dgn_map_set(&res, "status", dgn_int(-1));
        dgn_map_set(&res, "body",   dgn_str("Connection failed"));
        free(ctx); return res;
    }
    freeaddrinfo(ai);

    char req[8192];
    int rlen = snprintf(req, sizeof(req),
        "%s %s HTTP/1.0\r\nHost: %s\r\nConnection: close\r\n",
        ctx->method, path, host);
    if(ctx->headers[0]) {
        strncat(req, ctx->headers, sizeof(req)-rlen-1);
        rlen = (int)strlen(req);
    }
    if(strcmp(ctx->method, "POST")==0 || strcmp(ctx->method, "PUT")==0) {
        int blen = (int)strlen(ctx->body);
        rlen += snprintf(req+rlen, sizeof(req)-rlen,
            "Content-Length: %d\r\n\r\n", blen);
        strncat(req, ctx->body, sizeof(req)-rlen-1);
    } else {
        strncat(req, "\r\n", sizeof(req)-strlen(req)-1);
    }
    send(fd, req, (int)strlen(req), 0);

    char resp_buf[65536]; int resp_len = 0; int n;
    while((n = (int)recv(fd, resp_buf+resp_len, sizeof(resp_buf)-resp_len-1, 0)) > 0) resp_len += n;
    resp_buf[resp_len] = 0;
#ifdef _WIN32
    closesocket(fd);
#else
    close(fd);
#endif

    int status_code = 0;
    if(strncmp(resp_buf, "HTTP/", 5)==0) {
        const char* sp = strchr(resp_buf, ' ');
        if(sp) status_code = atoi(sp+1);
    }
    const char* body_start = strstr(resp_buf, "\r\n\r\n");
    const char* body = body_start ? body_start + 4 : resp_buf;

    dgn_map_set(&res, "status", dgn_int(status_code));
    dgn_map_set(&res, "body",   dgn_str(body));
    free(ctx);
    return res;
}

#ifndef _WIN32
#  include <regex.h>
#  define DGN_HAS_POSIX_REGEX 1
#endif

#ifdef _WIN32
typedef struct { char pattern[1024]; int flags; } DgnRegexImpl;
#else
typedef struct { regex_t re; char pattern[1024]; } DgnRegexImpl;
#endif
static DgnVal dgn_regex_test(DgnVal* args, int n, DgnEnv* env);
static DgnVal dgn_regex_match_fn(DgnVal* args, int n, DgnEnv* env);
static DgnVal dgn_regex_find_all(DgnVal* args, int n, DgnEnv* env);
static DgnVal dgn_regex_replace(DgnVal* args, int n, DgnEnv* env);
static DgnVal dgn_regex_groups(DgnVal* args, int n, DgnEnv* env);
static DgnVal dgn_regex_new(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "__regex__").tag == DGN_NULL) ? 0 : 0;
    if(n < 1 || args[off].tag != DGN_STR) return dgn_null();
    const char* pat = args[off].s;

    DgnRegexImpl* impl = (DgnRegexImpl*)malloc(sizeof(DgnRegexImpl));
    strncpy(impl->pattern, pat, sizeof(impl->pattern)-1);
#ifdef DGN_HAS_POSIX_REGEX
    if(regcomp(&impl->re, pat, REG_EXTENDED) != 0) {
        free(impl);
        return dgn_null();
    }
#endif

    DgnVal m = dgn_map_new();
    dgn_map_set(&m, "__regex__",   dgn_int(1));
    dgn_map_set(&m, "__pattern__", dgn_str(pat));
    dgn_map_set(&m, "__ptr__",     dgn_int((int64_t)(uintptr_t)impl));

    dgn_map_set(&m, "test",     dgn_func(dgn_regex_test,     2));
    dgn_map_set(&m, "match",    dgn_func(dgn_regex_match_fn, 2));
    dgn_map_set(&m, "find_all", dgn_func(dgn_regex_find_all, 2));
    dgn_map_set(&m, "replace",  dgn_func(dgn_regex_replace,  3));
    dgn_map_set(&m, "groups",   dgn_func(dgn_regex_groups,   2));

    return m;
}

static DgnRegexImpl* dgn_regex_get(DgnVal v) {
    if(v.tag != DGN_MAP) return NULL;
    DgnVal sentinel = dgn_map_get(v, "__regex__");
    if(sentinel.tag != DGN_INT || sentinel.i != 1) return NULL;
    DgnVal ptr = dgn_map_get(v, "__ptr__");
    if(ptr.tag != DGN_INT) return NULL;
    return (DgnRegexImpl*)(uintptr_t)(uint64_t)ptr.i;
}

static DgnVal dgn_regex_test(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0],"__regex__").i==1) ? 0 : 0;
    if(n - off < 2) return dgn_bool(0);
    DgnRegexImpl* impl = dgn_regex_get(args[off]);
    if(!impl) return dgn_bool(0);
    char* s = dgn_val_to_cstr(args[off+1]);
#ifdef DGN_HAS_POSIX_REGEX
    int r = regexec(&impl->re, s, 0, NULL, 0);
    free(s);
    return dgn_bool(r == 0);
#else
    int r = strstr(s, impl->pattern) != NULL;
    free(s); return dgn_bool(r);
#endif
}

static DgnVal dgn_regex_match_fn(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2) return dgn_null();
    DgnRegexImpl* impl = dgn_regex_get(args[0]);
    if(!impl) return dgn_null();
    char* s = dgn_val_to_cstr(args[1]);
#ifdef DGN_HAS_POSIX_REGEX
    regmatch_t m;
    if(regexec(&impl->re, s, 1, &m, 0) == 0) {
        int len = m.rm_eo - m.rm_so;
        char* buf = (char*)malloc(len+1); memcpy(buf, s+m.rm_so, len); buf[len]=0;
        DgnVal r = dgn_str(buf); free(buf); free(s); return r;
    }
    free(s); return dgn_null();
#else
    const char* found = strstr(s, impl->pattern);
    if(!found) { free(s); return dgn_null(); }
    DgnVal r = dgn_str(impl->pattern); free(s); return r;
#endif
}

static DgnVal dgn_regex_find_all(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2) return dgn_list_new(0);
    DgnRegexImpl* impl = dgn_regex_get(args[0]);
    if(!impl) return dgn_list_new(0);
    char* s = dgn_val_to_cstr(args[1]);
    DgnVal list = dgn_list_new(4);
#ifdef DGN_HAS_POSIX_REGEX
    const char* cursor = s;
    regmatch_t m;
    while(*cursor && regexec(&impl->re, cursor, 1, &m, 0) == 0) {
        if(m.rm_eo == m.rm_so) { cursor++; continue; }
        int len = m.rm_eo - m.rm_so;
        char* buf = (char*)malloc(len+1); memcpy(buf, cursor+m.rm_so, len); buf[len]=0;
        dgn_list_push(&list, dgn_str(buf)); free(buf);
        cursor += m.rm_eo;
    }
#endif
    free(s); return list;
}

static DgnVal dgn_regex_replace(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 3) return (n >= 2) ? dgn_str(args[1].tag==DGN_STR ? args[1].s : "") : dgn_null();
    DgnRegexImpl* impl = dgn_regex_get(args[0]);
    char* s    = dgn_val_to_cstr(args[1]);
    char* repl = dgn_val_to_cstr(args[2]);
    if(!impl) { DgnVal r = dgn_str(s); free(s); free(repl); return r; }
#ifdef DGN_HAS_POSIX_REGEX
    char out[65536]; out[0]=0; int oi=0;
    const char* cursor = s;
    regmatch_t m;
    while(*cursor && regexec(&impl->re, cursor, 1, &m, 0) == 0) {
        if(m.rm_eo == m.rm_so) { if(oi < 65534) out[oi++]=*cursor++; continue; }
        int plen = m.rm_so;
        if(oi+plen < 65530) { memcpy(out+oi, cursor, plen); oi+=plen; }
        int rlen = (int)strlen(repl);
        if(oi+rlen < 65530) { memcpy(out+oi, repl, rlen); oi+=rlen; }
        cursor += m.rm_eo;
    }
    int rem = (int)strlen(cursor);
    if(oi+rem < 65530) { memcpy(out+oi, cursor, rem); oi+=rem; }
    out[oi]=0;
    DgnVal r = dgn_str(out); free(s); free(repl); return r;
#else
    DgnVal r = dgn_str(s); free(s); free(repl); return r;
#endif
}

static DgnVal dgn_regex_groups(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2) return dgn_null();
    DgnRegexImpl* impl = dgn_regex_get(args[0]);
    if(!impl) return dgn_null();
    char* s = dgn_val_to_cstr(args[1]);
#ifdef DGN_HAS_POSIX_REGEX
    regmatch_t matches[16];
    if(regexec(&impl->re, s, 16, matches, 0) == 0) {
        DgnVal list = dgn_list_new(4);
        for(int i = 0; i < 16 && matches[i].rm_so != -1; i++) {
            int len = matches[i].rm_eo - matches[i].rm_so;
            char* buf = (char*)malloc(len+1); memcpy(buf, s+matches[i].rm_so, len); buf[len]=0;
            dgn_list_push(&list, dgn_str(buf)); free(buf);
        }
        free(s); return list;
    }
#endif
    free(s); return dgn_null();
}

static DgnVal dgn_net_res_send_fn(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    if(n < 2) return dgn_null();
    DgnVal res_map = args[0];
    char* body   = dgn_val_to_cstr(args[1]);
    int   status = (n >= 3 && args[2].tag == DGN_INT) ? (int)args[2].i : 200;
    const char* status_text = (status == 200) ? "OK" :
                              (status == 404) ? "Not Found" :
                              (status == 500) ? "Internal Server Error" : "OK";

    char hdrs[2048] = {0};
    if(n >= 4 && args[3].tag == DGN_MAP) {
        for(int i = 0; i < args[3].map->cap; i++) {
            if(args[3].map->entries[i].used) {
                char* v = dgn_val_to_cstr(args[3].map->entries[i].val);
                char tmp[512];
                snprintf(tmp, sizeof(tmp), "%s: %s\r\n", args[3].map->entries[i].key, v);
                strncat(hdrs, tmp, sizeof(hdrs)-strlen(hdrs)-1);
                free(v);
            }
        }
    }

    char response[65536+512];
    int rlen = snprintf(response, sizeof(response),
        "HTTP/1.0 %d %s\r\n"
        "Content-Length: %d\r\n"
        "Content-Type: text/plain\r\n"
        "%s"
        "\r\n"
        "%s",
        status, status_text, (int)strlen(body), hdrs, body);
    free(body);

    DgnVal fd_val = dgn_map_get(res_map, "__client__");
    if(fd_val.tag == DGN_INT) {
#ifdef _WIN32
        SOCKET fd = (SOCKET)(uintptr_t)(uint64_t)fd_val.i;
        send(fd, response, rlen, 0);
#else
        int fd = (int)fd_val.i;
        write(fd, response, rlen);
#endif
    }
    return dgn_null();
}

static DgnVal dgn_net_http_server_task(void* arg) {
    DgnServerCtx* ctx = (DgnServerCtx*)arg;
#ifdef _WIN32
    SOCKET srv = socket(AF_INET, SOCK_STREAM, 0);
    int opt = 1; setsockopt(srv, SOL_SOCKET, SO_REUSEADDR, (char*)&opt, sizeof(opt));
    struct sockaddr_in addr; memset(&addr,0,sizeof(addr));
    addr.sin_family=AF_INET; addr.sin_addr.s_addr=INADDR_ANY; addr.sin_port=htons((u_short)ctx->port);
    bind(srv,(struct sockaddr*)&addr,sizeof(addr)); listen(srv,16);
    while(1) {
        SOCKET client = accept(srv,NULL,NULL);
        if(client==INVALID_SOCKET) continue;
        char buf[8192]={0}; recv(client,buf,sizeof(buf)-1,0);
        char method[16]={0},path[1024]={0};
        sscanf(buf,"%15s %1023s",method,path);
        const char* body = strstr(buf,"\r\n\r\n"); body = body ? body+4 : "";
        DgnVal req = dgn_map_new();
        dgn_map_set(&req,"method",dgn_str(method));
        dgn_map_set(&req,"path",  dgn_str(path));
        dgn_map_set(&req,"body",  dgn_str(body));
        DgnVal res = dgn_map_new();
        dgn_map_set(&res,"__client__",dgn_int((int64_t)(uintptr_t)(void*)(intptr_t)client));
        dgn_map_set(&res, "send", dgn_func(dgn_net_res_send_fn, -1));
        DgnVal call_args[2] = {req, res};
        dgn_call_val(ctx->handler, call_args, 2);
        closesocket(client);
    }
#else
    int srv = socket(AF_INET,SOCK_STREAM,0);
    int opt=1; setsockopt(srv,SOL_SOCKET,SO_REUSEADDR,&opt,sizeof(opt));
    struct sockaddr_in addr; memset(&addr,0,sizeof(addr));
    addr.sin_family=AF_INET; addr.sin_addr.s_addr=INADDR_ANY; addr.sin_port=htons((uint16_t)ctx->port);
    bind(srv,(struct sockaddr*)&addr,sizeof(addr)); listen(srv,16);
    while(1) {
        int client = accept(srv,NULL,NULL); if(client<0) continue;
        char buf[8192]={0}; read(client,buf,sizeof(buf)-1);
        char method[16]={0},path[1024]={0};
        sscanf(buf,"%15s %1023s",method,path);
        const char* body=strstr(buf,"\r\n\r\n"); body=body?body+4:"";
        DgnVal req=dgn_map_new();
        dgn_map_set(&req,"method",dgn_str(method));
        dgn_map_set(&req,"path",  dgn_str(path));
        dgn_map_set(&req,"body",  dgn_str(body));
        DgnVal res=dgn_map_new();
        dgn_map_set(&res,"__client__",dgn_int((int64_t)client));
        DgnVal call_args[2]={req,res};
        dgn_call_val(ctx->handler,call_args,2);
        close(client);
    }
#endif
    return dgn_null();
}

static DgnVal dgn_builtin_typeof(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1) return dgn_str("null");
    switch(args[0].tag) {
        case DGN_NULL: return dgn_str("null"); case DGN_BOOL: return dgn_str("bool");
        case DGN_INT: return dgn_str("int"); case DGN_FLOAT: return dgn_str("number");
        case DGN_STR: return dgn_str("string"); case DGN_LIST: return dgn_str("list");
        case DGN_MAP: return dgn_str("map"); case DGN_FUNC: return dgn_str("function");
        case DGN_CLASS: return dgn_str("class"); case DGN_INSTANCE: return dgn_str("instance");
        default: return dgn_str("unknown");
    }
}
static DgnVal dgn_builtin_int(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1) return dgn_null();
    if(args[0].tag == DGN_INT) return args[0];
    if(args[0].tag == DGN_FLOAT) return dgn_int((int64_t)args[0].f);
    if(args[0].tag == DGN_STR) return dgn_int(strtoll(args[0].s, NULL, 10));
    if(args[0].tag == DGN_BOOL) return dgn_int(args[0].i);
    return dgn_null();
}
static DgnVal dgn_builtin_float(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1) return dgn_null();
    if(args[0].tag == DGN_FLOAT) return args[0];
    if(args[0].tag == DGN_INT) return dgn_float((double)args[0].i);
    if(args[0].tag == DGN_STR) return dgn_float(strtod(args[0].s, NULL));
    return dgn_null();
}
static DgnVal dgn_builtin_str(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1) return dgn_str("null");
    char buf[256];
    switch(args[0].tag) {
        case DGN_NULL: return dgn_str("null");
        case DGN_BOOL: return dgn_str(args[0].i ? "True" : "False");
        case DGN_INT: snprintf(buf, sizeof(buf), "%lld", (long long)args[0].i); return dgn_str(buf);
        case DGN_FLOAT: snprintf(buf, sizeof(buf), "%g", args[0].f); return dgn_str(buf);
        case DGN_STR: return args[0];
        default: return dgn_str("<object>");
    }
}
static DgnVal dgn_builtin_chr(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1) return dgn_null();
    int code = (int)(args[0].tag == DGN_INT ? args[0].i : args[0].f);
    char buf[2] = { (char)code, 0 }; return dgn_str(buf);
}
static DgnVal dgn_builtin_ord(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag != DGN_STR || !args[0].s[0]) return dgn_null();
    return dgn_int((int64_t)(uint8_t)args[0].s[0]);
}
static DgnVal dgn_builtin_parse_int(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag != DGN_STR) return dgn_null();
    return dgn_int(strtoll(args[0].s, NULL, 10));
}
static DgnVal dgn_builtin_parse_float(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag != DGN_STR) return dgn_null();
    return dgn_float(strtod(args[0].s, NULL));
}
static DgnVal dgn_builtin_format(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1) return dgn_str(""); if(args[0].tag != DGN_STR) return args[0];
    const char* fmt = args[0].s; char out[8192]; out[0] = 0;
    int out_idx = 0; int arg_idx = 1;
    for(int i=0; fmt[i]; i++) {
        if(fmt[i] == '{' && fmt[i+1] == '}') {
            if(arg_idx < n) {
                DgnVal s_val = dgn_builtin_str(&args[arg_idx++], 1, e);
                int len = strlen(s_val.s);
                if(out_idx + len < 8190) { strcpy(out + out_idx, s_val.s); out_idx += len; }
            }
            i++;
        } else { if(out_idx < 8190) out[out_idx++] = fmt[i]; }
    }
    out[out_idx] = 0; return dgn_str(out);
}

#define DGN_MEM_OFF(a) ((intptr_t)((a).tag == DGN_FLOAT ? (int64_t)(a).f : (a).i))

static DgnVal dgn_mem_alloc(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    int size = (n - ao > 0 && args[ao].tag == DGN_INT) ? (int)args[ao].i :
               (n - ao > 0 && args[ao].tag == DGN_FLOAT) ? (int)args[ao].f : 0;
    if (size <= 0) size = 1;

    char* buf = (char*)malloc(size + 1);
    memset(buf, 0, size + 1);

    DgnVal v; v.tag = DGN_STR; v.s = buf;
    return v;
}

static DgnVal dgn_mem_addr(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 1) return dgn_null();
    if (args[ao].tag == DGN_LIST) return dgn_int((int64_t)(uintptr_t)args[ao].list->data);
    if (args[ao].tag == DGN_STR)  return dgn_int((int64_t)(uintptr_t)args[ao].s);
    return dgn_null();
}

static DgnVal dgn_mem_read_ptr(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 2) return dgn_null();
    uintptr_t ptr = (uintptr_t)args[ao].i;
    intptr_t  off = DGN_MEM_OFF(args[ao + 1]);
    if (!ptr) return dgn_null();
    return dgn_int((int64_t)*(uintptr_t*)(ptr + off));
}

static DgnVal dgn_mem_read_float(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 2) return dgn_null();
    uintptr_t ptr = (uintptr_t)args[ao].i;
    intptr_t  off = DGN_MEM_OFF(args[ao + 1]);
    if (!ptr) return dgn_null();
    return dgn_float((double)*(float*)(ptr + off));
}

static DgnVal dgn_mem_read_mem_int(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 2) return dgn_null();
    uintptr_t ptr = (uintptr_t)args[ao].i;
    intptr_t  off = DGN_MEM_OFF(args[ao + 1]);
    if (!ptr) return dgn_null();
    return dgn_int((int64_t)*(uint32_t*)(ptr + off));
}

static DgnVal dgn_mem_read_string(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 1) return dgn_null();
    uintptr_t ptr = (uintptr_t)args[ao].i;
    if (!ptr) return dgn_null();
    return dgn_str((const char*)ptr);
}

static DgnVal dgn_mem_read_int(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 2) return dgn_null();
    int off = (int)DGN_MEM_OFF(args[ao + 1]);
    if (args[ao].tag == DGN_STR) {
        const unsigned char* buf = (const unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        uint32_t val = buf[off]
                     | ((uint32_t)buf[off+1] << 8)
                     | ((uint32_t)buf[off+2] << 16)
                     | ((uint32_t)buf[off+3] << 24);
        return dgn_float((double)val);
    }
    if (args[ao].tag == DGN_LIST) {
        DgnList* l = args[ao].list;
        if (off >= 0 && off + 4 <= l->len) {
            uint32_t val = (uint8_t)l->data[off].i
                         | ((uint8_t)l->data[off+1].i << 8)
                         | ((uint8_t)l->data[off+2].i << 16)
                         | ((uint8_t)l->data[off+3].i << 24);
            return dgn_float((double)val);
        }
    }
    return dgn_null();
}

static DgnVal dgn_mem_write_int(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 3) return dgn_null();
    int off = (int)DGN_MEM_OFF(args[ao + 1]);
    uint32_t val = (uint32_t)(args[ao + 2].tag == DGN_FLOAT ? (int64_t)args[ao + 2].f : args[ao + 2].i);
    if (args[ao].tag == DGN_STR) {
        unsigned char* buf = (unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        buf[off]   = val & 0xFF;
        buf[off+1] = (val >> 8)  & 0xFF;
        buf[off+2] = (val >> 16) & 0xFF;
        buf[off+3] = (val >> 24) & 0xFF;
        return dgn_null();
    }
    if (args[ao].tag == DGN_LIST) {
        DgnList* l = args[ao].list;
        if (off >= 0 && off + 4 <= l->len) {
            l->data[off]   = dgn_int(val & 0xFF);
            l->data[off+1] = dgn_int((val >> 8)  & 0xFF);
            l->data[off+2] = dgn_int((val >> 16) & 0xFF);
            l->data[off+3] = dgn_int((val >> 24) & 0xFF);
        }
    }
    return dgn_null();
}

static DgnVal dgn_mem_read_byte(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 2) return dgn_null();
    int off = (int)DGN_MEM_OFF(args[ao + 1]);
    if (args[ao].tag == DGN_STR) {
        const unsigned char* buf = (const unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        return dgn_int((int64_t)buf[off]);
    }
    if (args[ao].tag == DGN_LIST) {
        if (off >= 0 && off < args[ao].list->len) return args[ao].list->data[off];
    }
    return dgn_null();
}

static DgnVal dgn_mem_write_byte(DgnVal* args, int n, DgnEnv* e) {
    (void)e;
    int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - ao < 3) return dgn_null();
    int off = (int)DGN_MEM_OFF(args[ao + 1]);
    if (args[ao].tag == DGN_STR) {
        unsigned char* buf = (unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        int64_t val = args[ao + 2].tag == DGN_FLOAT ? (int64_t)args[ao + 2].f : args[ao + 2].i;
        buf[off] = (unsigned char)(val & 0xFF);
        return dgn_null();
    }
    if (args[ao].tag == DGN_LIST) {
        if (off >= 0 && off < args[ao].list->len) args[ao].list->data[off] = args[ao + 2];
    }
    return dgn_null();
}

#ifdef _WIN32
#  include <windows.h>
#else
#  include <dlfcn.h>
#endif

DgnVal _dgn_ffi_sigs;

typedef struct {
    char* name;
    int offset;
    int size;
    int align;
} DgnStructField;

typedef struct {
    char* name;
    int size;
    DgnStructField* fields;
    int num_fields;
} DgnStructDef;

static DgnStructDef* _dgn_ffi_structs = NULL;
static int _dgn_ffi_structs_count = 0;
static int _dgn_ffi_structs_cap = 0;

static DgnStructDef* dgn_get_struct_def(const char* name) {
    for(int i=0; i<_dgn_ffi_structs_count; i++) {
        if(strcmp(_dgn_ffi_structs[i].name, name) == 0) return &_dgn_ffi_structs[i];
    }
    return NULL;
}

static DgnVal dgn_struct_def(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - off < 2 || args[off].tag != DGN_STR || args[off+1].tag != DGN_LIST) return dgn_null();

    const char* struct_name = args[off].s;
    DgnList* fields_list = args[off+1].list;

    if(_dgn_ffi_structs_count >= _dgn_ffi_structs_cap) {
        _dgn_ffi_structs_cap = _dgn_ffi_structs_cap == 0 ? 8 : _dgn_ffi_structs_cap * 2;
        _dgn_ffi_structs = (DgnStructDef*)realloc(_dgn_ffi_structs, _dgn_ffi_structs_cap * sizeof(DgnStructDef));
    }

    DgnStructDef* def = &_dgn_ffi_structs[_dgn_ffi_structs_count++];
    def->name = strdup(struct_name);
    def->num_fields = fields_list->len;
    def->fields = (DgnStructField*)calloc(def->num_fields, sizeof(DgnStructField));

    int offset = 0;
    int max_align = 1;

    for (int i = 0; i < fields_list->len; i++) {
        DgnVal field_def = fields_list->data[i];
        if (field_def.tag != DGN_LIST || field_def.list->len < 2) continue;

        const char* type_name = field_def.list->data[0].tag == DGN_STR ? field_def.list->data[0].s : "";
        const char* field_name = field_def.list->data[1].tag == DGN_STR ? field_def.list->data[1].s : "";

        int size = 0, align = 1;
        if(strcmp(type_name,"u8")==0 || strcmp(type_name,"i8")==0 || strcmp(type_name,"byte")==0 || strcmp(type_name,"char")==0) { size=1; align=1; }
        else if(strcmp(type_name,"u16")==0 || strcmp(type_name,"i16")==0 || strcmp(type_name,"short")==0) { size=2; align=2; }
        else if(strcmp(type_name,"u32")==0 || strcmp(type_name,"i32")==0 || strcmp(type_name,"int")==0 || strcmp(type_name,"f32")==0 || strcmp(type_name,"float")==0) { size=4; align=4; }
        else if(strcmp(type_name,"u64")==0 || strcmp(type_name,"i64")==0 || strcmp(type_name,"long")==0 || strcmp(type_name,"f64")==0 || strcmp(type_name,"double")==0 || strcmp(type_name,"pointer")==0 || strcmp(type_name,"string")==0) { size=8; align=8; }

        int padding = (align - (offset % align)) % align;
        offset += padding;

        def->fields[i].name = strdup(field_name);
        def->fields[i].offset = offset;
        def->fields[i].size = size;
        def->fields[i].align = align;

        offset += size;
        if (align > max_align) max_align = align;
    }

    int final_padding = (max_align - (offset % max_align)) % max_align;
    def->size = offset + final_padding;

    return dgn_float((double)def->size);
}

static DgnVal dgn_struct_create(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if (n - off < 1 || args[off].tag != DGN_STR) return dgn_null();

    DgnStructDef* def = dgn_get_struct_def(args[off].s);
    if (!def) return dgn_null();

    DgnVal byte_array = dgn_list_new(0);
    for(int i=0; i<def->size; i++) dgn_list_push(&byte_array, dgn_int(0));

    DgnVal map = dgn_map_new();
    dgn_map_set(&map, "__struct_id__", dgn_str(def->name));
    dgn_map_set(&map, "__bytes__", byte_array);

    return map;
}

static DgnVal dgn_struct_set(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "__struct_id__").tag == DGN_NULL) ? 1 : 0;
    if (n - off < 3 || args[off].tag != DGN_MAP || args[off+1].tag != DGN_STR) return dgn_null();

    DgnVal obj = args[off];
    DgnVal struct_id_val = dgn_map_get(obj, "__struct_id__");
    if (struct_id_val.tag != DGN_STR) return dgn_null();

    DgnStructDef* def = dgn_get_struct_def(struct_id_val.s);
    if (!def) return dgn_null();

    const char* field_name = args[off+1].s;
    DgnVal val = args[off+2];

    DgnStructField* field = NULL;
    for(int i=0; i<def->num_fields; i++) {
        if(strcmp(def->fields[i].name, field_name) == 0) { field = &def->fields[i]; break; }
    }
    if(!field) return dgn_null();

    DgnVal buffer_val = dgn_map_get(obj, "__bytes__");
    if (buffer_val.tag != DGN_LIST) return dgn_null();
    DgnList* b = buffer_val.list;

    int offset = field->offset;
    if (offset + field->size <= b->len) {
        if(field->size == 4) {
            float f_val = (val.tag == DGN_FLOAT) ? (float)val.f : (float)val.i;
            uint32_t bytes;
            memcpy(&bytes, &f_val, sizeof(float));
            b->data[offset]   = dgn_int((bytes) & 0xFF);
            b->data[offset+1] = dgn_int((bytes >> 8) & 0xFF);
            b->data[offset+2] = dgn_int((bytes >> 16) & 0xFF);
            b->data[offset+3] = dgn_int((bytes >> 24) & 0xFF);
        }
    }

    DgnVal sync_val = (val.tag == DGN_FLOAT || val.tag == DGN_INT) ?
        dgn_float(val.tag == DGN_FLOAT ? val.f : (double)val.i) : val;
    dgn_map_set(&obj, field_name, sync_val);

    return dgn_null();
}

static DgnVal dgn_struct_get(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "__struct_id__").tag == DGN_NULL) ? 1 : 0;
    if (n - off < 2 || args[off].tag != DGN_MAP || args[off+1].tag != DGN_STR) return dgn_null();

    DgnVal obj = args[off];
    DgnVal struct_id_val = dgn_map_get(obj, "__struct_id__");
    if (struct_id_val.tag != DGN_STR) return dgn_null();

    DgnStructDef* def = dgn_get_struct_def(struct_id_val.s);
    if (!def) return dgn_null();

    const char* field_name = args[off+1].s;

    DgnStructField* field = NULL;
    for(int i=0; i<def->num_fields; i++) {
        if(strcmp(def->fields[i].name, field_name) == 0) { field = &def->fields[i]; break; }
    }
    if(!field) return dgn_null();

    DgnVal buffer_val = dgn_map_get(obj, "__bytes__");
    if (buffer_val.tag != DGN_LIST) return dgn_null();
    DgnList* b = buffer_val.list;

    int offset = field->offset;
    if (offset + 4 <= b->len) {
        uint32_t bytes = (uint8_t)b->data[offset].i
                       | ((uint8_t)b->data[offset+1].i << 8)
                       | ((uint8_t)b->data[offset+2].i << 16)
                       | ((uint8_t)b->data[offset+3].i << 24);
        float f_val;
        memcpy(&f_val, &bytes, sizeof(float));
        return dgn_float((double)f_val);
    }

    return dgn_null();
}

static DgnVal dgn_ffi_loadlib(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;

    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    void* handle = NULL;
#ifdef _WIN32
    handle = LoadLibraryA(args[off].s);
#else
    handle = dlopen(args[off].s, RTLD_LAZY);
#endif
    if(!handle) return dgn_null();

    DgnMap* raw = (DgnMap*)calloc(1, sizeof(DgnMap));
    raw->cap = 8; raw->len = 0;
    raw->entries = (DgnMapEntry*)calloc(8, sizeof(DgnMapEntry));
    DgnVal m; m.tag = DGN_MAP; m.map = raw;
    dgn_map_set(&m, "path", args[off]);
    dgn_map_set(&m, "ptr", dgn_int((int64_t)(uintptr_t)handle));
    return m;
}

static DgnVal dgn_ffi_def(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "ptr").tag == DGN_NULL) ? 1 : 0;

    if(n - off < 4 || args[off].tag != DGN_MAP || args[off+1].tag != DGN_STR || args[off+2].tag != DGN_STR || args[off+3].tag != DGN_LIST)
        return dgn_null();

    if(_dgn_ffi_sigs.tag == DGN_NULL) _dgn_ffi_sigs = dgn_map_new();

    DgnVal path_val = dgn_map_get(args[off], "path");
    char key[1024];
    snprintf(key, sizeof(key), "%s::%s", path_val.tag == DGN_STR ? path_val.s : "", args[off+1].s);

    DgnVal sig = dgn_map_new();
    dgn_map_set(&sig, "ret", args[off+2]);
    dgn_map_set(&sig, "args", args[off+3]);

    dgn_map_set(&_dgn_ffi_sigs, key, sig);
    return dgn_null();
}

static DgnVal dgn_ffi_symbol_addr(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "ptr").tag == DGN_NULL) ? 1 : 0;
    args += off; n -= off;

    if(n < 2 || args[0].tag != DGN_MAP || args[1].tag != DGN_STR) return dgn_null();
    void* handle = (void*)(uintptr_t)dgn_map_get(args[0], "ptr").i;
    if(!handle) return dgn_null();

    void* fn_ptr = NULL;
#ifdef _WIN32
    fn_ptr = (void*)GetProcAddress((HMODULE)handle, args[1].s);
#else
    fn_ptr = dlsym(handle, args[1].s);
#endif
    return dgn_int((int64_t)(uintptr_t)fn_ptr);
}

static DgnVal dgn_ffi_loadlib_call(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "ptr").tag == DGN_NULL) ? 1 : 0;
    args += off; n -= off;

    if(n < 3 || args[0].tag != DGN_MAP || args[1].tag != DGN_STR) return dgn_null();
    void* handle = (void*)(uintptr_t)dgn_map_get(args[0], "ptr").i;
    if(!handle) return dgn_null();

    void* fn_ptr = NULL;
#ifdef _WIN32
    fn_ptr = (void*)GetProcAddress((HMODULE)handle, args[1].s);
#else
    fn_ptr = dlsym(handle, args[1].s);
#endif
    if(!fn_ptr) return dgn_null();

    const char* input = (args[2].tag == DGN_STR) ? args[2].s : "";
    typedef char* (*StrFunc)(const char*);
    char* res = ((StrFunc)fn_ptr)(input);

    if(!res) return dgn_null();
    return dgn_str(res);
}

#ifdef _WIN32
#  define DGN_STDCALL __stdcall
#else
#  define DGN_STDCALL
#endif

typedef union { void* p; float f; double d; } DgnFfiArg;

#define DGN_FA_PTR  0
#define DGN_FA_FLT  1
#define DGN_FA_DBL  2

typedef void (DGN_STDCALL *DgnVoidFn_fff)(float,float,float);
typedef void (DGN_STDCALL *DgnVoidFn_ff )(float,float);
typedef void (DGN_STDCALL *DgnVoidFn_f  )(float);
typedef void (DGN_STDCALL *DgnVoidFn_ddd)(double,double,double);
typedef void (DGN_STDCALL *DgnVoidFn_dd )(double,double);
typedef void (DGN_STDCALL *DgnVoidFn_d  )(double);
typedef void (DGN_STDCALL *DgnVoidFn_ffff)(float,float,float,float);
typedef void (DGN_STDCALL *DgnVoidFn_fffff)(float,float,float,float,float);
typedef void (DGN_STDCALL *DgnVoidFn_ffffff)(float,float,float,float,float,float);
typedef void (DGN_STDCALL *DgnVoidFn_pff)(void*,float,float);
typedef void (DGN_STDCALL *DgnVoidFn_pfff)(void*,float,float,float);
typedef void (DGN_STDCALL *DgnVoidFn_pffff)(void*,float,float,float,float);

typedef void* (DGN_STDCALL *FfiFunc0)();
typedef void* (DGN_STDCALL *FfiFunc1)(void*);
typedef void* (DGN_STDCALL *FfiFunc2)(void*, void*);
typedef void* (DGN_STDCALL *FfiFunc3)(void*, void*, void*);
typedef void* (DGN_STDCALL *FfiFunc4)(void*, void*, void*, void*);
typedef void* (DGN_STDCALL *FfiFunc5)(void*, void*, void*, void*, void*);
typedef void* (DGN_STDCALL *FfiFunc6)(void*, void*, void*, void*, void*, void*);

static DgnVal dgn_ffi_call(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP && dgn_map_get(args[0], "ptr").tag == DGN_NULL) ? 1 : 0;
    args += off; n -= off;

    if(n < 2 || args[0].tag != DGN_MAP || args[1].tag != DGN_STR) return dgn_null();

    void* handle = (void*)(uintptr_t)dgn_map_get(args[0], "ptr").i;
    if(!handle) return dgn_null();

    void* fn_ptr = NULL;
#ifdef _WIN32
    fn_ptr = (void*)GetProcAddress((HMODULE)handle, args[1].s);
#else
    fn_ptr = dlsym(handle, args[1].s);
#endif
    if(!fn_ptr) return dgn_null();

    int limit = (n - 2 > 11) ? 11 : (n - 2);
    int   kinds[11]   = {0};
    DgnFfiArg cargs[11];
    memset(cargs, 0, sizeof(cargs));

    char* str_copies[11] = {NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL};

    for(int i = 0; i < limit; i++) {
        DgnVal val = args[2 + i];
        switch(val.tag) {
            case DGN_INT:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)(uintptr_t)val.i;
                break;
            case DGN_FLOAT:
                kinds[i]    = DGN_FA_FLT;
                cargs[i].f  = (float)val.f;
                break;
            case DGN_BOOL:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)(uintptr_t)(val.i ? 1 : 0);
                break;
            case DGN_STR: {
                size_t len  = strlen(val.s) + 1;
                char*  copy = (char*)malloc(len);
                strcpy(copy, val.s);
                str_copies[i] = copy;
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)copy;
                break;
            }
            case DGN_LIST:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)val.list->data;
                break;
            case DGN_MAP: {
                DgnVal ptr_val = dgn_map_get(val, "ptr");
                if(ptr_val.tag == DGN_NULL) ptr_val = dgn_map_get(val, "__ptr__");

                if (ptr_val.tag != DGN_NULL) {
                    kinds[i]    = DGN_FA_PTR;
                    cargs[i].p  = (void*)(uintptr_t)ptr_val.i;
                } else {
                    DgnVal bytes_val = dgn_map_get(val, "__bytes__");
                    if (bytes_val.tag == DGN_LIST) {
                        DgnList* b = bytes_val.list;
                        char* sbuf = (char*)malloc(b->len);
                        for(int _bi=0; _bi<b->len; _bi++) sbuf[_bi] = (char)b->data[_bi].i;
                        str_copies[i] = sbuf;
                        kinds[i]    = DGN_FA_PTR;
                        cargs[i].p  = (void*)sbuf;
                    } else {
                        kinds[i]    = DGN_FA_PTR;
                        cargs[i].p  = NULL;
                    }
                }
                break;
            }
            default:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = NULL;
                break;
        }
    }

    {
        DgnVal _path = dgn_map_get(args[0], "path");
        char _key[1024];
        snprintf(_key, sizeof(_key), "%s::%s",
                 _path.tag == DGN_STR ? _path.s : "", args[1].s);
        if (_dgn_ffi_sigs.tag == DGN_MAP) {
            DgnVal _sig = dgn_map_get(_dgn_ffi_sigs, _key);
            if (_sig.tag == DGN_MAP) {
                DgnVal _params = dgn_map_get(_sig, "args");
                if (_params.tag == DGN_LIST) {
                    for (int i = 0; i < limit && i < _params.list->len; i++) {
                        DgnVal _pt = _params.list->data[i];
                        if (_pt.tag == DGN_STR &&
                            (strcmp(_pt.s, "double") == 0 || strcmp(_pt.s, "f64") == 0)) {
                            DgnVal _orig = args[2 + i];
                            kinds[i]   = DGN_FA_DBL;
                            cargs[i].d = (_orig.tag == DGN_FLOAT) ? _orig.f : (double)_orig.i;
                        }
                    }
                }
            }
        }
    }

    int n_flt = 0, n_dbl = 0, n_ptr = 0;
    for(int i = 0; i < limit; i++) {
        if(kinds[i] == DGN_FA_FLT)      n_flt++;
        else if(kinds[i] == DGN_FA_DBL) n_dbl++;
        else                             n_ptr++;
    }

    if(limit > 0 && n_ptr == 0 && n_dbl == 0) {
        switch(limit) {
            case 1: ((DgnVoidFn_f  )fn_ptr)(cargs[0].f); break;
            case 2: ((DgnVoidFn_ff )fn_ptr)(cargs[0].f, cargs[1].f); break;
            case 3: ((DgnVoidFn_fff)fn_ptr)(cargs[0].f, cargs[1].f, cargs[2].f); break;
            case 4: ((DgnVoidFn_ffff  )fn_ptr)(cargs[0].f, cargs[1].f, cargs[2].f, cargs[3].f); break;
            case 5: ((DgnVoidFn_fffff )fn_ptr)(cargs[0].f, cargs[1].f, cargs[2].f, cargs[3].f, cargs[4].f); break;
            case 6: ((DgnVoidFn_ffffff)fn_ptr)(cargs[0].f, cargs[1].f, cargs[2].f, cargs[3].f, cargs[4].f, cargs[5].f); break;
            default: break;
        }
        for(int i = 0; i < limit; i++) if(str_copies[i]) free(str_copies[i]);
        return dgn_null();
    }

    if(limit > 0 && n_ptr == 0 && n_flt == 0) {
        typedef void (DGN_STDCALL *DgnVoidFn_dddd    )(double,double,double,double);
        typedef void (DGN_STDCALL *DgnVoidFn_ddddd   )(double,double,double,double,double);
        typedef void (DGN_STDCALL *DgnVoidFn_dddddd  )(double,double,double,double,double,double);
        typedef void (DGN_STDCALL *DgnVoidFn_ddddddddd)(double,double,double,double,double,double,double,double,double);
        typedef void* (DGN_STDCALL *DgnRetFn_d)(double);
        switch(limit) {
            case 1: {
    void* _r = ((DgnRetFn_d)fn_ptr)(cargs[0].d);
    for(int i=0;i<limit;i++) if(str_copies[i]) free(str_copies[i]);
    return dgn_int((int64_t)(uintptr_t)_r);
}
            case 2: ((DgnVoidFn_dd       )fn_ptr)(cargs[0].d,cargs[1].d); break;
            case 3: ((DgnVoidFn_ddd      )fn_ptr)(cargs[0].d,cargs[1].d,cargs[2].d); break;
            case 4: ((DgnVoidFn_dddd     )fn_ptr)(cargs[0].d,cargs[1].d,cargs[2].d,cargs[3].d); break;
            case 5: ((DgnVoidFn_ddddd    )fn_ptr)(cargs[0].d,cargs[1].d,cargs[2].d,cargs[3].d,cargs[4].d); break;
            case 6: ((DgnVoidFn_dddddd   )fn_ptr)(cargs[0].d,cargs[1].d,cargs[2].d,cargs[3].d,cargs[4].d,cargs[5].d); break;
            case 9: ((DgnVoidFn_ddddddddd)fn_ptr)(cargs[0].d,cargs[1].d,cargs[2].d,cargs[3].d,cargs[4].d,cargs[5].d,cargs[6].d,cargs[7].d,cargs[8].d); break;
            default: break;
        }
        for(int i = 0; i < limit; i++) if(str_copies[i]) free(str_copies[i]);
        return dgn_null();
    }

    if(n_ptr == 1 && kinds[0] == DGN_FA_PTR && n_flt > 0 && n_flt == limit - 1) {
        switch(limit) {
            case 3: ((DgnVoidFn_pff  )fn_ptr)(cargs[0].p, cargs[1].f, cargs[2].f); break;
            case 4: ((DgnVoidFn_pfff )fn_ptr)(cargs[0].p, cargs[1].f, cargs[2].f, cargs[3].f); break;
            case 5: ((DgnVoidFn_pffff)fn_ptr)(cargs[0].p, cargs[1].f, cargs[2].f, cargs[3].f, cargs[4].f); break;
            default: {
                void* pa[6] = { cargs[0].p, cargs[1].p, cargs[2].p, cargs[3].p, cargs[4].p, cargs[5].p };
                if(limit == 2)      ((FfiFunc2)fn_ptr)(pa[0], pa[1]);
                else if(limit >= 6) ((FfiFunc6)fn_ptr)(pa[0], pa[1], pa[2], pa[3], pa[4], pa[5]);
                break;
            }
        }
        for(int i = 0; i < limit; i++) if(str_copies[i]) free(str_copies[i]);
        return dgn_null();
    }
    if (n_dbl > 0 && n_flt == 0) {
        typedef void (DGN_STDCALL *Sig_pdddpp)(void*,double,double,double,void*,void*);
        typedef void (DGN_STDCALL *Sig_pdpp  )(void*,double,void*,void*);
        typedef void (DGN_STDCALL *Sig_pd    )(void*,double);
        typedef void (DGN_STDCALL *Sig_pdd   )(void*,double,double);
        typedef void (DGN_STDCALL *Sig_pddd  )(void*,double,double,double);
        unsigned int pat = 0;
        for(int i = 0; i < limit && i < 6; i++)
            pat |= (kinds[i] == DGN_FA_DBL ? 1u : 0u) << i;
        if      (limit==6 && pat==0x0E) ((Sig_pdddpp)fn_ptr)(cargs[0].p,cargs[1].d,cargs[2].d,cargs[3].d,cargs[4].p,cargs[5].p);
        else if (limit==4 && pat==0x02) ((Sig_pdpp  )fn_ptr)(cargs[0].p,cargs[1].d,cargs[2].p,cargs[3].p);
        else if (limit==2 && pat==0x02) ((Sig_pd    )fn_ptr)(cargs[0].p,cargs[1].d);
        else if (limit==3 && pat==0x06) ((Sig_pdd   )fn_ptr)(cargs[0].p,cargs[1].d,cargs[2].d);
        else if (limit==4 && pat==0x0E) ((Sig_pddd  )fn_ptr)(cargs[0].p,cargs[1].d,cargs[2].d,cargs[3].d);
        else {
            void* _pa[11]; for(int i=0;i<limit;i++) _pa[i]=cargs[i].p;
            if(limit<=6) ((FfiFunc6)fn_ptr)(_pa[0],_pa[1],_pa[2],_pa[3],_pa[4],_pa[5]);
        }
        for(int i = 0; i < limit; i++) if(str_copies[i]) free(str_copies[i]);
        return dgn_null();
    }
    void* res = NULL;
    void* pa[11];
    for(int i = 0; i < limit; i++) pa[i] = cargs[i].p;

    if(limit == 0)      res = ((FfiFunc0)fn_ptr)();
    else if(limit == 1) res = ((FfiFunc1)fn_ptr)(pa[0]);
    else if(limit == 2) res = ((FfiFunc2)fn_ptr)(pa[0], pa[1]);
    else if(limit == 3) res = ((FfiFunc3)fn_ptr)(pa[0], pa[1], pa[2]);
    else if(limit == 4) res = ((FfiFunc4)fn_ptr)(pa[0], pa[1], pa[2], pa[3]);
    else if(limit == 5) res = ((FfiFunc5)fn_ptr)(pa[0], pa[1], pa[2], pa[3], pa[4]);
    else if(limit >= 6) res = ((FfiFunc6)fn_ptr)(pa[0], pa[1], pa[2], pa[3], pa[4], pa[5]);

    for(int i = 0; i < limit; i++) if(str_copies[i]) free(str_copies[i]);

    const char* ret_type = "int";
    DgnVal path_val = dgn_map_get(args[0], "path");
    char key[1024];
    snprintf(key, sizeof(key), "%s::%s", path_val.tag == DGN_STR ? path_val.s : "", args[1].s);

    if(_dgn_ffi_sigs.tag == DGN_MAP) {
        DgnVal sig = dgn_map_get(_dgn_ffi_sigs, key);
        if(sig.tag == DGN_MAP) {
            DgnVal rt = dgn_map_get(sig, "ret");
            if(rt.tag == DGN_STR) ret_type = rt.s;
        }
    }

    if(strcmp(ret_type, "string") == 0 || strcmp(ret_type, "str") == 0 || strcmp(ret_type, "char*") == 0) {
        if(!res) return dgn_null();
        return dgn_str((const char*)res);
    } else if (strcmp(ret_type, "bool") == 0) {
        return dgn_bool(res != NULL);
    } else {
        return dgn_int((int64_t)(uintptr_t)res);
    }
}

/* ---- raw memory, port I/O and block device access: used by system software written in Dragon (bare metal), and
 * available in native host builds too (module "hw" with the memory subset, module "disk" backed by $DGN_DISK) ---- */
static int64_t dgn_bm_num(DgnVal v) { return v.tag == DGN_FLOAT ? (int64_t)v.f : v.i; }
#define DGN_BM_AO ((n > 0 && args[0].tag == DGN_MAP) ? 1 : 0)
#define DGN_PEEK(NAME, T, SIGNED) static DgnVal dgn_hw_##NAME(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; \
    if (n - ao < 1) return dgn_null(); return dgn_int((int64_t)(SIGNED)*(volatile T*)(uintptr_t)dgn_bm_num(args[ao])); }
#define DGN_POKE(NAME, T) static DgnVal dgn_hw_##NAME(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; \
    if (n - ao < 2) return dgn_null(); *(volatile T*)(uintptr_t)dgn_bm_num(args[ao]) = (T)dgn_bm_num(args[ao + 1]); return dgn_null(); }
DGN_PEEK(peek8, uint8_t, uint8_t) DGN_PEEK(peek16, uint16_t, uint16_t) DGN_PEEK(peek32, uint32_t, uint32_t) DGN_PEEK(peek64, uint64_t, int64_t)
DGN_POKE(poke8, uint8_t) DGN_POKE(poke16, uint16_t) DGN_POKE(poke32, uint32_t) DGN_POKE(poke64, uint64_t)
#if defined(__x86_64__)
static inline uint32_t dgn_port_in(int port, int size) { uint32_t v = 0; uint16_t p = (uint16_t)port;
    if (size == 1) { uint8_t b; __asm__ volatile("inb %1, %0" : "=a"(b) : "Nd"(p)); v = b; }
    else if (size == 2) { uint16_t w; __asm__ volatile("inw %1, %0" : "=a"(w) : "Nd"(p)); v = w; }
    else { __asm__ volatile("inl %1, %0" : "=a"(v) : "Nd"(p)); } return v; }
static inline void dgn_port_out(int port, int size, uint32_t v) { uint16_t p = (uint16_t)port;
    if (size == 1) __asm__ volatile("outb %0, %1" :: "a"((uint8_t)v), "Nd"(p));
    else if (size == 2) __asm__ volatile("outw %0, %1" :: "a"((uint16_t)v), "Nd"(p));
    else __asm__ volatile("outl %0, %1" :: "a"(v), "Nd"(p)); }
static inline void dgn_port_ins16(int port, void* addr, size_t n) { __asm__ volatile("rep insw" : "+D"(addr), "+c"(n) : "d"((uint16_t)port) : "memory"); }
static inline void dgn_port_outs16(int port, const void* addr, size_t n) { __asm__ volatile("rep outsw" : "+S"(addr), "+c"(n) : "d"((uint16_t)port) : "memory"); }
#elif defined(__aarch64__) && defined(DGN_BAREMETAL)
/* ARM has no port space. The PC-style accesses drivers make are mapped onto the QEMU virt PCIe host:
 * 0xCF8/0xCFC (config mechanism 1) -> ECAM, 0x70/0x71 (CMOS clock, binary mode) -> the PL031 RTC,
 * every other port -> the PCI I/O window. */
extern uintptr_t dgn_ecam_base;
#define DGN_ECAM dgn_ecam_base
#define DGN_PIOWIN 0x3eff0000UL
static uint32_t dgn_cfg_addr; static int dgn_cmos_idx;
static inline uint32_t dgn_cmos_read(int idx) {
    uint32_t t = *(volatile uint32_t*)0x09010000UL;
    uint32_t days = t / 86400, sec = t % 86400;
    uint32_t y = 1970, md[12] = {31,28,31,30,31,30,31,31,30,31,30,31};
    for (;;) { uint32_t yl = (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) ? 366 : 365; if (days < yl) break; days -= yl; y++; }
    if (y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)) md[1] = 29;
    uint32_t mo = 0; while (days >= md[mo]) { days -= md[mo]; mo++; }
    switch (idx) {
    case 0: return sec % 60;
    case 2: return (sec / 60) % 60;
    case 4: return sec / 3600;
    case 7: return days + 1;
    case 8: return mo + 1;
    case 9: return y % 100;
    case 0x0B: return 6;
    default: return 0; } }
static inline uintptr_t dgn_cfg_ptr(int port) {
    uint32_t a = dgn_cfg_addr; uint32_t bus = (a >> 16) & 255;
    if (!dgn_ecam_base || !(a & 0x80000000u) || bus > 15) return 0;
    return DGN_ECAM + ((uintptr_t)bus << 20) + (((a >> 11) & 31) << 15) + (((a >> 8) & 7) << 12) + (a & 252) + (port - 0xCFC); }
static inline uint32_t dgn_port_in(int port, int size) {
    if (port == 0x71) return dgn_cmos_read(dgn_cmos_idx);
    if (port == 0x70) return (uint32_t)dgn_cmos_idx;
    if (port == 0xCF8) return dgn_cfg_addr;
    if (port >= 0xCFC && port <= 0xCFF) { uintptr_t p = dgn_cfg_ptr(port);
        if (!p) return 0xFFFFFFFFu >> (32 - 8 * size);
        if (size == 1) return *(volatile uint8_t*)p; if (size == 2) return *(volatile uint16_t*)p; return *(volatile uint32_t*)p; }
    uintptr_t p = DGN_PIOWIN + (uint16_t)port;
    if (size == 1) return *(volatile uint8_t*)p; if (size == 2) return *(volatile uint16_t*)p; return *(volatile uint32_t*)p; }
static inline void dgn_port_out(int port, int size, uint32_t v) {
    if (port == 0x70) { dgn_cmos_idx = (int)(v & 0x7f); return; }
    if (port == 0x71) return;
    if (port == 0xCF8) { dgn_cfg_addr = v; return; }
    if (port >= 0xCFC && port <= 0xCFF) { uintptr_t p = dgn_cfg_ptr(port); if (!p) return;
        if (size == 1) *(volatile uint8_t*)p = (uint8_t)v; else if (size == 2) *(volatile uint16_t*)p = (uint16_t)v; else *(volatile uint32_t*)p = v; return; }
    uintptr_t p = DGN_PIOWIN + (uint16_t)port;
    if (size == 1) *(volatile uint8_t*)p = (uint8_t)v; else if (size == 2) *(volatile uint16_t*)p = (uint16_t)v; else *(volatile uint32_t*)p = v; }
static inline void dgn_port_ins16(int port, void* addr, size_t n) { uint16_t* d = (uint16_t*)addr; while (n--) *d++ = (uint16_t)dgn_port_in(port, 2); }
static inline void dgn_port_outs16(int port, const void* addr, size_t n) { const uint16_t* d = (const uint16_t*)addr; while (n--) dgn_port_out(port, 2, *d++); }
#else
static inline uint32_t dgn_port_in(int port, int size) { (void)port; (void)size; return 0; }
static inline void dgn_port_out(int port, int size, uint32_t v) { (void)port; (void)size; (void)v; }
static inline void dgn_port_ins16(int port, void* addr, size_t n) { (void)port; (void)addr; (void)n; }
static inline void dgn_port_outs16(int port, const void* addr, size_t n) { (void)port; (void)addr; (void)n; }
#endif
/* highest physical address the kernel keeps mapped (device registers above it cannot be reached) */
static DgnVal dgn_hw_maplimit_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e;
#if defined(__aarch64__) && defined(DGN_BAREMETAL)
    return dgn_int((int64_t)1 << 40);
#else
    return dgn_int((int64_t)1 << 39);
#endif
}
static DgnVal dgn_hw_ports_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e;
#if defined(__x86_64__) || (defined(__aarch64__) && defined(DGN_BAREMETAL))
    return dgn_bool(1);
#else
    return dgn_bool(0);
#endif
}
/* insw(port, addr, count) / outsw(port, addr, count): move count 16-bit words between a port and memory (rep insw / rep outsw) */
static DgnVal dgn_hw_insw(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null();
    dgn_port_ins16((int)dgn_bm_num(args[ao]), (void*)(uintptr_t)dgn_bm_num(args[ao + 1]), (size_t)dgn_bm_num(args[ao + 2])); return dgn_null(); }
static DgnVal dgn_hw_outsw(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null();
    dgn_port_outs16((int)dgn_bm_num(args[ao]), (const void*)(uintptr_t)dgn_bm_num(args[ao + 1]), (size_t)dgn_bm_num(args[ao + 2])); return dgn_null(); }
#define DGN_PORT_FN(NAME, SZ) \
static DgnVal dgn_hw_in##NAME(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); return dgn_int((int64_t)dgn_port_in((int)dgn_bm_num(args[ao]), SZ)); } \
static DgnVal dgn_hw_out##NAME(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_null(); dgn_port_out((int)dgn_bm_num(args[ao]), SZ, (uint32_t)dgn_bm_num(args[ao + 1])); return dgn_null(); }
DGN_PORT_FN(b, 1) DGN_PORT_FN(w, 2) DGN_PORT_FN(l, 4)
/* alloc(size[, align]) -> address of zeroed memory (never moved or collected; free(addr) gives it back) */
static DgnVal dgn_hw_alloc(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null();
    size_t sz = (size_t)dgn_bm_num(args[ao]), al = n - ao > 1 ? (size_t)dgn_bm_num(args[ao + 1]) : 16; if (al < 16) al = 16;
    char* raw = (char*)malloc(sz + al + 16); if (!raw) return dgn_int(0);
    uintptr_t a = ((uintptr_t)raw + 8 + al - 1) & ~(uintptr_t)(al - 1);
    ((char**)a)[-1] = raw; memset((void*)a, 0, sz); return dgn_int((int64_t)a);
}
static DgnVal dgn_hw_free(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); uintptr_t a = (uintptr_t)dgn_bm_num(args[ao]); if (a) free(((char**)a)[-1]); return dgn_null(); }
static DgnVal dgn_hw_memcpy(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null(); memmove((void*)(uintptr_t)dgn_bm_num(args[ao]), (void*)(uintptr_t)dgn_bm_num(args[ao + 1]), (size_t)dgn_bm_num(args[ao + 2])); return dgn_null(); }
static DgnVal dgn_hw_memset(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null(); memset((void*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), (size_t)dgn_bm_num(args[ao + 2])); return dgn_null(); }
typedef struct { const uint8_t* s; size_t n, p; uint32_t bb; int bc; uint8_t* o; size_t oc, op; int err; } DgnInf;
typedef struct { uint16_t cnt[16]; uint16_t sym[288]; } DgnHuf;
static int dgn_inf_bits(DgnInf* z, int need) {
    while (z->bc < need) { if (z->p >= z->n) { z->err = 1; return 0; } z->bb |= (uint32_t)z->s[z->p++] << z->bc; z->bc += 8; }
    int v = (int)(z->bb & ((1u << need) - 1)); z->bb >>= need; z->bc -= need; return v;
}
static void dgn_inf_build(DgnHuf* h, const uint8_t* len, int n) {
    uint16_t offs[16]; int i;
    for (i = 0; i < 16; i++) h->cnt[i] = 0;
    for (i = 0; i < n; i++) h->cnt[len[i]]++;
    offs[1] = 0; for (i = 1; i < 15; i++) offs[i + 1] = offs[i] + h->cnt[i];
    for (i = 0; i < n; i++) if (len[i]) h->sym[offs[len[i]]++] = (uint16_t)i;
}
static int dgn_inf_sym(DgnInf* z, const DgnHuf* h) {
    int code = 0, first = 0, index = 0, l;
    for (l = 1; l <= 15; l++) {
        code |= dgn_inf_bits(z, 1); if (z->err) return -1;
        int c = h->cnt[l];
        if (code - c < first) return h->sym[index + (code - first)];
        index += c; first += c; first <<= 1; code <<= 1;
    }
    return -1;
}
static int dgn_inf_codes(DgnInf* z, const DgnHuf* lc, const DgnHuf* dc) {
    static const uint16_t lb[29] = {3,4,5,6,7,8,9,10,11,13,15,17,19,23,27,31,35,43,51,59,67,83,99,115,131,163,195,227,258};
    static const uint8_t le[29] = {0,0,0,0,0,0,0,0,1,1,1,1,2,2,2,2,3,3,3,3,4,4,4,4,5,5,5,5,0};
    static const uint16_t db[30] = {1,2,3,4,5,7,9,13,17,25,33,49,65,97,129,193,257,385,513,769,1025,1537,2049,3073,4097,6145,8193,12289,16385,24577};
    static const uint8_t de[30] = {0,0,0,0,1,1,2,2,3,3,4,4,5,5,6,6,7,7,8,8,9,9,10,10,11,11,12,12,13,13};
    for (;;) {
        int s = dgn_inf_sym(z, lc); if (s < 0 || z->err) return -1;
        if (s < 256) { if (z->op >= z->oc) return -1; z->o[z->op++] = (uint8_t)s; }
        else if (s == 256) return 0;
        else {
            s -= 257; if (s >= 29) return -1;
            int len = lb[s] + dgn_inf_bits(z, le[s]); int ds = dgn_inf_sym(z, dc); if (ds < 0 || ds >= 30 || z->err) return -1;
            size_t dist = db[ds] + dgn_inf_bits(z, de[ds]); if (z->err || dist > z->op || z->op + len > z->oc) return -1;
            while (len--) { z->o[z->op] = z->o[z->op - dist]; z->op++; }
        }
    }
}
static int64_t dgn_inflate_raw(const uint8_t* src, size_t n, uint8_t* dst, size_t cap) {
    DgnInf z; z.s = src; z.n = n; z.p = 0; z.bb = 0; z.bc = 0; z.o = dst; z.oc = cap; z.op = 0; z.err = 0;
    DgnHuf lc, dc; int last;
    do {
        last = dgn_inf_bits(&z, 1); int type = dgn_inf_bits(&z, 2); if (z.err) return -1;
        if (type == 0) {
            z.bb = 0; z.bc = 0; if (z.p + 4 > z.n) return -1;
            size_t len = z.s[z.p] | (z.s[z.p + 1] << 8); z.p += 4;
            if (z.p + len > z.n || z.op + len > z.oc) return -1;
            memcpy(z.o + z.op, z.s + z.p, len); z.p += len; z.op += len;
        } else if (type == 1) {
            uint8_t l[288]; int i; for (i = 0; i < 144; i++) l[i] = 8; for (; i < 256; i++) l[i] = 9; for (; i < 280; i++) l[i] = 7; for (; i < 288; i++) l[i] = 8;
            dgn_inf_build(&lc, l, 288); for (i = 0; i < 30; i++) l[i] = 5; dgn_inf_build(&dc, l, 30);
            if (dgn_inf_codes(&z, &lc, &dc)) return -1;
        } else if (type == 2) {
            static const uint8_t ord[19] = {16,17,18,0,8,7,9,6,10,5,11,4,12,3,13,2,14,1,15};
            uint8_t l[320]; int nl = dgn_inf_bits(&z, 5) + 257, nd = dgn_inf_bits(&z, 5) + 1, nc = dgn_inf_bits(&z, 4) + 4, i;
            if (z.err || nl > 286 || nd > 30) return -1;
            for (i = 0; i < 19; i++) l[i] = 0;
            for (i = 0; i < nc; i++) l[ord[i]] = (uint8_t)dgn_inf_bits(&z, 3);
            if (z.err) return -1;
            DgnHuf cc; dgn_inf_build(&cc, l, 19);
            uint8_t ls[320]; int idx = 0;
            while (idx < nl + nd) {
                int s = dgn_inf_sym(&z, &cc); if (s < 0 || z.err) return -1;
                if (s < 16) ls[idx++] = (uint8_t)s;
                else {
                    int rep, v = 0;
                    if (s == 16) { if (idx == 0) return -1; v = ls[idx - 1]; rep = 3 + dgn_inf_bits(&z, 2); }
                    else if (s == 17) rep = 3 + dgn_inf_bits(&z, 3); else rep = 11 + dgn_inf_bits(&z, 7);
                    if (idx + rep > nl + nd) return -1;
                    while (rep--) ls[idx++] = (uint8_t)v;
                }
            }
            dgn_inf_build(&lc, ls, nl); dgn_inf_build(&dc, ls + nl, nd);
            if (dgn_inf_codes(&z, &lc, &dc)) return -1;
        } else return -1;
    } while (!last);
    return (int64_t)z.op;
}

static uint32_t dgn_s1_rol(uint32_t v, int n) { return (v << n) | (v >> (32 - n)); }
static void dgn_sha1_block(uint32_t* h, const uint8_t* b) {
    uint32_t w[80], a, bb, c, d, e, f, k, t; int i;
    for (i = 0; i < 16; i++) w[i] = ((uint32_t)b[i*4] << 24) | ((uint32_t)b[i*4+1] << 16) | ((uint32_t)b[i*4+2] << 8) | b[i*4+3];
    for (i = 16; i < 80; i++) w[i] = dgn_s1_rol(w[i-3] ^ w[i-8] ^ w[i-14] ^ w[i-16], 1);
    a = h[0]; bb = h[1]; c = h[2]; d = h[3]; e = h[4];
    for (i = 0; i < 80; i++) {
        if (i < 20) { f = (bb & c) | (~bb & d); k = 0x5A827999; } else if (i < 40) { f = bb ^ c ^ d; k = 0x6ED9EBA1; }
        else if (i < 60) { f = (bb & c) | (bb & d) | (c & d); k = 0x8F1BBCDC; } else { f = bb ^ c ^ d; k = 0xCA62C1D6; }
        t = dgn_s1_rol(a, 5) + f + e + k + w[i]; e = d; d = c; c = dgn_s1_rol(bb, 30); bb = a; a = t;
    }
    h[0] += a; h[1] += bb; h[2] += c; h[3] += d; h[4] += e;
}
static void dgn_sha1(const uint8_t* m, size_t n, uint8_t* out) {
    uint32_t h[5] = {0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0}; uint8_t blk[128]; size_t i, full = n / 64, rem = n % 64;
    for (i = 0; i < full; i++) dgn_sha1_block(h, m + i * 64);
    memset(blk, 0, sizeof blk); memcpy(blk, m + full * 64, rem); blk[rem] = 0x80;
    size_t tl = rem < 56 ? 64 : 128; uint64_t bits = (uint64_t)n * 8;
    for (i = 0; i < 8; i++) blk[tl - 1 - i] = (uint8_t)(bits >> (8 * i));
    dgn_sha1_block(h, blk); if (tl == 128) dgn_sha1_block(h, blk + 64);
    for (i = 0; i < 5; i++) { out[i*4] = h[i] >> 24; out[i*4+1] = h[i] >> 16; out[i*4+2] = h[i] >> 8; out[i*4+3] = h[i]; }
}
static void dgn_hmac_sha1(const uint8_t* key, size_t kn, const uint8_t* msg, size_t mn, uint8_t* out) {
    uint8_t k[64], ib[64 + 128], ob[64 + 20], ih[20]; size_t i; memset(k, 0, 64);
    if (kn > 64) dgn_sha1(key, kn, k); else memcpy(k, key, kn);
    if (mn > 128) mn = 128;
    for (i = 0; i < 64; i++) { ib[i] = k[i] ^ 0x36; ob[i] = k[i] ^ 0x5c; }
    memcpy(ib + 64, msg, mn); dgn_sha1(ib, 64 + mn, ih); memcpy(ob + 64, ih, 20); dgn_sha1(ob, 84, out);
}
static DgnVal dgn_hw_pbkdf2_sha1(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_int(-1);
    const uint8_t* pw = (const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]); int64_t pn = dgn_bm_num(args[ao + 1]);
    const uint8_t* salt = (const uint8_t*)(uintptr_t)dgn_bm_num(args[ao + 2]); int64_t sn = dgn_bm_num(args[ao + 3]);
    uint8_t* out = (uint8_t*)(uintptr_t)dgn_bm_num(args[ao + 4]);
    if (sn > 64 || pn > 200) return dgn_int(-1);
    uint8_t msg[68], u[20], acc[20]; int blk, it, i;
    for (blk = 1; blk <= 2; blk++) {
        memcpy(msg, salt, sn); msg[sn] = 0; msg[sn+1] = 0; msg[sn+2] = 0; msg[sn+3] = (uint8_t)blk;
        dgn_hmac_sha1(pw, pn, msg, sn + 4, u); memcpy(acc, u, 20);
        for (it = 1; it < 4096; it++) { dgn_hmac_sha1(pw, pn, u, 20, u); for (i = 0; i < 20; i++) acc[i] ^= u[i]; }
        memcpy(out + (blk - 1) * 20, acc, blk == 1 ? 20 : 12);
    }
    return dgn_int(0); }
static DgnVal dgn_hw_inflate(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_int(-1);
    const uint8_t* s = (const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]); int64_t sn = dgn_bm_num(args[ao + 1]);
    if (sn < 3) return dgn_int(-1);
    return dgn_int(dgn_inflate_raw(s + 2, (size_t)sn - 2, (uint8_t*)(uintptr_t)dgn_bm_num(args[ao + 2]), (size_t)dgn_bm_num(args[ao + 3]))); }
__attribute__((weak)) int dgn_imgdec_info(const uint8_t* src, int n, int* wh) { (void)src; (void)n; (void)wh; return 0; }
__attribute__((weak)) int dgn_imgdec(const uint8_t* src, int n, uint32_t* out, int cap) { (void)src; (void)n; (void)out; (void)cap; return -1; }
static DgnVal dgn_hw_img_info(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(-1);
    int wh[2] = {0, 0};
    if (!dgn_imgdec_info((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), wh)) return dgn_int(-1);
    uint32_t* o = (uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 2]); o[0] = (uint32_t)wh[0]; o[1] = (uint32_t)wh[1];
    return dgn_int(0); }
static DgnVal dgn_hw_img_decode(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_int(-1);
    return dgn_int(dgn_imgdec((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), (uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 2]), (int)dgn_bm_num(args[ao + 3]))); }
__attribute__((weak)) int dgn_webp_info(const uint8_t* src, int n, int* wh) { (void)src; (void)n; (void)wh; return 0; }
__attribute__((weak)) int dgn_webp_decode(const uint8_t* src, int n, uint32_t* out, int cap, int w) { (void)src; (void)n; (void)out; (void)cap; (void)w; return -1; }
static DgnVal dgn_hw_webp_info(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(-1);
    int wh[2] = {0, 0};
    if (!dgn_webp_info((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), wh)) return dgn_int(-1);
    uint32_t* o = (uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 2]); o[0] = (uint32_t)wh[0]; o[1] = (uint32_t)wh[1];
    return dgn_int(0); }
static DgnVal dgn_hw_webp_decode(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_int(-1);
    return dgn_int(dgn_webp_decode((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), (uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 2]), (int)dgn_bm_num(args[ao + 3]), (int)dgn_bm_num(args[ao + 4]))); }
__attribute__((weak)) void* dgn_aud_open(const uint8_t* d, int n, int* info) { (void)d; (void)n; (void)info; return 0; }
__attribute__((weak)) int dgn_aud_read(void* h, int16_t* out, int frames) { (void)h; (void)out; (void)frames; return 0; }
__attribute__((weak)) int dgn_aud_seek(void* h, int frame) { (void)h; (void)frame; return -1; }
__attribute__((weak)) void dgn_aud_close(void* h) { (void)h; }
static DgnVal dgn_hw_aud_open(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(0);
    void* h = dgn_aud_open((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), (int*)(uintptr_t)dgn_bm_num(args[ao + 2]));
    return dgn_int((int64_t)(uintptr_t)h); }
static DgnVal dgn_hw_aud_read(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(0);
    return dgn_int(dgn_aud_read((void*)(uintptr_t)dgn_bm_num(args[ao]), (int16_t*)(uintptr_t)dgn_bm_num(args[ao + 1]), (int)dgn_bm_num(args[ao + 2]))); }
static DgnVal dgn_hw_aud_seek(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_int(-1);
    return dgn_int(dgn_aud_seek((void*)(uintptr_t)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]))); }
static DgnVal dgn_hw_aud_close(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_int(0);
    dgn_aud_close((void*)(uintptr_t)dgn_bm_num(args[ao])); return dgn_int(0); }
static DgnVal dgn_hw_png_unfilter(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_int(-1);
    uint8_t* raw = (uint8_t*)(uintptr_t)dgn_bm_num(args[ao]); int64_t rb = dgn_bm_num(args[ao + 1]), h = dgn_bm_num(args[ao + 2]), bpp = dgn_bm_num(args[ao + 3]);
    uint8_t* prev = 0; int64_t y, i;
    for (y = 0; y < h; y++) {
        uint8_t* row = raw + y * (rb + 1); int ft = row[0]; uint8_t* c = row + 1;
        for (i = 0; i < rb; i++) {
            int l = i >= bpp ? c[i - bpp] : 0, u = prev ? prev[i] : 0, ul = (prev && i >= bpp) ? prev[i - bpp] : 0, add = 0;
            if (ft == 1) add = l; else if (ft == 2) add = u; else if (ft == 3) add = (l + u) >> 1;
            else if (ft == 4) { int p = l + u - ul, pa = p > l ? p - l : l - p, pb = p > u ? p - u : u - p, pc = p > ul ? p - ul : ul - p; add = (pa <= pb && pa <= pc) ? l : (pb <= pc ? u : ul); }
            c[i] = (uint8_t)(c[i] + add);
        }
        prev = c;
    }
    return dgn_null(); }
/* peek_str(addr, n) -> string of the n bytes (stops at the first NUL); poke_str(addr, string) writes the bytes (no NUL) and returns the length */
static DgnVal dgn_hw_peek_str(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_null();
    size_t len = (size_t)dgn_bm_num(args[ao + 1]); const char* p = (const char*)(uintptr_t)dgn_bm_num(args[ao]);
    char* b = (char*)malloc(len + 1); if (!b) return dgn_null(); memcpy(b, p, len); b[len] = 0; DgnVal v = dgn_str(b); free(b); return v;
}
static DgnVal dgn_hw_poke_str(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 2 || args[ao + 1].tag != DGN_STR) return dgn_null();
    size_t len = strlen(args[ao + 1].s); memcpy((void*)(uintptr_t)dgn_bm_num(args[ao]), args[ao + 1].s, len); return dgn_int((int64_t)len);
}
#ifndef DGN_BAREMETAL
/* host simulation of the block device: a file named by $DGN_DISK (so system software written in Dragon, e.g. the
 * file system, can be developed and tested natively before it runs on hardware) */
static FILE* dgn_host_disk; static uint64_t dgn_host_disk_sectors;
uint64_t dgn_bm_disk_init(void) {
    if (dgn_host_disk) return dgn_host_disk_sectors;
    const char* path = getenv("DGN_DISK"); if (!path) return 0;
    dgn_host_disk = fopen(path, "r+b"); if (!dgn_host_disk) return 0;
    fseek(dgn_host_disk, 0, SEEK_END); dgn_host_disk_sectors = (uint64_t)ftell(dgn_host_disk) / 512; return dgn_host_disk_sectors;
}
int dgn_bm_disk_rw(uint64_t sector, void* buf, uint32_t count, int write) {
    if (!dgn_host_disk || sector + count > dgn_host_disk_sectors) return -1;
    fseek(dgn_host_disk, (long)(sector * 512), SEEK_SET);
    size_t r = write ? fwrite(buf, 512, count, dgn_host_disk) : fread(buf, 512, count, dgn_host_disk);
    if (write) fflush(dgn_host_disk);
    return r == count ? 0 : -1;
}
#else
extern uint64_t dgn_bm_disk_init(void);
extern int dgn_bm_disk_rw(uint64_t sector, void* buf, uint32_t count, int write);
#endif
/* disk: size() -> sectors (0 = no disk); read(sector, count) -> string of count*512 chars (NUL shown as space);
 * write(sector, string) -> bool (padded with spaces); read_to / write_from(sector, count, addr) move raw sectors */
static DgnVal dgn_disk_size_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_disk_init()); }
static DgnVal dgn_disk_read_f(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_null();
    int64_t sec = dgn_bm_num(args[ao]), cnt = dgn_bm_num(args[ao + 1]);
    if (sec < 0 || cnt <= 0 || cnt > 4096) return dgn_null();
    char* buf = (char*)malloc((size_t)cnt * 512 + 1); if (!buf) return dgn_null();
    if (dgn_bm_disk_rw((uint64_t)sec, buf, (uint32_t)cnt, 0) != 0) { free(buf); return dgn_null(); }
    for (int64_t i = 0; i < cnt * 512; i++) if (buf[i] == 0) buf[i] = ' ';
    buf[cnt * 512] = 0; DgnVal v = dgn_str(buf); free(buf); return v;
}
static DgnVal dgn_disk_read_to_f(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_bool(0);
    return dgn_bool(dgn_bm_disk_rw((uint64_t)dgn_bm_num(args[ao]), (void*)(uintptr_t)dgn_bm_num(args[ao + 2]), (uint32_t)dgn_bm_num(args[ao + 1]), 0) == 0); }
static DgnVal dgn_disk_write_from_f(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_bool(0);
    return dgn_bool(dgn_bm_disk_rw((uint64_t)dgn_bm_num(args[ao]), (void*)(uintptr_t)dgn_bm_num(args[ao + 2]), (uint32_t)dgn_bm_num(args[ao + 1]), 1) == 0); }
static DgnVal dgn_disk_write_f(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 2 || args[ao + 1].tag != DGN_STR) return dgn_bool(0);
    int64_t sec = dgn_bm_num(args[ao]); size_t len = strlen(args[ao + 1].s); uint32_t cnt = (uint32_t)((len + 511) / 512);
    if (sec < 0) return dgn_bool(0);
    if (cnt == 0) return dgn_bool(1);
    char* buf = (char*)malloc((size_t)cnt * 512); if (!buf) return dgn_bool(0);
    memset(buf, ' ', (size_t)cnt * 512); memcpy(buf, args[ao + 1].s, len);
    int rc = dgn_bm_disk_rw((uint64_t)sec, buf, cnt, 1); free(buf); return dgn_bool(rc == 0);
}

#if defined(DGN_BAREMETAL) && !defined(DGN_USER)
/* ---- bare-metal modules: `hw` (device access, clock) and `task` (preemptive multitasking) ---- */
extern unsigned long long dgn_bm_now_us(void);
extern unsigned long long dgn_bm_ticks(void);
extern void   dgn_bm_putc(char c);
extern size_t dgn_bm_heap_used(void);
extern size_t dgn_bm_heap_peak(void);
extern void   dgn_bm_halt(int code) __attribute__((noreturn));
extern int    dgn_bm_task_create(void (*entry)(void*), const void* data, size_t n, size_t stack_bytes);
extern int    dgn_bm_task_create_aff(void (*entry)(void*), const void* data, size_t n, size_t stack_bytes, int aff);
extern int    dgn_bm_mutex_new(void);
extern int    dgn_bm_mutex_trylock(int m);
extern void   dgn_bm_mutex_lock(int m);
extern void   dgn_bm_mutex_unlock(int m);
extern void   dgn_bm_task_yield(void);
extern void   dgn_bm_task_sleep_ms(unsigned long long ms);
extern int    dgn_bm_task_id(void);
extern int    dgn_bm_task_alive(int id);
extern int    dgn_bm_task_count(void);

static DgnVal dgn_hw_micros(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_now_us()); }
extern char __bootinfo[] __attribute__((weak));
static DgnVal dgn_hw_bootinfo(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)(uintptr_t)__bootinfo); }
extern long dgn_ext_call(long cmd, long a, long b) __attribute__((weak));
static DgnVal dgn_hw_ext(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (!dgn_ext_call) return dgn_int(-1); long c = n - ao > 0 ? (long)dgn_bm_num(args[ao]) : 0, x = n - ao > 1 ? (long)dgn_bm_num(args[ao + 1]) : 0, y = n - ao > 2 ? (long)dgn_bm_num(args[ao + 2]) : 0; return dgn_int((int64_t)dgn_ext_call(c, x, y)); }
extern int dgn_ata_rw(uint64_t sector, int count, void* addr, int write); extern unsigned int dgn_ata_probe(void); extern void dgn_smp_set_parallel(int on); extern int dgn_smp_init(void); extern int dgn_smp_count(void); extern int dgn_smp_start(int cpu, int job, uint64_t arg); extern int dgn_smp_busy(int cpu); extern uint64_t dgn_smp_result(int cpu);
static DgnVal dgn_hw_smp_init(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_smp_init()); }
static DgnVal dgn_hw_smp_parallel(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); dgn_smp_set_parallel((int)dgn_bm_num(args[ao + 0])); return dgn_null(); }
static DgnVal dgn_hw_ata_probe(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int((int64_t)dgn_ata_probe()); }
static DgnVal dgn_hw_ata_rw(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_bool(0); return dgn_bool(dgn_ata_rw((uint64_t)dgn_bm_num(args[ao + 0]), (int)dgn_bm_num(args[ao + 1]), (void*)(uintptr_t)dgn_bm_num(args[ao + 2]), (int)dgn_bm_num(args[ao + 3]))); }
static DgnVal dgn_hw_csum(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(0);
    const uint8_t* p = (const uint8_t*)(uintptr_t)dgn_bm_num(args[ao + 0]); int64_t len = dgn_bm_num(args[ao + 1]); uint64_t s = (uint64_t)dgn_bm_num(args[ao + 2]);
    int64_t i = 0; for (; i + 1 < len; i += 2) s += ((uint64_t)p[i] << 8) | p[i + 1]; if (i < len) s += (uint64_t)p[i] << 8;
    while (s >> 16) s = (s & 0xFFFF) + (s >> 16); return dgn_int((int64_t)s); }
static DgnVal dgn_hw_smp_count(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_smp_count()); }
static DgnVal dgn_hw_smp_start(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(-1); return dgn_int(dgn_smp_start((int)dgn_bm_num(args[ao + 0]), (int)dgn_bm_num(args[ao + 1]), (uint64_t)dgn_bm_num(args[ao + 2]))); }
static DgnVal dgn_hw_smp_busy(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_int(0); return dgn_int(dgn_smp_busy((int)dgn_bm_num(args[ao + 0]))); }
static DgnVal dgn_hw_smp_result(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_int(0); return dgn_int((int64_t)dgn_smp_result((int)dgn_bm_num(args[ao + 0]))); }
static DgnVal dgn_hw_millis(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)(dgn_bm_now_us() / 1000)); }
static DgnVal dgn_hw_ticks(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_ticks()); }
extern void dgn_bm_cur_enable(int on);
extern void dgn_bm_input_inject(int kind, int type, int code, int value);
/* inject(kind, type, code, value): feeds a raw input event (Linux key codes, as the hardware drivers do) from a driver written in Dragon */
static DgnVal dgn_input_inject_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_null();
    dgn_bm_input_inject((int)dgn_bm_num(args[ao]), (int)dgn_bm_num(args[ao + 1]), (int)dgn_bm_num(args[ao + 2]), (int)dgn_bm_num(args[ao + 3])); return dgn_null(); }
static DgnVal dgn_input_cursor_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_bm_cur_enable(n - ao > 0 && dgn_truthy(args[ao])); return dgn_null(); }
extern size_t dgn_bm_stack_free(void);
static DgnVal dgn_hw_stack_free(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_stack_free()); }
extern int dgn_bm_getc_nb(void);
static DgnVal dgn_hw_getc_nb(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int(dgn_bm_getc_nb()); }
static DgnVal dgn_hw_putc(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); dgn_bm_putc((char)dgn_bm_num(args[ao])); return dgn_null(); }
static DgnVal dgn_hw_heap_used(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_heap_used()); }
static DgnVal dgn_hw_heap_peak(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_heap_peak()); }
static DgnVal dgn_hw_halt(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_bm_halt(n - ao > 0 ? (int)dgn_bm_num(args[ao]) : 0); }

typedef struct { DgnVal fn; DgnVal arg; int has_arg; } DgnTaskBlk;
static void dgn_task_entry(void* p) {            /* p points into this task's own stack, so the collector sees fn and arg */
    DgnTaskBlk* b = (DgnTaskBlk*)p;
    DgnVal a[1]; a[0] = b->arg;
    dgn_call_val(b->fn, a, b->has_arg ? 1 : 0);
}
static DgnVal dgn_task_spawn(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = (n > 0 && args[0].tag != DGN_FUNC) ? 1 : 0;
    if (n - ao < 1 || args[ao].tag != DGN_FUNC) return dgn_int(-1);
    DgnTaskBlk b; memset(&b, 0, sizeof b); b.fn = args[ao];
    if (n - ao > 1) { b.arg = args[ao + 1]; b.has_arg = 1; }
    size_t stack = (n - ao > 2) ? (size_t)dgn_bm_num(args[ao + 2]) * 1024 : 0;      /* optional stack size in KB */
    return dgn_int(dgn_bm_task_create(dgn_task_entry, &b, sizeof b, stack));
}
static DgnVal dgn_task_spawn_par(DgnVal* args, int n, DgnEnv* e) {   /* like spawn, but the task may run on any CPU core */
    (void)e; int ao = (n > 0 && args[0].tag != DGN_FUNC) ? 1 : 0;
    if (n - ao < 1 || args[ao].tag != DGN_FUNC) return dgn_int(-1);
    DgnTaskBlk b; memset(&b, 0, sizeof b); b.fn = args[ao];
    if (n - ao > 1) { b.arg = args[ao + 1]; b.has_arg = 1; }
    size_t stack = (n - ao > 2) ? (size_t)dgn_bm_num(args[ao + 2]) * 1024 : 0;
    return dgn_int(dgn_bm_task_create_aff(dgn_task_entry, &b, sizeof b, stack, 1));
}
static DgnVal dgn_task_yield(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; dgn_bm_task_yield(); return dgn_null(); }
static DgnVal dgn_task_sleep(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_bm_task_sleep_ms(n - ao > 0 ? (unsigned long long)dgn_bm_num(args[ao]) : 0); return dgn_null(); }
static DgnVal dgn_task_id(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_task_id()); }
static DgnVal dgn_task_alive(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; return dgn_bool(n - ao > 0 && dgn_bm_task_alive((int)dgn_bm_num(args[ao]))); }
static DgnVal dgn_task_count(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_task_count()); }
static DgnVal dgn_task_wait(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null();
    int id = (int)dgn_bm_num(args[ao]);
    while (dgn_bm_task_alive(id)) dgn_bm_task_sleep_ms(1);
    return dgn_null();
}
static DgnVal dgn_task_atomic(DgnVal* args, int n, DgnEnv* e) {   /* run fn with task switching off; an exception releases it */
    (void)e; int ao = (n > 0 && args[0].tag != DGN_FUNC) ? 1 : 0; if (n - ao < 1) return dgn_null();
    DgnTryFrame f; f.thrown = dgn_null();
    dgn_bm_crit_enter(); dgn_try_push(&f);
    if (setjmp(f.buf) == 0) { DgnVal r = dgn_call_val(args[ao], NULL, 0); dgn_try_pop(&f); dgn_bm_crit_exit(); return r; }
    dgn_bm_crit_exit(); dgn_raise(f.thrown); return dgn_null();
}
static DgnVal dgn_task_mutex(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_mutex_new()); }
static DgnVal dgn_task_lock(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao > 0) dgn_bm_mutex_lock((int)dgn_bm_num(args[ao])); return dgn_null(); }
static DgnVal dgn_task_unlock(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao > 0) dgn_bm_mutex_unlock((int)dgn_bm_num(args[ao])); return dgn_null(); }
static DgnVal dgn_task_locked(DgnVal* args, int n, DgnEnv* e) {   /* locked(mutex, fn): lock, run fn, unlock even on an exception */
    (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_null();
    int m = (int)dgn_bm_num(args[ao]); DgnVal fn = args[ao + 1];
    dgn_bm_mutex_lock(m);
    DgnTryFrame f; f.thrown = dgn_null(); dgn_try_push(&f);
    if (setjmp(f.buf) == 0) { DgnVal r = dgn_call_val(fn, NULL, 0); dgn_try_pop(&f); dgn_bm_mutex_unlock(m); return r; }
    dgn_bm_mutex_unlock(m); dgn_raise(f.thrown); return dgn_null();
}

#endif

#ifdef DGN_BAREMETAL
/* gfx and input: in the kernel they are the screen and the devices, in a user program (.dxe) the program's own window and its events */
extern int  dgn_bm_fb_init(int w, int h);
extern long long dgn_bm_fb_vram(void);
extern long long dgn_bm_fb_diag(void);
extern uint32_t* dgn_bm_fb_ptr;
extern int  dgn_bm_fb_w, dgn_bm_fb_h;
extern void dgn_gfx_clear(uint32_t c);
extern void dgn_gfx_pixel(int x, int y, uint32_t c);
extern uint32_t dgn_gfx_get(int x, int y);
extern void dgn_gfx_rect(int x, int y, int w, int h, uint32_t c);
extern void dgn_gfx_box(int x, int y, int w, int h, uint32_t c);
extern void dgn_gfx_line(int x0, int y0, int x1, int y1, uint32_t c);
extern void dgn_gfx_circle(int cx, int cy, int r, uint32_t c, int fill);
extern void dgn_gfx_text(int x, int y, const char* s, uint32_t fg, long bg, int scale);
extern void dgn_gfx_scroll(int dy, uint32_t bg);
extern int dgn_gfx_buffer(int on);
extern void dgn_gfx_present(void);
extern void dgn_gfx_glass_warm(const uint32_t* src, int stride, uint32_t tint, int ta);
extern void dgn_gfx_resize(uint32_t* dst, int dw, int dh, const uint32_t* src, int stride, int sx, int sy, int sw, int sh);
extern void dgn_gfx_sprite_make(uint32_t* dst, const uint32_t* b, const uint32_t* w, int n);
extern void dgn_gfx_sprite(const uint32_t* src, int sw, int sh, int dx, int dy);
extern void dgn_gfx_present_rect(int x, int y, int w, int h);
extern void dgn_gfx_clip(int x, int y, int w, int h);
extern void dgn_gfx_unclip(void);
extern void dgn_gfx_clip_push(int x, int y, int w, int h);
extern void dgn_gfx_clip_pop(void);
extern void dgn_gfx_blit(const uint32_t* src, int sw, int sh, int stride, int dx, int dy);
extern int dgn_gfx_jpeg_info(const uint8_t* src, int n);
extern int dgn_gfx_jpeg(const uint8_t* src, int n, uint32_t* dst);
extern void dgn_gfx_blit_scaled(const uint32_t* src, int sw, int sh, int dx, int dy, int dw, int dh, int cx, int cy, int cw, int ch);
extern void dgn_gfx_blend(int x, int y, int w, int h, uint32_t c, int a);
extern void dgn_gfx_rrect(int x, int y, int w, int h, int r, uint32_t c, int a);
extern void dgn_gfx_gradient(int x, int y, int w, int h, uint32_t c0, uint32_t c1, int dir);
extern void dgn_gfx_glow(int cx, int cy, int r, uint32_t c, int a);
extern void dgn_gfx_shadow(int x, int y, int w, int h, int s, int a, int dy, int r);
extern void dgn_gfx_blur(int x, int y, int w, int h, int rad, int passes);
extern void dgn_gfx_saturate(int x, int y, int w, int h, int pct);
extern void dgn_gfx_gpill(int x, int y, int w, int h, int r, uint32_t c0, uint32_t c1, int a);
extern void dgn_gfx_glass(const uint32_t* src, int stride, int x, int y, int w, int h, int r, uint32_t tint, int ta, int corners);
extern void dgn_gfx_rframe(int x, int y, int w, int h, int r, uint32_t c, int a, int corners);
extern void dgn_gfx_aaline(float x0, float y0, float x1, float y1, float w, uint32_t c, int a);
extern void dgn_gfx_aaring(float cx, float cy, float r, float th, uint32_t c, int a);
extern void dgn_gfx_aadisc(float cx, float cy, float r, uint32_t c, int a);
extern int dgn_gfx_twidth(const char* s, int size, int flags);
extern int dgn_gfx_fontadd(const void* data, int size, int slot);
extern int dgn_gfx_tmetric(int size, int flags, int which);
extern int dgn_gfx_ttext(int x, int y, const char* s, uint32_t c, int size, int flags);
#define DGN_GN(i) ((int)dgn_bm_num(args[ao + (i)]))
#define DGN_GC(i) ((uint32_t)dgn_bm_num(args[ao + (i)]))
extern int dgn_gfx_tw(void); extern int dgn_gfx_th(void);
static int dgn_gfx_k = 256;
extern int dgn_gfx_custom(void);
static inline int dgn_gk(void) { return dgn_gfx_k != 256 && !dgn_gfx_custom(); }
static inline int dgn_gs(int v) { return dgn_gk() ? (int)(((long)v * dgn_gfx_k + (v < 0 ? -128 : 128)) / 256) : v; }
static inline int dgn_gw(int x, int w) { if (!dgn_gk()) return w; int d = dgn_gs(x + w) - dgn_gs(x); return (w > 0 && d < 1) ? 1 : d; }
static DgnVal dgn_gfx_scale_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao > 0) { int k = DGN_GN(0); dgn_gfx_k = k < 64 ? 256 : k; } return dgn_int(dgn_gfx_k); }
static DgnVal dgn_gfx_devw_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_gfx_tw()); }
static DgnVal dgn_gfx_devh_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_gfx_th()); }
static DgnVal dgn_con_diag_f(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_fb_diag()); }
static DgnVal dgn_con_vram_f(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int((int64_t)dgn_bm_fb_vram()); }
static DgnVal dgn_gfx_init_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; int w = n - ao > 0 ? DGN_GN(0) : 800, h = n - ao > 1 ? DGN_GN(1) : 600; return dgn_bool(dgn_bm_fb_init(w, h)); }
extern int dgn_gfx_tw(void); extern int dgn_gfx_th(void); extern void dgn_gfx_surface(uint32_t* p, int w, int h);
static DgnVal dgn_gfx_width_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_gk() ? dgn_gfx_tw() * 256 / dgn_gfx_k : dgn_gfx_tw()); }
static DgnVal dgn_gfx_height_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_gk() ? dgn_gfx_th() * 256 / dgn_gfx_k : dgn_gfx_th()); }
/* surface(address, width, height): draw into a block of 32-bit pixels instead of the screen; surface(0) goes back to the screen */
static DgnVal dgn_gfx_surface_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO;
    if (n - ao < 3) { dgn_gfx_surface(0, 0, 0); return dgn_null(); }
    dgn_gfx_surface((uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), DGN_GN(2)); return dgn_null(); }
static DgnVal dgn_gfx_clear_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_gfx_clear(n - ao > 0 ? DGN_GC(0) : 0); return dgn_null(); }
static DgnVal dgn_gfx_pixel_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null(); dgn_gfx_pixel(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), DGN_GC(2)); return dgn_null(); }
static DgnVal dgn_gfx_get_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_null(); return dgn_int(dgn_gfx_get(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)))); }
static DgnVal dgn_gfx_rect_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_rect(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), DGN_GC(4)); return dgn_null(); }
static DgnVal dgn_gfx_box_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_box(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), DGN_GC(4)); return dgn_null(); }
static DgnVal dgn_gfx_line_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_line(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gs(DGN_GN(2)), dgn_gs(DGN_GN(3)), DGN_GC(4)); return dgn_null(); }
static DgnVal dgn_gfx_circle_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_null(); dgn_gfx_circle(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gs(DGN_GN(2)), DGN_GC(3), n - ao > 4 && dgn_truthy(args[ao + 4])); return dgn_null(); }
static DgnVal dgn_gfx_text_f(DgnVal* args, int n, DgnEnv* e) {   /* text(x, y, string, color[, scale[, background]]) */
    (void)e; int ao = DGN_BM_AO; if (n - ao < 4 || args[ao + 2].tag != DGN_STR) return dgn_null();
    int scale = n - ao > 4 ? DGN_GN(4) : 1; long bg = n - ao > 5 && args[ao + 5].tag != DGN_NULL ? (long)DGN_GC(5) : -1;
    dgn_gfx_text(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), args[ao + 2].s, DGN_GC(3), bg, dgn_gk() ? (scale * dgn_gfx_k + 128) / 256 : scale); return dgn_null();
}
#ifdef DGN_USER
extern void dgn_win_title(const char* t);
static DgnVal dgn_gfx_title_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao > 0 && args[ao].tag == DGN_STR) dgn_win_title(args[ao].s); return dgn_null(); }
#endif
static DgnVal dgn_gfx_buffer_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; return dgn_bool(dgn_gfx_buffer(n - ao > 0 && dgn_truthy(args[ao]))); }
static DgnVal dgn_gfx_present_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; dgn_gfx_present(); return dgn_null(); }
/* blit(address, width, height, x, y[, stride]): copy a block of 32-bit pixels (rows `stride` pixels apart, default width) to the screen */
static DgnVal dgn_gfx_blit_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null();
    int w = DGN_GN(1); dgn_gfx_blit((const uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), w, DGN_GN(2), n - ao > 5 ? DGN_GN(5) : w, dgn_gs(DGN_GN(3)), dgn_gs(DGN_GN(4))); return dgn_null(); }
/* jpeg_info(address, length): (width << 16) | height of a baseline JPEG, or a negative number when it cannot be decoded; jpeg(address, length, destination): decode it into 32-bit pixels, 0 on success */
static DgnVal dgn_gfx_jpeg_info_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_int(-1);
    return dgn_int(dgn_gfx_jpeg_info((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1))); }
static DgnVal dgn_gfx_jpeg_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(-1);
    return dgn_int(dgn_gfx_jpeg((const uint8_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), (uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 2]))); }
static DgnVal dgn_gfx_blit_scaled_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 7) return dgn_null();
    int hc = n - ao > 10; { extern int dgn_gfx_alpha; dgn_gfx_alpha = (n - ao > 11 && dgn_bm_num(args[ao + 11]) != 0); } dgn_gfx_blit_scaled((const uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), DGN_GN(2), dgn_gs(DGN_GN(3)), dgn_gs(DGN_GN(4)), dgn_gw(DGN_GN(3), DGN_GN(5)), dgn_gw(DGN_GN(4), DGN_GN(6)), hc ? dgn_gs(DGN_GN(7)) : 0, hc ? dgn_gs(DGN_GN(8)) : 0, hc ? dgn_gw(DGN_GN(7), DGN_GN(9)) : 0, hc ? dgn_gw(DGN_GN(8), DGN_GN(10)) : 0); return dgn_null(); }
static DgnVal dgn_gfx_blend_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 6) return dgn_null(); dgn_gfx_blend(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), DGN_GC(4), DGN_GN(5)); return dgn_null(); }
static DgnVal dgn_gfx_rrect_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 6) return dgn_null(); dgn_gfx_rrect(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), dgn_gs(DGN_GN(4)), DGN_GC(5), n - ao > 6 ? DGN_GN(6) : 255); return dgn_null(); }
static DgnVal dgn_gfx_gradient_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 6) return dgn_null(); dgn_gfx_gradient(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), DGN_GC(4), DGN_GC(5), n - ao > 6 ? DGN_GN(6) : 0); return dgn_null(); }
static DgnVal dgn_gfx_glow_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_glow(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gs(DGN_GN(2)), DGN_GC(3), DGN_GN(4)); return dgn_null(); }
static DgnVal dgn_gfx_shadow_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 6) return dgn_null(); dgn_gfx_shadow(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), dgn_gs(DGN_GN(4)), DGN_GN(5), n - ao > 6 ? dgn_gs(DGN_GN(6)) : 0, n - ao > 7 ? dgn_gs(DGN_GN(7)) : 0); return dgn_null(); }
static DgnVal dgn_gfx_saturate_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_saturate(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), DGN_GN(4)); return dgn_null(); }
static DgnVal dgn_gfx_gpill_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 8) return dgn_null(); dgn_gfx_gpill(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), dgn_gs(DGN_GN(4)), DGN_GC(5), DGN_GC(6), DGN_GN(7)); return dgn_null(); }
static DgnVal dgn_gfx_blur_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_blur(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), dgn_gs(DGN_GN(4)), n - ao > 5 ? DGN_GN(5) : 2); return dgn_null(); }
static DgnVal dgn_gfx_glass_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 9) return dgn_null(); dgn_gfx_glass((const uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), dgn_gs(DGN_GN(2)), dgn_gs(DGN_GN(3)), dgn_gw(DGN_GN(2), DGN_GN(4)), dgn_gw(DGN_GN(3), DGN_GN(5)), dgn_gs(DGN_GN(6)), DGN_GC(7), DGN_GN(8), n - ao > 9 ? DGN_GN(9) : 15); return dgn_null(); }
static DgnVal dgn_gfx_rframe_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 7) return dgn_null(); dgn_gfx_rframe(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), dgn_gw(DGN_GN(0), DGN_GN(2)), dgn_gw(DGN_GN(1), DGN_GN(3)), dgn_gs(DGN_GN(4)), DGN_GC(5), DGN_GN(6), n - ao > 7 ? DGN_GN(7) : 15); return dgn_null(); }
static inline float dgn_gf(int v8) { return dgn_gk() ? (float)v8 * (float)dgn_gfx_k / 2048.0f : (float)v8 / 8.0f; }
static DgnVal dgn_gfx_aaline_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 6) return dgn_null(); dgn_gfx_aaline(dgn_gf(DGN_GN(0)), dgn_gf(DGN_GN(1)), dgn_gf(DGN_GN(2)), dgn_gf(DGN_GN(3)), dgn_gf(DGN_GN(4)), DGN_GC(5), n - ao > 6 ? DGN_GN(6) : 255); return dgn_null(); }
static DgnVal dgn_gfx_aaring_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_aaring(dgn_gf(DGN_GN(0)), dgn_gf(DGN_GN(1)), dgn_gf(DGN_GN(2)), dgn_gf(DGN_GN(3)), DGN_GC(4), n - ao > 5 ? DGN_GN(5) : 255); return dgn_null(); }
static DgnVal dgn_gfx_aadisc_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_null(); dgn_gfx_aadisc(dgn_gf(DGN_GN(0)), dgn_gf(DGN_GN(1)), dgn_gf(DGN_GN(2)), DGN_GC(3), n - ao > 4 ? DGN_GN(4) : 255); return dgn_null(); }
static DgnVal dgn_gfx_clip_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) { dgn_gfx_unclip(); return dgn_null(); } int x0 = dgn_gs(DGN_GN(0)), y0 = dgn_gs(DGN_GN(1)), x1 = dgn_gs(DGN_GN(0) + DGN_GN(2)), y1 = dgn_gs(DGN_GN(1) + DGN_GN(3)); dgn_gfx_clip(x0, y0, x1 - x0, y1 - y0); return dgn_null(); }
static DgnVal dgn_gfx_clip_push_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_null(); int x0 = dgn_gs(DGN_GN(0)), y0 = dgn_gs(DGN_GN(1)), x1 = dgn_gs(DGN_GN(0) + DGN_GN(2)), y1 = dgn_gs(DGN_GN(1) + DGN_GN(3)); dgn_gfx_clip_push(x0, y0, x1 - x0, y1 - y0); return dgn_null(); }
static DgnVal dgn_gfx_clip_pop_f(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; dgn_gfx_clip_pop(); return dgn_null(); }
static DgnVal dgn_gfx_present_rect_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) { dgn_gfx_present(); return dgn_null(); } int x0 = dgn_gs(DGN_GN(0)), y0 = dgn_gs(DGN_GN(1)), x1 = dgn_gs(DGN_GN(0) + DGN_GN(2)), y1 = dgn_gs(DGN_GN(1) + DGN_GN(3)); dgn_gfx_present_rect(x0, y0, x1 - x0, y1 - y0); return dgn_null(); }
static DgnVal dgn_gfx_resize_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 9) return dgn_null(); dgn_gfx_resize((uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), DGN_GN(2), (const uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 3]), DGN_GN(4), DGN_GN(5), DGN_GN(6), DGN_GN(7), DGN_GN(8)); return dgn_null(); }
static DgnVal dgn_gfx_sprite_make_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_null(); dgn_gfx_sprite_make((uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), (const uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 1]), (const uint32_t*)(uintptr_t)dgn_bm_num(args[ao + 2]), DGN_GN(3)); return dgn_null(); }
static DgnVal dgn_gfx_sprite_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_null(); dgn_gfx_sprite((const uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), DGN_GN(2), dgn_gs(DGN_GN(3)), dgn_gs(DGN_GN(4))); return dgn_null(); }
static DgnVal dgn_gfx_glass_warm_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_null(); dgn_gfx_glass_warm((const uint32_t*)(uintptr_t)dgn_bm_num(args[ao]), DGN_GN(1), DGN_GC(2), DGN_GN(3)); return dgn_null(); }
static DgnVal dgn_gfx_ttext_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5 || args[ao + 2].tag != DGN_STR) return dgn_int(0); return dgn_int(dgn_gfx_ttext(dgn_gs(DGN_GN(0)), dgn_gs(DGN_GN(1)), args[ao + 2].s, DGN_GC(3), dgn_gs(DGN_GN(4)), n - ao > 5 ? DGN_GN(5) : 0)); }
static DgnVal dgn_gfx_twidth_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2 || args[ao].tag != DGN_STR) return dgn_int(0); int w = dgn_gfx_twidth(args[ao].s, dgn_gs(DGN_GN(1)), n - ao > 2 ? DGN_GN(2) : 0); return dgn_int(dgn_gk() ? (w * 256 + dgn_gfx_k - 1) / dgn_gfx_k : w); }
static DgnVal dgn_gfx_tmetric_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(0); int v = dgn_gfx_tmetric(dgn_gs(DGN_GN(0)), DGN_GN(1), DGN_GN(2)); return dgn_int(dgn_gk() ? (v * 256 + dgn_gfx_k - 1) / dgn_gfx_k : v); }
static DgnVal dgn_gfx_fontadd_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_int(-1); return dgn_int(dgn_gfx_fontadd((const void*)(uintptr_t)dgn_bm_num(args[ao]), (int)DGN_GN(1), (int)DGN_GN(2))); }
static DgnVal dgn_gfx_scroll_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); dgn_gfx_scroll(DGN_GN(0), n - ao > 1 ? DGN_GC(1) : 0); return dgn_null(); }

extern int dgn_bm_input_init(void);
extern int dgn_bm_input_event(char* name, int* a, int* b, int* c);
static DgnVal dgn_input_init_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_input_init()); }
/* poll(): null when idle, otherwise {type:"key", key, down, shift, ctrl, alt} / {type:"mouse", x, y, left, right, middle} / {type:"wheel", delta} */
static DgnVal dgn_input_poll_f(DgnVal* a, int n, DgnEnv* e) {
    (void)a; (void)n; (void)e; char name[24]; int x = 0, y = 0, z = 0;
    int k = dgn_bm_input_event(name, &x, &y, &z);
    if (k == 0) return dgn_null();
    DgnVal m = dgn_map_new();
    if (k == 1) {
        dgn_map_set(&m, "type", dgn_str("key")); dgn_map_set(&m, "key", dgn_str(name)); dgn_map_set(&m, "down", dgn_bool(x));
        dgn_map_set(&m, "shift", dgn_bool(y & 1)); dgn_map_set(&m, "ctrl", dgn_bool(y & 2)); dgn_map_set(&m, "alt", dgn_bool(y & 4));
    } else if (k == 2) {
        dgn_map_set(&m, "type", dgn_str("mouse")); dgn_map_set(&m, "x", dgn_int(x)); dgn_map_set(&m, "y", dgn_int(y));
        dgn_map_set(&m, "left", dgn_bool(z & 1)); dgn_map_set(&m, "right", dgn_bool(z & 2)); dgn_map_set(&m, "middle", dgn_bool(z & 4));
    } else { dgn_map_set(&m, "type", dgn_str("wheel")); dgn_map_set(&m, "delta", dgn_int(x)); }
    return m;
}
#endif

#if defined(DGN_BAREMETAL) && !defined(DGN_USER)
/* con: the text console on the framebuffer (everything printed also appears there once con.init() ran) */
extern int dgn_bm_con_init(void); extern void dgn_bm_con_off(void); extern void dgn_bm_con_clear(void); extern void dgn_bm_con_redraw(void);
extern void dgn_bm_con_goto(int x, int y); extern void dgn_bm_con_color(uint32_t f, long b); extern void dgn_bm_con_cursor(int on);
extern int dgn_bm_con_cols(void), dgn_bm_con_rows(void), dgn_bm_con_x(void), dgn_bm_con_y(void);
extern int dgn_bm_con_fbonly, dgn_bm_con_seronly;
extern int dgn_bm_cap_begin(void); extern void dgn_bm_cap_end(int i); extern char* dgn_bm_cap_take(int i);
/* capture() -> slot: from now on what this task prints is collected; read(slot) -> collected text (and empties it); release(slot) */
static DgnVal dgn_con_capture_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_cap_begin()); }
static DgnVal dgn_con_read_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); char* t = dgn_bm_cap_take((int)dgn_bm_num(args[ao])); if (!t) return dgn_null(); DgnVal v = dgn_str(t); free(t); return v; }
static DgnVal dgn_con_release_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao > 0) dgn_bm_cap_end((int)dgn_bm_num(args[ao])); return dgn_null(); }
static DgnVal dgn_con_init_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; int ok = 1;
    if (!dgn_bm_fb_ptr || n - ao > 0) ok = dgn_bm_fb_init(n - ao > 0 ? DGN_GN(0) : 800, n - ao > 1 ? DGN_GN(1) : 600);
    return dgn_bool(ok && dgn_bm_con_init()); }
static DgnVal dgn_con_off_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; dgn_bm_con_off(); return dgn_null(); }
static DgnVal dgn_con_clear_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; dgn_bm_con_clear(); return dgn_null(); }
static DgnVal dgn_con_redraw_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; dgn_bm_con_redraw(); return dgn_null(); }
static DgnVal dgn_con_goto_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_null(); dgn_bm_con_goto(DGN_GN(0), DGN_GN(1)); return dgn_null(); }
static DgnVal dgn_con_color_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); dgn_bm_con_color(DGN_GC(0), n - ao > 1 && args[ao + 1].tag != DGN_NULL ? (long)DGN_GC(1) : -1); return dgn_null(); }
static DgnVal dgn_con_cursor_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_bm_con_cursor(n - ao > 0 && dgn_truthy(args[ao])); return dgn_null(); }
static DgnVal dgn_con_fbonly_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_bm_con_fbonly = n - ao > 0 && dgn_truthy(args[ao]); return dgn_null(); }
static DgnVal dgn_con_seronly_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; dgn_bm_con_seronly = n - ao > 0 && dgn_truthy(args[ao]); return dgn_null(); }
static DgnVal dgn_con_cols_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_con_cols()); }
static DgnVal dgn_con_rows_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_con_rows()); }
static DgnVal dgn_con_x_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_con_x()); }
static DgnVal dgn_con_y_f(DgnVal* a, int n, DgnEnv* e) { (void)a; (void)n; (void)e; return dgn_int(dgn_bm_con_y()); }
#endif

#if defined(DGN_BAREMETAL) && !defined(DGN_USER)
/* ---- user processes (x86-64 ring 3; other targets report failure) ---- */
extern int dgn_proc_create(uint64_t base, uint64_t size);
extern void dgn_proc_destroy(int pid);
extern int dgn_proc_alive(int pid);
extern uint64_t dgn_proc_base(int pid);
extern uint64_t dgn_proc_size(int pid);
extern uint64_t dgn_proc_mem(int pid);
extern int dgn_proc_copy(int pid, uint64_t ua, void* k, uint64_t n, int to_user);
extern int64_t dgn_proc_strlen(int pid, uint64_t ua, uint64_t max);
extern int dgn_proc_start(int pid, uint64_t entry, uint64_t sp, uint64_t a0, uint64_t a1);
extern uint64_t dgn_proc_reg(int pid, int i);
extern void dgn_proc_set_reg(int pid, int i, uint64_t v);
extern int dgn_proc_run(int pid);
extern int dgn_proc_map_shared(int pid, uint64_t va, uint64_t kaddr, uint64_t n);
extern int dgn_proc_release(int pid, uint64_t va, uint64_t n);
extern int dgn_proc_set_exec(int pid, uint64_t va, uint64_t n, int on);
extern int dgn_proc_fork(int pid);
extern int dgn_proc_clone(int pid);
extern uint64_t dgn_proc_frames_used(void);
extern uint64_t dgn_proc_frames_total(void);
#define DGN_PN(i) dgn_bm_num(args[ao + (i)])
static DgnVal dgn_proc_create_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_int(-1); return dgn_int(dgn_proc_create((uint64_t)DGN_PN(0), (uint64_t)DGN_PN(1))); }
static DgnVal dgn_proc_destroy_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); dgn_proc_destroy((int)DGN_PN(0)); return dgn_null(); }
static DgnVal dgn_proc_alive_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; return dgn_bool(n - ao > 0 && dgn_proc_alive((int)DGN_PN(0))); }
static DgnVal dgn_proc_base_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; return dgn_int(n - ao > 0 ? (int64_t)dgn_proc_base((int)DGN_PN(0)) : 0); }
static DgnVal dgn_proc_size_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; return dgn_int(n - ao > 0 ? (int64_t)dgn_proc_size((int)DGN_PN(0)) : 0); }
static DgnVal dgn_proc_mem_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; return dgn_int(n - ao > 0 ? (int64_t)dgn_proc_mem((int)DGN_PN(0)) : 0); }
/* copy_in(pid, user_addr, kernel_addr, n): kernel -> process;  copy_out(...): process -> kernel.  false when the range leaves the process's memory */
static DgnVal dgn_proc_copy_in_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_bool(0);
    return dgn_bool(dgn_proc_copy((int)DGN_PN(0), (uint64_t)DGN_PN(1), (void*)(uintptr_t)DGN_PN(2), (uint64_t)DGN_PN(3), 1) == 0); }
static DgnVal dgn_proc_copy_out_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_bool(0);
    return dgn_bool(dgn_proc_copy((int)DGN_PN(0), (uint64_t)DGN_PN(1), (void*)(uintptr_t)DGN_PN(2), (uint64_t)DGN_PN(3), 0) == 0); }
/* cstr(pid, user_addr, max): the NUL-terminated string in the process, or null when it is not entirely inside its memory */
static DgnVal dgn_proc_cstr_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null();
    int pid = (int)DGN_PN(0); uint64_t ua = (uint64_t)DGN_PN(1); int64_t len = dgn_proc_strlen(pid, ua, (uint64_t)DGN_PN(2));
    if (len < 0) return dgn_null();
    char* b = (char*)malloc((size_t)len + 1); if (!b) return dgn_null();
    dgn_proc_copy(pid, ua, b, (uint64_t)len, 0); b[len] = 0;
    DgnVal r = dgn_str(b); free(b); return r; }
static DgnVal dgn_proc_start_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 5) return dgn_bool(0);
    return dgn_bool(dgn_proc_start((int)DGN_PN(0), (uint64_t)DGN_PN(1), (uint64_t)DGN_PN(2), (uint64_t)DGN_PN(3), (uint64_t)DGN_PN(4)) == 0); }
static DgnVal dgn_proc_run_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_int(-1); return dgn_int(dgn_proc_run((int)DGN_PN(0))); }
static DgnVal dgn_proc_reg_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 2) return dgn_int(0); return dgn_int((int64_t)dgn_proc_reg((int)DGN_PN(0), (int)DGN_PN(1))); }
static DgnVal dgn_proc_set_reg_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_null(); dgn_proc_set_reg((int)dgn_bm_num(args[ao + 0]), (int)dgn_bm_num(args[ao + 1]), (uint64_t)dgn_bm_num(args[ao + 2])); return dgn_null(); }
static DgnVal dgn_proc_share_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_bool(0);
    return dgn_bool(dgn_proc_map_shared((int)DGN_PN(0), (uint64_t)DGN_PN(1), (uint64_t)DGN_PN(2), (uint64_t)DGN_PN(3)) == 0); }
static DgnVal dgn_proc_release_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 3) return dgn_bool(0);
    return dgn_bool(dgn_proc_release((int)DGN_PN(0), (uint64_t)DGN_PN(1), (uint64_t)DGN_PN(2)) == 0); }
static DgnVal dgn_proc_exec_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 4) return dgn_bool(0);
    return dgn_bool(dgn_proc_set_exec((int)DGN_PN(0), (uint64_t)DGN_PN(1), (uint64_t)DGN_PN(2), (int)DGN_PN(3)) == 0); }
static DgnVal dgn_proc_clone_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_int(-1); return dgn_int(dgn_proc_clone((int)DGN_PN(0))); }
static DgnVal dgn_proc_fork_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_int(-1); return dgn_int(dgn_proc_fork((int)DGN_PN(0))); }
static DgnVal dgn_proc_frames_used_f(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int((int64_t)dgn_proc_frames_used()); }
static DgnVal dgn_proc_frames_total_f(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int((int64_t)dgn_proc_frames_total()); }
extern int dgn_blob_count(void);
extern const char* dgn_blob_name(int i);
extern const void* dgn_blob_addr(int i);
extern size_t dgn_blob_size(int i);
static DgnVal dgn_blob_count_f(DgnVal* args, int n, DgnEnv* e) { (void)args; (void)n; (void)e; return dgn_int(dgn_blob_count()); }
static DgnVal dgn_blob_name_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); const char* s = dgn_blob_name((int)DGN_PN(0)); return s ? dgn_str(s) : dgn_null(); }
static DgnVal dgn_blob_addr_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); return dgn_int((int64_t)(uintptr_t)dgn_blob_addr((int)DGN_PN(0))); }
static DgnVal dgn_blob_size_f(DgnVal* args, int n, DgnEnv* e) { (void)e; int ao = DGN_BM_AO; if (n - ao < 1) return dgn_null(); return dgn_int((int64_t)dgn_blob_size((int)DGN_PN(0))); }
#endif

static DgnVal dgn_load_module(const char* name) {
    DgnVal m=dgn_map_new();
    if(strcmp(name,"math")==0){
        dgn_map_set(&m,"sqrt",  dgn_func(dgn_math_sqrt, 1)); dgn_map_set(&m,"floor", dgn_func(dgn_math_floor, 1));
        dgn_map_set(&m,"ceil",  dgn_func(dgn_math_ceil, 1)); dgn_map_set(&m,"abs",   dgn_func(dgn_math_abs, 1));
        dgn_map_set(&m,"sin",   dgn_func(dgn_math_sin, 1));  dgn_map_set(&m,"cos",   dgn_func(dgn_math_cos, 1));
        dgn_map_set(&m,"tan",   dgn_func(dgn_math_tan, 1));  dgn_map_set(&m,"log",   dgn_func(dgn_math_log, 1));
        dgn_map_set(&m,"exp",   dgn_func(dgn_math_exp, 1));  dgn_map_set(&m,"round", dgn_func(dgn_math_round, 1));
        dgn_map_set(&m,"pow",   dgn_func(dgn_math_pow, 2));  dgn_map_set(&m,"atan2", dgn_func(dgn_math_atan2, 2));
        dgn_map_set(&m,"PI",dgn_float(3.14159265358979323846)); dgn_map_set(&m,"E", dgn_float(2.71828182845904523536));
#ifdef INFINITY
        dgn_map_set(&m,"INF",dgn_float(INFINITY));
#else
        dgn_map_set(&m,"INF",dgn_float(1e500));
#endif
        dgn_map_set(&m,"range",dgn_func(dgn_range_wrap,-1));
    }
    else if(strcmp(name,"file")==0) {
            dgn_map_set(&m, "read_all",   dgn_func(dgn_file_read_text, 1));
            dgn_map_set(&m, "read_bytes",  dgn_func(dgn_file_read_bytes, 1));
            dgn_map_set(&m, "write",      dgn_func(dgn_file_write_text, 2));
            dgn_map_set(&m, "append",     dgn_func(dgn_file_append_text, 2));
            dgn_map_set(&m, "exists",     dgn_func(dgn_file_exists, 1));
            dgn_map_set(&m, "delete",     dgn_func(dgn_file_delete, 1));
            dgn_map_set(&m, "mkdir",      dgn_func(dgn_file_mkdir, 1));
            dgn_map_set(&m, "rmdir",      dgn_func(dgn_file_rmdir, 1));
            dgn_map_set(&m, "rename",     dgn_func(dgn_file_rename, 2));
            dgn_map_set(&m, "size",       dgn_func(dgn_file_size, 1));
            dgn_map_set(&m, "is_dir",     dgn_func(dgn_file_is_dir, 1));
        }
        else if(strcmp(name,"os")==0) {
dgn_map_set(&m, "name", dgn_func(dgn_os_name_fn, 0));
    dgn_map_set(&m, "cwd",     dgn_func(dgn_os_cwd, 0));
    dgn_map_set(&m, "chdir",   dgn_func(dgn_os_chdir, 1));
    dgn_map_set(&m, "getenv",  dgn_func(dgn_os_getenv, 1));
    dgn_map_set(&m, "setenv",  dgn_func(dgn_os_setenv, 2));
    dgn_map_set(&m, "system",  dgn_func(dgn_os_system, 1));
    dgn_map_set(&m, "listdir", dgn_func(dgn_os_listdir, 1));
    dgn_map_set(&m, "arch", dgn_func(dgn_os_arch_fn, 0));
}
else if(strcmp(name,"path")==0) {
    dgn_map_set(&m, "join",        dgn_func(dgn_path_join, -1));
    dgn_map_set(&m, "basename",    dgn_func(dgn_path_basename, 1));
    dgn_map_set(&m, "dirname",     dgn_func(dgn_path_dirname, 1));
    dgn_map_set(&m, "extname",     dgn_func(dgn_path_extname, 1));
    dgn_map_set(&m, "is_absolute", dgn_func(dgn_path_is_absolute, 1));
}
else if(strcmp(name,"time")==0) {
    dgn_map_set(&m, "now",      dgn_func(dgn_time_now, 0));
    dgn_map_set(&m, "unix",     dgn_func(dgn_time_now, 0));
    dgn_map_set(&m, "sleep_ms", dgn_func(dgn_time_sleep_ms, 1));
    dgn_map_set(&m, "date_now", dgn_func(dgn_time_date_now, 0));
}
else if(strcmp(name,"json")==0) {
    dgn_map_set(&m, "encode", dgn_func(dgn_json_encode, 1));
    dgn_map_set(&m, "decode", dgn_func(dgn_json_decode, 1));
}
else if(strcmp(name,"base64")==0) {
    dgn_map_set(&m, "encode", dgn_func(dgn_b64_encode, 1));
    dgn_map_set(&m, "decode", dgn_func(dgn_b64_decode, 1));
}
else if(strcmp(name,"uuid")==0) {
    dgn_map_set(&m, "v4", dgn_func(dgn_uuid_v4, 0));
}
else if(strcmp(name, "net") == 0) {
#ifdef _WIN32
    WSADATA wsa; WSAStartup(MAKEWORD(2,2), &wsa);
#endif
    dgn_map_set(&m, "http_get",          dgn_func(dgn_net_http_get,       1));
    dgn_map_set(&m, "http_post",         dgn_func(dgn_net_http_post,      3));
    dgn_map_set(&m, "http_put",          dgn_func(dgn_net_http_put,       3));
    dgn_map_set(&m, "http_delete",       dgn_func(dgn_net_http_delete,    1));
    dgn_map_set(&m, "socket_connect",    dgn_func(dgn_net_socket_connect, 2));
    dgn_map_set(&m, "socket_send",       dgn_func(dgn_net_socket_send,    2));
    dgn_map_set(&m, "socket_read",       dgn_func(dgn_net_socket_read,    1));
    dgn_map_set(&m, "socket_close",      dgn_func(dgn_net_socket_close,   1));
    dgn_map_set(&m, "start_http_server", dgn_func(dgn_net_start_server,   2));
}
#if !defined(DGN_BAREMETAL) || defined(DGN_USER)
    else if(strcmp(name, "hw") == 0) {
        dgn_map_set(&m, "peek8",  dgn_func(dgn_hw_peek8, 1));  dgn_map_set(&m, "peek16", dgn_func(dgn_hw_peek16, 1));
        dgn_map_set(&m, "peek32", dgn_func(dgn_hw_peek32, 1)); dgn_map_set(&m, "peek64", dgn_func(dgn_hw_peek64, 1));
        dgn_map_set(&m, "poke8",  dgn_func(dgn_hw_poke8, 2));  dgn_map_set(&m, "poke16", dgn_func(dgn_hw_poke16, 2));
        dgn_map_set(&m, "poke32", dgn_func(dgn_hw_poke32, 2)); dgn_map_set(&m, "poke64", dgn_func(dgn_hw_poke64, 2));
        dgn_map_set(&m, "alloc", dgn_func(dgn_hw_alloc, -1)); dgn_map_set(&m, "free", dgn_func(dgn_hw_free, 1));
        dgn_map_set(&m, "memcpy", dgn_func(dgn_hw_memcpy, 3)); dgn_map_set(&m, "memset", dgn_func(dgn_hw_memset, 3)); dgn_map_set(&m, "inflate", dgn_func(dgn_hw_inflate, 4)); dgn_map_set(&m, "png_unfilter", dgn_func(dgn_hw_png_unfilter, 4)); dgn_map_set(&m, "pbkdf2_sha1", dgn_func(dgn_hw_pbkdf2_sha1, 5)); dgn_map_set(&m, "img_info", dgn_func(dgn_hw_img_info, 3)); dgn_map_set(&m, "img_decode", dgn_func(dgn_hw_img_decode, 4)); dgn_map_set(&m, "webp_info", dgn_func(dgn_hw_webp_info, 3)); dgn_map_set(&m, "webp_decode", dgn_func(dgn_hw_webp_decode, 5)); dgn_map_set(&m, "aud_open", dgn_func(dgn_hw_aud_open, 3)); dgn_map_set(&m, "aud_read", dgn_func(dgn_hw_aud_read, 3)); dgn_map_set(&m, "aud_seek", dgn_func(dgn_hw_aud_seek, 2)); dgn_map_set(&m, "aud_close", dgn_func(dgn_hw_aud_close, 1));
        dgn_map_set(&m, "peek_str", dgn_func(dgn_hw_peek_str, 2)); dgn_map_set(&m, "poke_str", dgn_func(dgn_hw_poke_str, 2));
    }
#ifndef DGN_BAREMETAL
    else if(strcmp(name, "disk") == 0) {
        dgn_map_set(&m, "size", dgn_func(dgn_disk_size_f, 0)); dgn_map_set(&m, "read", dgn_func(dgn_disk_read_f, 2));
        dgn_map_set(&m, "write", dgn_func(dgn_disk_write_f, 2));
        dgn_map_set(&m, "read_to", dgn_func(dgn_disk_read_to_f, 3)); dgn_map_set(&m, "write_from", dgn_func(dgn_disk_write_from_f, 3));
    }
#endif
#endif
#ifdef DGN_USER
    else if(strcmp(name, "gfx") == 0) {
        dgn_map_set(&m, "title", dgn_func(dgn_gfx_title_f, 1));
        dgn_map_set(&m, "init",   dgn_func(dgn_gfx_init_f, -1)); dgn_map_set(&m, "width",  dgn_func(dgn_gfx_width_f, 0));
        dgn_map_set(&m, "height", dgn_func(dgn_gfx_height_f, 0)); dgn_map_set(&m, "clear",  dgn_func(dgn_gfx_clear_f, -1));
        dgn_map_set(&m, "pixel",  dgn_func(dgn_gfx_pixel_f, 3));  dgn_map_set(&m, "get",    dgn_func(dgn_gfx_get_f, 2));
        dgn_map_set(&m, "rect",   dgn_func(dgn_gfx_rect_f, 5));   dgn_map_set(&m, "box",    dgn_func(dgn_gfx_box_f, 5));
        dgn_map_set(&m, "line",   dgn_func(dgn_gfx_line_f, 5));   dgn_map_set(&m, "circle", dgn_func(dgn_gfx_circle_f, -1));
        dgn_map_set(&m, "text",   dgn_func(dgn_gfx_text_f, -1));  dgn_map_set(&m, "scroll", dgn_func(dgn_gfx_scroll_f, -1));
        dgn_map_set(&m, "buffer", dgn_func(dgn_gfx_buffer_f, 1)); dgn_map_set(&m, "surface", dgn_func(dgn_gfx_surface_f, -1)); dgn_map_set(&m, "present", dgn_func(dgn_gfx_present_f, 0)); dgn_map_set(&m, "blit", dgn_func(dgn_gfx_blit_f, -1)); dgn_map_set(&m, "blit_scaled", dgn_func(dgn_gfx_blit_scaled_f, -1)); dgn_map_set(&m, "jpeg_info", dgn_func(dgn_gfx_jpeg_info_f, -1)); dgn_map_set(&m, "jpeg", dgn_func(dgn_gfx_jpeg_f, -1));
        dgn_map_set(&m, "blend", dgn_func(dgn_gfx_blend_f, -1)); dgn_map_set(&m, "rrect", dgn_func(dgn_gfx_rrect_f, -1)); dgn_map_set(&m, "gradient", dgn_func(dgn_gfx_gradient_f, -1));
        dgn_map_set(&m, "glow", dgn_func(dgn_gfx_glow_f, -1)); dgn_map_set(&m, "shadow", dgn_func(dgn_gfx_shadow_f, -1)); dgn_map_set(&m, "blur", dgn_func(dgn_gfx_blur_f, -1)); dgn_map_set(&m, "saturate", dgn_func(dgn_gfx_saturate_f, -1)); dgn_map_set(&m, "gpill", dgn_func(dgn_gfx_gpill_f, -1));
        dgn_map_set(&m, "scale", dgn_func(dgn_gfx_scale_f, -1)); dgn_map_set(&m, "glass_warm", dgn_func(dgn_gfx_glass_warm_f, -1)); dgn_map_set(&m, "sprite_make", dgn_func(dgn_gfx_sprite_make_f, -1)); dgn_map_set(&m, "resize", dgn_func(dgn_gfx_resize_f, -1)); dgn_map_set(&m, "sprite", dgn_func(dgn_gfx_sprite_f, -1)); dgn_map_set(&m, "clip", dgn_func(dgn_gfx_clip_f, -1)); dgn_map_set(&m, "clip_push", dgn_func(dgn_gfx_clip_push_f, -1)); dgn_map_set(&m, "clip_pop", dgn_func(dgn_gfx_clip_pop_f, 0)); dgn_map_set(&m, "present_rect", dgn_func(dgn_gfx_present_rect_f, -1)); dgn_map_set(&m, "aaline", dgn_func(dgn_gfx_aaline_f, -1)); dgn_map_set(&m, "aaring", dgn_func(dgn_gfx_aaring_f, -1)); dgn_map_set(&m, "aadisc", dgn_func(dgn_gfx_aadisc_f, -1)); dgn_map_set(&m, "devw", dgn_func(dgn_gfx_devw_f, 0)); dgn_map_set(&m, "devh", dgn_func(dgn_gfx_devh_f, 0)); dgn_map_set(&m, "glass", dgn_func(dgn_gfx_glass_f, -1)); dgn_map_set(&m, "rframe", dgn_func(dgn_gfx_rframe_f, -1));
        dgn_map_set(&m, "ttext", dgn_func(dgn_gfx_ttext_f, -1)); dgn_map_set(&m, "twidth", dgn_func(dgn_gfx_twidth_f, -1)); dgn_map_set(&m, "tmetric", dgn_func(dgn_gfx_tmetric_f, -1)); dgn_map_set(&m, "fontadd", dgn_func(dgn_gfx_fontadd_f, -1));
    }
    else if(strcmp(name, "input") == 0) {
        dgn_map_set(&m, "init", dgn_func(dgn_input_init_f, 0)); dgn_map_set(&m, "poll", dgn_func(dgn_input_poll_f, 0));
    }
#endif
#if defined(DGN_BAREMETAL) && !defined(DGN_USER)
    else if(strcmp(name, "hw") == 0) {
        dgn_map_set(&m, "peek8",  dgn_func(dgn_hw_peek8, 1));  dgn_map_set(&m, "peek16", dgn_func(dgn_hw_peek16, 1));
        dgn_map_set(&m, "peek32", dgn_func(dgn_hw_peek32, 1)); dgn_map_set(&m, "peek64", dgn_func(dgn_hw_peek64, 1));
        dgn_map_set(&m, "poke8",  dgn_func(dgn_hw_poke8, 2));  dgn_map_set(&m, "poke16", dgn_func(dgn_hw_poke16, 2));
        dgn_map_set(&m, "poke32", dgn_func(dgn_hw_poke32, 2)); dgn_map_set(&m, "poke64", dgn_func(dgn_hw_poke64, 2));
        dgn_map_set(&m, "micros", dgn_func(dgn_hw_micros, 0)); dgn_map_set(&m, "smp_init", dgn_func(dgn_hw_smp_init, 0)); dgn_map_set(&m, "smp_count", dgn_func(dgn_hw_smp_count, 0)); dgn_map_set(&m, "smp_parallel", dgn_func(dgn_hw_smp_parallel, 1)); dgn_map_set(&m, "ata_rw", dgn_func(dgn_hw_ata_rw, 4)); dgn_map_set(&m, "ata_probe", dgn_func(dgn_hw_ata_probe, 0)); dgn_map_set(&m, "csum", dgn_func(dgn_hw_csum, 3)); dgn_map_set(&m, "smp_start", dgn_func(dgn_hw_smp_start, 3)); dgn_map_set(&m, "smp_busy", dgn_func(dgn_hw_smp_busy, 1)); dgn_map_set(&m, "smp_result", dgn_func(dgn_hw_smp_result, 1)); dgn_map_set(&m, "millis", dgn_func(dgn_hw_millis, 0)); dgn_map_set(&m, "bootinfo", dgn_func(dgn_hw_bootinfo, 0)); dgn_map_set(&m, "ext", dgn_func(dgn_hw_ext, -1));
        dgn_map_set(&m, "ticks",  dgn_func(dgn_hw_ticks, 0));  dgn_map_set(&m, "putc",   dgn_func(dgn_hw_putc, 1)); dgn_map_set(&m, "key", dgn_func(dgn_hw_getc_nb, 0));
        dgn_map_set(&m, "heap_used", dgn_func(dgn_hw_heap_used, 0)); dgn_map_set(&m, "heap_peak", dgn_func(dgn_hw_heap_peak, 0));
        dgn_map_set(&m, "halt",   dgn_func(dgn_hw_halt, 1)); dgn_map_set(&m, "stack_free", dgn_func(dgn_hw_stack_free, 0));
        dgn_map_set(&m, "inb", dgn_func(dgn_hw_inb, 1)); dgn_map_set(&m, "inw", dgn_func(dgn_hw_inw, 1)); dgn_map_set(&m, "inl", dgn_func(dgn_hw_inl, 1));
        dgn_map_set(&m, "ports", dgn_func(dgn_hw_ports_f, 0)); dgn_map_set(&m, "maplimit", dgn_func(dgn_hw_maplimit_f, 0)); dgn_map_set(&m, "insw", dgn_func(dgn_hw_insw, 3)); dgn_map_set(&m, "outsw", dgn_func(dgn_hw_outsw, 3));
        dgn_map_set(&m, "outb", dgn_func(dgn_hw_outb, 2)); dgn_map_set(&m, "outw", dgn_func(dgn_hw_outw, 2)); dgn_map_set(&m, "outl", dgn_func(dgn_hw_outl, 2));
        dgn_map_set(&m, "alloc", dgn_func(dgn_hw_alloc, -1)); dgn_map_set(&m, "free", dgn_func(dgn_hw_free, 1));
        dgn_map_set(&m, "memcpy", dgn_func(dgn_hw_memcpy, 3)); dgn_map_set(&m, "memset", dgn_func(dgn_hw_memset, 3)); dgn_map_set(&m, "inflate", dgn_func(dgn_hw_inflate, 4)); dgn_map_set(&m, "png_unfilter", dgn_func(dgn_hw_png_unfilter, 4)); dgn_map_set(&m, "pbkdf2_sha1", dgn_func(dgn_hw_pbkdf2_sha1, 5)); dgn_map_set(&m, "img_info", dgn_func(dgn_hw_img_info, 3)); dgn_map_set(&m, "img_decode", dgn_func(dgn_hw_img_decode, 4)); dgn_map_set(&m, "webp_info", dgn_func(dgn_hw_webp_info, 3)); dgn_map_set(&m, "webp_decode", dgn_func(dgn_hw_webp_decode, 5)); dgn_map_set(&m, "aud_open", dgn_func(dgn_hw_aud_open, 3)); dgn_map_set(&m, "aud_read", dgn_func(dgn_hw_aud_read, 3)); dgn_map_set(&m, "aud_seek", dgn_func(dgn_hw_aud_seek, 2)); dgn_map_set(&m, "aud_close", dgn_func(dgn_hw_aud_close, 1));
        dgn_map_set(&m, "peek_str", dgn_func(dgn_hw_peek_str, 2)); dgn_map_set(&m, "poke_str", dgn_func(dgn_hw_poke_str, 2));
    }
    else if(strcmp(name, "input") == 0) {
        dgn_map_set(&m, "init", dgn_func(dgn_input_init_f, 0)); dgn_map_set(&m, "poll", dgn_func(dgn_input_poll_f, 0));
        dgn_map_set(&m, "cursor", dgn_func(dgn_input_cursor_f, 1)); dgn_map_set(&m, "inject", dgn_func(dgn_input_inject_f, 4));
    }
    else if(strcmp(name, "disk") == 0) {
        dgn_map_set(&m, "size", dgn_func(dgn_disk_size_f, 0)); dgn_map_set(&m, "read", dgn_func(dgn_disk_read_f, 2));
        dgn_map_set(&m, "write", dgn_func(dgn_disk_write_f, 2));
        dgn_map_set(&m, "read_to", dgn_func(dgn_disk_read_to_f, 3)); dgn_map_set(&m, "write_from", dgn_func(dgn_disk_write_from_f, 3));
    }
    else if(strcmp(name, "proc") == 0) {
        dgn_map_set(&m, "create", dgn_func(dgn_proc_create_f, 2)); dgn_map_set(&m, "destroy", dgn_func(dgn_proc_destroy_f, 1));
        dgn_map_set(&m, "alive", dgn_func(dgn_proc_alive_f, 1)); dgn_map_set(&m, "base", dgn_func(dgn_proc_base_f, 1));
        dgn_map_set(&m, "size", dgn_func(dgn_proc_size_f, 1)); dgn_map_set(&m, "mem", dgn_func(dgn_proc_mem_f, 1));
        dgn_map_set(&m, "copy_in", dgn_func(dgn_proc_copy_in_f, 4)); dgn_map_set(&m, "copy_out", dgn_func(dgn_proc_copy_out_f, 4));
        dgn_map_set(&m, "cstr", dgn_func(dgn_proc_cstr_f, 3)); dgn_map_set(&m, "start", dgn_func(dgn_proc_start_f, 5));
        dgn_map_set(&m, "run", dgn_func(dgn_proc_run_f, 1)); dgn_map_set(&m, "reg", dgn_func(dgn_proc_reg_f, 2));
        dgn_map_set(&m, "set_reg", dgn_func(dgn_proc_set_reg_f, 3)); dgn_map_set(&m, "share", dgn_func(dgn_proc_share_f, 4));
        dgn_map_set(&m, "release", dgn_func(dgn_proc_release_f, 3)); dgn_map_set(&m, "set_exec", dgn_func(dgn_proc_exec_f, 4)); dgn_map_set(&m, "fork", dgn_func(dgn_proc_fork_f, 1)); dgn_map_set(&m, "clone", dgn_func(dgn_proc_clone_f, 1));
        dgn_map_set(&m, "frames_used", dgn_func(dgn_proc_frames_used_f, 0)); dgn_map_set(&m, "frames_total", dgn_func(dgn_proc_frames_total_f, 0));
    }
    else if(strcmp(name, "blob") == 0) {
        dgn_map_set(&m, "count", dgn_func(dgn_blob_count_f, 0)); dgn_map_set(&m, "name", dgn_func(dgn_blob_name_f, 1));
        dgn_map_set(&m, "addr", dgn_func(dgn_blob_addr_f, 1)); dgn_map_set(&m, "size", dgn_func(dgn_blob_size_f, 1));
    }
    else if(strcmp(name, "gfx") == 0) {
        dgn_map_set(&m, "init",   dgn_func(dgn_gfx_init_f, -1)); dgn_map_set(&m, "width",  dgn_func(dgn_gfx_width_f, 0));
        dgn_map_set(&m, "height", dgn_func(dgn_gfx_height_f, 0)); dgn_map_set(&m, "clear",  dgn_func(dgn_gfx_clear_f, -1));
        dgn_map_set(&m, "pixel",  dgn_func(dgn_gfx_pixel_f, 3));  dgn_map_set(&m, "get",    dgn_func(dgn_gfx_get_f, 2));
        dgn_map_set(&m, "rect",   dgn_func(dgn_gfx_rect_f, 5));   dgn_map_set(&m, "box",    dgn_func(dgn_gfx_box_f, 5));
        dgn_map_set(&m, "line",   dgn_func(dgn_gfx_line_f, 5));   dgn_map_set(&m, "circle", dgn_func(dgn_gfx_circle_f, -1));
        dgn_map_set(&m, "text",   dgn_func(dgn_gfx_text_f, -1));  dgn_map_set(&m, "scroll", dgn_func(dgn_gfx_scroll_f, -1));
        dgn_map_set(&m, "buffer", dgn_func(dgn_gfx_buffer_f, 1)); dgn_map_set(&m, "surface", dgn_func(dgn_gfx_surface_f, -1)); dgn_map_set(&m, "present", dgn_func(dgn_gfx_present_f, 0)); dgn_map_set(&m, "blit", dgn_func(dgn_gfx_blit_f, -1)); dgn_map_set(&m, "blit_scaled", dgn_func(dgn_gfx_blit_scaled_f, -1)); dgn_map_set(&m, "jpeg_info", dgn_func(dgn_gfx_jpeg_info_f, -1)); dgn_map_set(&m, "jpeg", dgn_func(dgn_gfx_jpeg_f, -1));
        dgn_map_set(&m, "blend", dgn_func(dgn_gfx_blend_f, -1)); dgn_map_set(&m, "rrect", dgn_func(dgn_gfx_rrect_f, -1)); dgn_map_set(&m, "gradient", dgn_func(dgn_gfx_gradient_f, -1));
        dgn_map_set(&m, "glow", dgn_func(dgn_gfx_glow_f, -1)); dgn_map_set(&m, "shadow", dgn_func(dgn_gfx_shadow_f, -1)); dgn_map_set(&m, "blur", dgn_func(dgn_gfx_blur_f, -1)); dgn_map_set(&m, "saturate", dgn_func(dgn_gfx_saturate_f, -1)); dgn_map_set(&m, "gpill", dgn_func(dgn_gfx_gpill_f, -1));
        dgn_map_set(&m, "scale", dgn_func(dgn_gfx_scale_f, -1)); dgn_map_set(&m, "glass_warm", dgn_func(dgn_gfx_glass_warm_f, -1)); dgn_map_set(&m, "sprite_make", dgn_func(dgn_gfx_sprite_make_f, -1)); dgn_map_set(&m, "resize", dgn_func(dgn_gfx_resize_f, -1)); dgn_map_set(&m, "sprite", dgn_func(dgn_gfx_sprite_f, -1)); dgn_map_set(&m, "clip", dgn_func(dgn_gfx_clip_f, -1)); dgn_map_set(&m, "clip_push", dgn_func(dgn_gfx_clip_push_f, -1)); dgn_map_set(&m, "clip_pop", dgn_func(dgn_gfx_clip_pop_f, 0)); dgn_map_set(&m, "present_rect", dgn_func(dgn_gfx_present_rect_f, -1)); dgn_map_set(&m, "aaline", dgn_func(dgn_gfx_aaline_f, -1)); dgn_map_set(&m, "aaring", dgn_func(dgn_gfx_aaring_f, -1)); dgn_map_set(&m, "aadisc", dgn_func(dgn_gfx_aadisc_f, -1)); dgn_map_set(&m, "devw", dgn_func(dgn_gfx_devw_f, 0)); dgn_map_set(&m, "devh", dgn_func(dgn_gfx_devh_f, 0)); dgn_map_set(&m, "glass", dgn_func(dgn_gfx_glass_f, -1)); dgn_map_set(&m, "rframe", dgn_func(dgn_gfx_rframe_f, -1));
        dgn_map_set(&m, "ttext", dgn_func(dgn_gfx_ttext_f, -1)); dgn_map_set(&m, "twidth", dgn_func(dgn_gfx_twidth_f, -1)); dgn_map_set(&m, "tmetric", dgn_func(dgn_gfx_tmetric_f, -1)); dgn_map_set(&m, "fontadd", dgn_func(dgn_gfx_fontadd_f, -1));
    }
    else if(strcmp(name, "con") == 0) {
        dgn_map_set(&m, "init", dgn_func(dgn_con_init_f, -1)); dgn_map_set(&m, "off", dgn_func(dgn_con_off_f, 0));
        dgn_map_set(&m, "clear", dgn_func(dgn_con_clear_f, 0)); dgn_map_set(&m, "redraw", dgn_func(dgn_con_redraw_f, 0));
        dgn_map_set(&m, "goto", dgn_func(dgn_con_goto_f, 2)); dgn_map_set(&m, "color", dgn_func(dgn_con_color_f, -1));
        dgn_map_set(&m, "cursor", dgn_func(dgn_con_cursor_f, 1)); dgn_map_set(&m, "vram", dgn_func(dgn_con_vram_f, 0)); dgn_map_set(&m, "diag", dgn_func(dgn_con_diag_f, 0)); dgn_map_set(&m, "fbonly", dgn_func(dgn_con_fbonly_f, 1)); dgn_map_set(&m, "serialonly", dgn_func(dgn_con_seronly_f, 1));
        dgn_map_set(&m, "capture", dgn_func(dgn_con_capture_f, 0)); dgn_map_set(&m, "read", dgn_func(dgn_con_read_f, 1)); dgn_map_set(&m, "release", dgn_func(dgn_con_release_f, 1));
        dgn_map_set(&m, "cols", dgn_func(dgn_con_cols_f, 0)); dgn_map_set(&m, "rows", dgn_func(dgn_con_rows_f, 0));
        dgn_map_set(&m, "x", dgn_func(dgn_con_x_f, 0)); dgn_map_set(&m, "y", dgn_func(dgn_con_y_f, 0));
    }
    else if(strcmp(name, "task") == 0) {
        dgn_map_set(&m, "spawn",  dgn_func(dgn_task_spawn, -1)); dgn_map_set(&m, "spawn_par", dgn_func(dgn_task_spawn_par, -1)); dgn_map_set(&m, "pause", dgn_func(dgn_task_yield, 0));
        dgn_map_set(&m, "sleep",  dgn_func(dgn_task_sleep, 1));  dgn_map_set(&m, "id",    dgn_func(dgn_task_id, 0));
        dgn_map_set(&m, "alive",  dgn_func(dgn_task_alive, 1));  dgn_map_set(&m, "count", dgn_func(dgn_task_count, 0));
        dgn_map_set(&m, "wait",   dgn_func(dgn_task_wait, 1));   dgn_map_set(&m, "atomic", dgn_func(dgn_task_atomic, 1));
        dgn_map_set(&m, "mutex",  dgn_func(dgn_task_mutex, 0));  dgn_map_set(&m, "lock",   dgn_func(dgn_task_lock, 1));
        dgn_map_set(&m, "unlock", dgn_func(dgn_task_unlock, 1)); dgn_map_set(&m, "locked", dgn_func(dgn_task_locked, 2));
    }
#endif
    else if(strcmp(name, "mem") == 0) {
        dgn_map_set(&m, "alloc", dgn_func(dgn_mem_alloc, 1));
        dgn_map_set(&m, "addr", dgn_func(dgn_mem_addr, 1));
        dgn_map_set(&m, "read_ptr", dgn_func(dgn_mem_read_ptr, 2));
        dgn_map_set(&m, "read_float", dgn_func(dgn_mem_read_float, 2));
        dgn_map_set(&m, "read_mem_int", dgn_func(dgn_mem_read_mem_int, 2));
        dgn_map_set(&m, "read_string", dgn_func(dgn_mem_read_string, 1));
        dgn_map_set(&m, "read_int", dgn_func(dgn_mem_read_int, 2));
        dgn_map_set(&m, "write_int", dgn_func(dgn_mem_write_int, 3));
        dgn_map_set(&m, "read_byte", dgn_func(dgn_mem_read_byte, 2));
        dgn_map_set(&m, "write_byte", dgn_func(dgn_mem_write_byte, 3));
    }

    else if(strcmp(name, "ffi") == 0) {
        if(_dgn_ffi_sigs.tag == DGN_NULL) _dgn_ffi_sigs = dgn_map_new();

        dgn_map_set(&m, "loadlib",      dgn_func(dgn_ffi_loadlib,      2));
dgn_map_set(&m, "def",          dgn_func(dgn_ffi_def,          5));
dgn_map_set(&m, "call",         dgn_func(dgn_ffi_call,        -1));
dgn_map_set(&m, "symbol_addr",  dgn_func(dgn_ffi_symbol_addr,  3));
dgn_map_set(&m, "loadlib_call", dgn_func(dgn_ffi_loadlib_call, 4));
    }

    else if(strcmp(name, "struct") == 0) {
        dgn_map_set(&m, "def",    dgn_func(dgn_struct_def,    2));
        dgn_map_set(&m, "create", dgn_func(dgn_struct_create, 1));
        dgn_map_set(&m, "set",    dgn_func(dgn_struct_set,    3));
        dgn_map_set(&m, "get",    dgn_func(dgn_struct_get,    2));
    }
    return m;
}

static DgnVal dgn_binop(const char* op, DgnVal a, DgnVal b) { (void)op;(void)a;(void)b; return dgn_null(); }
static DgnVal dgn_unary(const char* op, DgnVal a)            { (void)op;(void)a; return dgn_null(); }

typedef struct { char* data; size_t len; size_t cap; } DgnStrBuf;

static void dgn_strbuf_init(DgnStrBuf* sb, const char* seed) {
    size_t seed_len = strlen(seed);
    sb->cap = seed_len < 16 ? 16 : seed_len * 2;
    sb->data = (char*)malloc(sb->cap);
    memcpy(sb->data, seed, seed_len);
    sb->len = seed_len;
    sb->data[sb->len] = 0;
}
static void dgn_strbuf_ensure(DgnStrBuf* sb, size_t extra) {
    if (sb->len + extra + 1 > sb->cap) {
        size_t newcap = sb->cap * 2;
        while (newcap < sb->len + extra + 1) newcap *= 2;
        sb->data = (char*)realloc(sb->data, newcap);
        sb->cap = newcap;
    }
}
static void dgn_strbuf_append_val(DgnStrBuf* sb, DgnVal v) {
    char* s = dgn_val_to_cstr(v);
    size_t slen = strlen(s);
    dgn_strbuf_ensure(sb, slen);
    memcpy(sb->data + sb->len, s, slen);
    sb->len += slen;
    sb->data[sb->len] = 0;
    free(s);
}
static DgnVal dgn_strbuf_to_dgnval(DgnStrBuf* sb) {
    char* r = (char*)dgn_gc_alloc(sb->len + 1, DGN_STR);
    memcpy(r, sb->data, sb->len + 1);
    free(sb->data);
    DgnVal v; v.tag = DGN_STR; v.s = r;
    return v;
}

static int dgn_is_objlike(DgnVal v){ return v.tag==DGN_MAP||v.tag==DGN_CLASS||v.tag==DGN_INSTANCE; }
static DgnVal dgn_b_class_name(DgnVal v){
    if(v.tag==DGN_INSTANCE||v.tag==DGN_CLASS){ DgnVal n=dgn_map_get(v,"__name__"); if(n.tag==DGN_STR) return n; }
    return dgn_str("");
}
static DgnVal dgn_b_is_a(DgnVal obj, DgnVal cls){
    if(cls.tag!=DGN_CLASS || !(obj.tag==DGN_INSTANCE||obj.tag==DGN_CLASS)) return dgn_bool(0);
    DgnVal tn=dgn_map_get(cls,"__name__"); if(tn.tag!=DGN_STR) return dgn_bool(0);
    DgnVal cur=obj; int guard=0;
    while(dgn_is_objlike(cur) && guard++<64){
        DgnVal n=dgn_map_get(cur,"__name__");
        if(n.tag==DGN_STR && strcmp(n.s,tn.s)==0) return dgn_bool(1);
        cur=dgn_map_get(cur,"__parent__");
    }
    return dgn_bool(0);
}
static int dgn_map_has_key(DgnVal m,const char* key){
    if(!dgn_is_objlike(m)) return 0;
    return dgn_map_find(m.map, key) >= 0;
}
static DgnVal dgn_b_has_attr(DgnVal obj, DgnVal name){
    if(name.tag!=DGN_STR || !dgn_is_objlike(obj)) return dgn_bool(0);
    DgnVal cur=obj; int guard=0;
    while(dgn_is_objlike(cur) && guard++<64){
        if(dgn_map_has_key(cur,name.s)) return dgn_bool(1);
        cur=dgn_map_get(cur,"__parent__");
    }
    return dgn_bool(0);
}
static DgnVal dgn_b_set_attr(DgnVal obj, DgnVal name, DgnVal v){
    if(name.tag==DGN_STR && dgn_is_objlike(obj)) dgn_map_set(&obj,name.s,v);
    return dgn_null();
}
static DgnVal dgn_b_same(DgnVal a, DgnVal b){
    if(a.tag!=b.tag) return dgn_bool(0);
    switch(a.tag){
        case DGN_NULL: return dgn_bool(1);
        case DGN_BOOL: case DGN_INT: return dgn_bool(a.i==b.i);
        case DGN_FLOAT: return dgn_bool(a.f==b.f);
        case DGN_STR: return dgn_bool(strcmp(a.s,b.s)==0);
        case DGN_LIST: return dgn_bool(a.list==b.list);
        case DGN_MAP: case DGN_CLASS: case DGN_INSTANCE: return dgn_bool(a.map==b.map);
        case DGN_FUNC: return dgn_bool(a.func==b.func);
        default: return dgn_bool(0);
    }
}
static double dgn_num_of(DgnVal v){ return v.tag==DGN_INT? (double)v.i : (v.tag==DGN_FLOAT? v.f : 0.0); }
static int dgn_is_num(DgnVal v){ return v.tag==DGN_INT||v.tag==DGN_FLOAT; }
static DgnVal dgn_b_min(DgnVal* a,int n){ int bi=-1; for(int i=0;i<n;i++){ if(!dgn_is_num(a[i])) continue; if(bi<0||dgn_num_of(a[i])<dgn_num_of(a[bi])) bi=i; } return bi<0?dgn_null():a[bi]; }
static DgnVal dgn_b_max(DgnVal* a,int n){ int bi=-1; for(int i=0;i<n;i++){ if(!dgn_is_num(a[i])) continue; if(bi<0||dgn_num_of(a[i])>dgn_num_of(a[bi])) bi=i; } return bi<0?dgn_null():a[bi]; }
static DgnVal dgn_b_abs(DgnVal v){ if(v.tag==DGN_INT) return dgn_int(v.i<0?-v.i:v.i); return dgn_float(fabs(dgn_num_of(v))); }
static DgnVal dgn_b_pow(DgnVal a,DgnVal b){ return dgn_float(pow(dgn_num_of(a),dgn_num_of(b))); }
static DgnVal dgn_b_sin(DgnVal a){ return dgn_float(sin(dgn_num_of(a))); }
static DgnVal dgn_b_floor_int(DgnVal a){ return dgn_int((int64_t)floor(dgn_num_of(a))); }
static DgnVal dgn_b_round_int(DgnVal a){ return dgn_int((int64_t)llround(dgn_num_of(a))); }
static DgnVal dgn_b_now_ms(void){
#ifdef _WIN32
    return dgn_int((int64_t)GetTickCount64());
#else
    struct timespec ts; clock_gettime(CLOCK_REALTIME,&ts); return dgn_int((int64_t)ts.tv_sec*1000+ts.tv_nsec/1000000);
#endif
}
typedef struct { char* d; size_t n, cap; } DgnSb;
static void dgn_sb_put(DgnSb* b,const char* s,size_t k){ if(b->n+k+1>b->cap){ size_t c=b->cap?b->cap:256; while(c<b->n+k+1) c*=2; b->d=(char*)realloc(b->d,c); b->cap=c; } memcpy(b->d+b->n,s,k); b->n+=k; b->d[b->n]=0; }
static void dgn_sb_puts(DgnSb* b,const char* s){ dgn_sb_put(b,s,strlen(s)); }
static void dgn_json_put_str(DgnSb* b,const char* s){
    dgn_sb_put(b,"\"",1);
    for(const unsigned char* p=(const unsigned char*)s;*p;p++){
        switch(*p){
            case '"': dgn_sb_puts(b,"\\\""); break;
            case '\\': dgn_sb_puts(b,"\\\\"); break;
            case '\n': dgn_sb_puts(b,"\\n"); break;
            case '\r': dgn_sb_puts(b,"\\r"); break;
            case '\t': dgn_sb_puts(b,"\\t"); break;
            default:
                if(*p<0x20){ char u[8]; snprintf(u,sizeof u,"\\u%04x",*p); dgn_sb_puts(b,u); }
                else dgn_sb_put(b,(const char*)p,1);
        }
    }
    dgn_sb_put(b,"\"",1);
}
static void dgn_json_compact_into(DgnSb* b, DgnVal v, int depth){
    char buf[64];
    if(depth>200){ dgn_sb_puts(b,"null"); return; }
    switch(v.tag){
        case DGN_BOOL: dgn_sb_puts(b, v.i?"true":"false"); break;
        case DGN_INT: snprintf(buf,sizeof buf,"%lld",(long long)v.i); dgn_sb_puts(b,buf); break;
        case DGN_FLOAT:
            if(isfinite(v.f) && v.f==floor(v.f) && fabs(v.f)<9.0e15) snprintf(buf,sizeof buf,"%lld",(long long)v.f);
            else if(isfinite(v.f)) snprintf(buf,sizeof buf,"%.15g",v.f);
            else snprintf(buf,sizeof buf,"null");
            dgn_sb_puts(b,buf); break;
        case DGN_STR: dgn_json_put_str(b,v.s); break;
        case DGN_LIST:
            dgn_sb_put(b,"[",1);
            for(int i=0;i<v.list->len;i++){ if(i) dgn_sb_put(b,",",1); dgn_json_compact_into(b,v.list->data[i],depth+1); }
            dgn_sb_put(b,"]",1); break;
        case DGN_MAP: case DGN_INSTANCE: {
            int first=1; dgn_sb_put(b,"{",1);
            for(int i=0;i<v.map->cap;i++){
                if(!v.map->entries[i].used) continue;
                if(v.map->entries[i].val.tag==DGN_NULL) continue;
                if(!first) dgn_sb_put(b,",",1); first=0;
                dgn_json_put_str(b,v.map->entries[i].key); dgn_sb_put(b,":",1);
                dgn_json_compact_into(b,v.map->entries[i].val,depth+1);
            }
            dgn_sb_put(b,"}",1); break;
        }
        default: dgn_sb_puts(b,"null");
    }
}
static DgnVal dgn_b_json_compact(DgnVal v){
    DgnSb b={0,0,0}; dgn_json_compact_into(&b,v,0);
    DgnVal r=dgn_str(b.d?b.d:"null"); free(b.d); return r;
}

static char** _dgn_argv=NULL; static int _dgn_argc=0;
static void dgn_runtime_init(int argc, char** argv, void* stack_bottom) {
    _dgn_argc=argc; _dgn_argv=argv; srand((unsigned)time(NULL));
    _dgn_stack_bottom = stack_bottom;
}
"#;
