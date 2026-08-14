#![allow(dead_code)]

pub(crate) const RUNTIME_HEADER: &str = r#"
/* ============================================================
   Dragon Native Runtime (with Conservative GC)
   ============================================================ */
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
struct DgnMap  { DgnMapEntry* entries; int cap; int len; };
struct DgnFunc { DgnFuncPtr fn; int arity; DgnMap* env; };
typedef struct { jmp_buf buf; DgnVal thrown; } DgnTryFrame;
struct DgnEnv { DgnMap* map; };


/* ── Async / Promise ─────────────────────────────────────── */
typedef enum { DGN_PROMISE_PENDING, DGN_PROMISE_RESOLVED } DgnPromiseStatus;

typedef struct DgnPromise DgnPromise;
struct DgnPromise {
    DgnPromiseStatus status;
    DgnVal           result;
    int              done;      /* set to 1 by worker thread when finished */
#ifdef _WIN32
    HANDLE           thread;
    CRITICAL_SECTION lock;
#else
    pthread_t        thread;
    pthread_mutex_t  lock;
#endif
};

/* A socket handle stored inside a DgnVal (tag = DGN_SOCKET) */
#ifdef _WIN32
typedef SOCKET DgnSocket;
#else
typedef int    DgnSocket;
#endif

/* Extend DgnTag */
/* We reuse DGN_MAP tag for Promise (stored in map ptr as a DgnPromise*) */
/* Instead, add two new tags at the end: */
#undef  DGN_INSTANCE   /* temporarily undefine to re-declare enum cleanly */

/* ── Garbage Collector ───────────────────────────────────── */
typedef struct GcHeader GcHeader;
struct GcHeader { int marked; DgnTag tag; GcHeader* next; };

static GcHeader* _dgn_gc_head = NULL;
static void* _dgn_stack_bottom = NULL;

static void dgn_gc_mark_ptr(void* ptr) {
    if(!ptr) return;
    GcHeader* curr = _dgn_gc_head;
    while(curr) {
        void* payload = (void*)(curr + 1);
        if(ptr == payload) {
            if(!curr->marked) {
                curr->marked = 1;
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
            return;
        }
        curr = curr->next;
    }
}

extern DgnVal _dgn_aug_str;
extern DgnVal _dgn_aug_list;
extern int _dgn_aug_init;
/* Forward-declare the FFI signature registry so dgn_gc_collect can root it.
   The actual definition is further down in the FFI module section. */
extern DgnVal _dgn_ffi_sigs;

/* Generic registry of all top-level DgnVal globals.
   The transpiler fills _dgn_global_roots[] after emitting the globals,
   terminated by a DGN_NULL sentinel. dgn_gc_collect walks this list so
   static globals in BSS/data (invisible to the stack scanner) are rooted. */
extern DgnVal* _dgn_global_roots[];

/* Helper: mark any DgnVal regardless of tag */
static void dgn_gc_mark_val(DgnVal v) {
    if(v.tag==DGN_MAP||v.tag==DGN_CLASS||v.tag==DGN_INSTANCE) dgn_gc_mark_ptr(v.map);
    else if(v.tag==DGN_LIST) dgn_gc_mark_ptr(v.list);
    else if(v.tag==DGN_STR)  dgn_gc_mark_ptr(v.s);
    else if(v.tag==DGN_FUNC) dgn_gc_mark_ptr(v.func);
}

static void dgn_gc_collect() {
    if(_dgn_aug_init) {
        dgn_gc_mark_ptr(_dgn_aug_str.map);
        dgn_gc_mark_ptr(_dgn_aug_list.map);
    }
    /* Root the FFI signature registry */
    if(_dgn_ffi_sigs.tag == DGN_MAP) dgn_gc_mark_ptr(_dgn_ffi_sigs.map);
    /* Root all top-level Dragon globals — they live in BSS/data, not the stack */
    for(int _gi = 0; _dgn_global_roots[_gi] != NULL; _gi++)
        dgn_gc_mark_val(*_dgn_global_roots[_gi]);
    
    void* top;
    void** curr = (void**)&top;
    void** bottom = (void**)_dgn_stack_bottom;
    if(curr > bottom) { void** tmp = curr; curr = bottom; bottom = tmp; }
    while(curr < bottom) { dgn_gc_mark_ptr(*curr); curr++; }

    GcHeader** node = &_dgn_gc_head;
    while(*node) {
        if(!(*node)->marked) {
            GcHeader* unreached = *node;
            *node = unreached->next;
            if(unreached->tag == DGN_LIST) {
                DgnList* l = (DgnList*)(unreached + 1);
                if(l->data) free(l->data);
            } else if(unreached->tag == DGN_MAP || unreached->tag == DGN_CLASS || unreached->tag == DGN_INSTANCE) {
                DgnMap* m = (DgnMap*)(unreached + 1);
                if(m->entries) {
                    for(int i=0; i<m->cap; i++) if(m->entries[i].used && m->entries[i].key) free(m->entries[i].key);
                    free(m->entries);
                }
            }
            free(unreached);
        } else {
            (*node)->marked = 0;
            node = &(*node)->next;
        }
    }
}

static void* dgn_gc_alloc(size_t size, DgnTag tag) {
    /* The old auto-trigger here ran a conservative stack-scanning collector
       every 1000 allocations. That technique is unsound once the C source
       is compiled with optimizations (-O2 is always used by --native): the
       compiler is free to keep live DgnVal pointers purely in registers,
       which are invisible to the stack scanner, so live objects could be
       freed while still in use. For short-lived CLI programs it's much
       safer to just let allocations accumulate and let the OS reclaim
       everything at process exit. Call dgn_gc_collect() manually/explicitly
       if you ever need to reduce peak memory in a long-running program. */
    GcHeader* hdr = (GcHeader*)malloc(sizeof(GcHeader) + size);
    hdr->marked = 0;
    hdr->tag = tag;
    hdr->next = _dgn_gc_head;
    _dgn_gc_head = hdr;
    return (void*)(hdr + 1);
}

/* ════════════════════════════════════════════════════════════
   Constructors
   ════════════════════════════════════════════════════════════ */
static DgnVal dgn_null(void)      { DgnVal v; v.tag=DGN_NULL;  v.i=0;    return v; }
static DgnVal dgn_bool(int b)     { DgnVal v; v.tag=DGN_BOOL;  v.i=b?1:0;return v; }
static DgnVal dgn_int(int64_t n)  { DgnVal v; v.tag=DGN_INT;   v.i=n;    return v; }
static DgnVal dgn_float(double d) { DgnVal v; v.tag=DGN_FLOAT; v.f=d;    return v; }


static DgnVal dgn_str(const char* s) {
    DgnVal v; v.tag=DGN_STR;
    v.s=(char*)dgn_gc_alloc(strlen(s)+1, DGN_STR); strcpy(v.s,s); return v;
}

static DgnVal dgn_list_new(int hint) {
    DgnVal v; v.tag=DGN_LIST;
    v.list=(DgnList*)dgn_gc_alloc(sizeof(DgnList), DGN_LIST);
    int cap=hint>4?hint:4;
    v.list->data=(DgnVal*)malloc((size_t)cap*sizeof(DgnVal));
    v.list->cap=cap; v.list->len=0;
    return v;
}

static void dgn_list_push(DgnVal* lv, DgnVal item) {
    DgnList* l=lv->list;
    if(l->len>=l->cap){ l->cap*=2; l->data=(DgnVal*)realloc(l->data,(size_t)l->cap*sizeof(DgnVal)); }
    l->data[l->len++]=item;
}


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
    v.map->cap=8; v.map->len=0;
    v.map->entries=(DgnMapEntry*)calloc(8,sizeof(DgnMapEntry));
    return v;
}
static void dgn_map_set(DgnVal* mv, const char* key, DgnVal val) {
    DgnMap* m=mv->map;
    for(int i=0;i<m->cap;i++) if(m->entries[i].used && strcmp(m->entries[i].key,key)==0){ m->entries[i].val=val; return; }
    if(m->len>=m->cap*3/4){
        int nc=m->cap*2;
        m->entries=(DgnMapEntry*)realloc(m->entries,(size_t)nc*sizeof(DgnMapEntry));
        memset(m->entries+m->cap,0,(size_t)(nc-m->cap)*sizeof(DgnMapEntry));
        m->cap=nc;
    }
    for(int i=0;i<m->cap;i++) if(!m->entries[i].used){ m->entries[i].key=strdup(key); m->entries[i].val=val; m->entries[i].used=1; m->len++; return; }
}
static DgnVal dgn_map_get(DgnVal mv, const char* key) {
    DgnMap* m=mv.map;
    for(int i=0;i<m->cap;i++) if(m->entries[i].used && strcmp(m->entries[i].key,key)==0) return m->entries[i].val;
    return dgn_null();
}

static char* _dgn_val_to_cstr(DgnVal v, int depth) {
    char buf[8192]; buf[0]=0;
    if(depth > 10) return strdup("...");
    switch(v.tag){
        case DGN_NULL:  return strdup("null");
        case DGN_BOOL:  return strdup(v.i?"True":"False");
        case DGN_INT:   snprintf(buf,sizeof(buf),"%lld",(long long)v.i); return strdup(buf);
        case DGN_FLOAT: snprintf(buf,sizeof(buf),"%.10g",v.f); return strdup(buf);
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
    if(a.tag==DGN_STR&&b.tag==DGN_STR) return dgn_bool(strcmp(a.s,b.s)==0);
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

static int64_t dgn_len_raw(DgnVal v) {
    if(v.tag==DGN_STR)  return (int64_t)strlen(v.s);
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
    char buf[8192]; buf[0] = 0; DgnList* l = args[0].list;
    for(int i = 0; i < l->len; i++) {
        if(i > 0 && strlen(buf) + strlen(sep) < sizeof(buf) - 1) strcat(buf, sep);
        char* s = dgn_val_to_cstr(l->data[i]);
        if(strlen(buf) + strlen(s) < sizeof(buf) - 1) strcat(buf, s);
        free(s);
    }
    return dgn_str(buf);
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
    return dgn_bool(strncmp(args[0].s, args[1].s, strlen(args[1].s)) == 0);
}

static DgnVal dgn_str_ends_with(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR || args[1].tag != DGN_STR) return dgn_bool(0);
    int len1 = (int)strlen(args[0].s); int len2 = (int)strlen(args[1].s);
    if(len2 > len1) return dgn_bool(0);
    return dgn_bool(strcmp(args[0].s + len1 - len2, args[1].s) == 0);
}

static DgnVal dgn_str_index_of(DgnVal* args, int n, DgnEnv* env) {
    (void)env; if(n < 2 || args[0].tag != DGN_STR || args[1].tag != DGN_STR) return dgn_int(-1);
    const char* match = strstr(args[0].s, args[1].s);
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


/* ── Map Methods ────────────────────────────────────────── */
static DgnVal dgn_map_set_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<3 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_null(); dgn_map_set(&args[0], args[1].s, args[2]); return dgn_null(); }
static DgnVal dgn_map_get_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_null(); return dgn_map_get(args[0], args[1].s); }
static DgnVal dgn_map_remove_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_bool(0); DgnMap* m = args[0].map; for(int i=0; i<m->cap; i++) { if(m->entries[i].used && strcmp(m->entries[i].key, args[1].s)==0) { m->entries[i].used = 0; m->len--; return dgn_bool(1); } } return dgn_bool(0); }
static DgnVal dgn_map_contains_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_MAP || args[1].tag!=DGN_STR) return dgn_bool(0); return dgn_bool(dgn_map_get(args[0], args[1].s).tag != DGN_NULL); }
static DgnVal dgn_map_clear_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_MAP) return dgn_null(); DgnMap* m = args[0].map; for(int i=0; i<m->cap; i++) m->entries[i].used = 0; m->len = 0; return dgn_null(); }
static DgnVal dgn_map_keys_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_MAP) return dgn_list_new(0); DgnMap* m = args[0].map; DgnVal l = dgn_list_new(m->len); for(int i=0; i<m->cap; i++) if(m->entries[i].used) dgn_list_push(&l, dgn_str(m->entries[i].key)); return l; }
static DgnVal dgn_map_values_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_MAP) return dgn_list_new(0); DgnMap* m = args[0].map; DgnVal l = dgn_list_new(m->len); for(int i=0; i<m->cap; i++) if(m->entries[i].used) dgn_list_push(&l, m->entries[i].val); return l; }

/* ── List & ByteArray Methods ────────────────────────────── */
static DgnVal dgn_list_pop_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST || args[0].list->len==0) return dgn_null(); return args[0].list->data[--args[0].list->len]; }
static DgnVal dgn_list_insert_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<3 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list; int idx = (int)(args[1].tag==DGN_INT?args[1].i:0); if(idx<0||idx>l->len) return dgn_null(); dgn_list_push(&args[0], dgn_null()); for(int i=l->len-1; i>idx; i--) l->data[i] = l->data[i-1]; l->data[idx] = args[2]; return dgn_null(); }
static DgnVal dgn_list_remove_at_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list; int idx = (int)(args[1].tag==DGN_INT?args[1].i:0); if(idx<0||idx>=l->len) return dgn_null(); DgnVal v = l->data[idx]; for(int i=idx; i<l->len-1; i++) l->data[i] = l->data[i+1]; l->len--; return v; }
static DgnVal dgn_list_reverse_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list; for(int i=0; i<l->len/2; i++) { DgnVal tmp = l->data[i]; l->data[i] = l->data[l->len-1-i]; l->data[l->len-1-i] = tmp; } return args[0]; }
static DgnVal dgn_list_first_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST || args[0].list->len==0) return dgn_null(); return args[0].list->data[0]; }
static DgnVal dgn_list_last_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST || args[0].list->len==0) return dgn_null(); return args[0].list->data[args[0].list->len-1]; }
static DgnVal dgn_list_contains_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<2 || args[0].tag!=DGN_LIST) return dgn_bool(0); for(int i=0; i<args[0].list->len; i++) if(dgn_truthy(dgn_eq(args[0].list->data[i], args[1]))) return dgn_bool(1); return dgn_bool(0); }
static DgnVal dgn_list_sum_method(DgnVal* args, int n, DgnEnv* e) { (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_int(0); double sum=0; int is_float=0; for(int i=0; i<args[0].list->len; i++) { if(args[0].list->data[i].tag==DGN_FLOAT) { is_float=1; sum+=args[0].list->data[i].f; } else if(args[0].list->data[i].tag==DGN_INT) sum+=(double)args[0].list->data[i].i; } return is_float ? dgn_float(sum) : dgn_int((int64_t)sum); }

/* Bytes specific */
static DgnVal dgn_list_to_string_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_str("");
    DgnList* l = args[0].list;
    char* buf = (char*)dgn_gc_alloc(l->len + 1, DGN_STR);
    for(int i=0; i<l->len; i++) { buf[i] = (char)((args[0].list->data[i].tag == DGN_INT) ? args[0].list->data[i].i : 0); }
    buf[l->len] = '\0';
    DgnVal v; v.tag = DGN_STR; v.s = buf; return v;
}

/* Functional Operators */
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
static DgnVal dgn_list_sort_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<1 || args[0].tag!=DGN_LIST) return dgn_null(); DgnList* l = args[0].list;
    for(int i=0; i<l->len-1; i++) { for(int j=0; j<l->len-i-1; j++) { if(dgn_truthy(dgn_gt(l->data[j], l->data[j+1]))) { DgnVal tmp = l->data[j]; l->data[j] = l->data[j+1]; l->data[j+1] = tmp; } } } return args[0];
}
static DgnVal dgn_list_sort_by_method(DgnVal* args, int n, DgnEnv* e) {
    (void)e; if(n<2 || args[0].tag!=DGN_LIST || args[1].tag!=DGN_FUNC) return dgn_null(); DgnList* l = args[0].list; DgnVal cb = args[1];
    DgnVal* keys = (DgnVal*)malloc(l->len * sizeof(DgnVal));
    for(int i=0; i<l->len; i++) { DgnVal call_args[1] = { l->data[i] }; keys[i] = dgn_call_val(cb, call_args, 1); }
    for(int i=0; i<l->len-1; i++) { for(int j=0; j<l->len-i-1; j++) { if(dgn_truthy(dgn_gt(keys[j], keys[j+1]))) { DgnVal tK = keys[j]; keys[j] = keys[j+1]; keys[j+1] = tK; DgnVal tV = l->data[j]; l->data[j] = l->data[j+1]; l->data[j+1] = tV; } } }
    free(keys); return args[0];
}

DgnVal _dgn_aug_str;
DgnVal _dgn_aug_list;
int _dgn_aug_init = 0;
static void _init_augments(void);

static DgnVal dgn_get_attr(DgnVal obj, const char* key) {
    _init_augments();
    


    if(obj.tag==DGN_MAP || obj.tag==DGN_CLASS || obj.tag==DGN_INSTANCE) {
        /* 1. Check the map itself FIRST to prevent shadowing user/module keys */
        DgnVal v = dgn_map_get(obj, key);
        if (v.tag != DGN_NULL) return v;

        /* 2. Map built-in methods fallback */
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
        if(strcmp(key,"starts_with")==0) return dgn_func(dgn_str_starts_with, 2);
        if(strcmp(key,"ends_with")==0) return dgn_func(dgn_str_ends_with, 2);
        if(strcmp(key,"index_of")==0) return dgn_func(dgn_str_index_of, 2);
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
        
        /* New List / Bytes Methods */
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
        
        /* Advanced operators */
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
    if(obj.tag==DGN_STR&&(idx.tag==DGN_INT||idx.tag==DGN_FLOAT)){ int64_t i=(idx.tag==DGN_INT)?idx.i:(int64_t)idx.f; if(i<0)i+=(int64_t)strlen(obj.s); if(i>=0&&(size_t)i<strlen(obj.s)){ char buf[2]; buf[0]=obj.s[i]; buf[1]=0; return dgn_str(buf); } return dgn_null(); }
    if((obj.tag==DGN_MAP||obj.tag==DGN_INSTANCE)&&idx.tag==DGN_STR) return dgn_map_get(obj,idx.s);
    return dgn_null();
}
static void dgn_set_index(DgnVal* obj, DgnVal idx, DgnVal val) {
    if(obj->tag==DGN_LIST){ int64_t i=(idx.tag==DGN_INT)?idx.i:(int64_t)idx.f; if(i>=0&&i<obj->list->len) obj->list->data[i]=val; return; }
    if((obj->tag==DGN_MAP||obj->tag==DGN_INSTANCE)&&idx.tag==DGN_STR) dgn_map_set(obj,idx.s,val);
}

static DgnVal dgn_iter_get(DgnVal obj, int i) {
    if(obj.tag==DGN_LIST){ if(i>=0&&i<obj.list->len) return obj.list->data[i]; return dgn_null(); }
    if(obj.tag==DGN_STR){ int l=(int)strlen(obj.s); if(i>=0&&i<l){ char buf[2]; buf[0]=obj.s[i]; buf[1]=0; return dgn_str(buf); } return dgn_null(); }
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
    v.func->fn=fn; v.func->arity=arity; v.func->env=NULL;
    return v;
}
static DgnVal dgn_closure(DgnFuncPtr fn, int arity, DgnMap* env) {
    DgnVal v; v.tag=DGN_FUNC;
    v.func=(DgnFunc*)dgn_gc_alloc(sizeof(DgnFunc), DGN_FUNC);
    v.func->fn=fn; v.func->arity=arity; v.func->env=env;
    return v;
}
/* Raw map accessors that bypass DgnVal wrapping — used by closure env */
/* Raw map accessors that bypass DgnVal wrapping — used by closure env */
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
static DgnVal dgn_call_val(DgnVal fv, DgnVal* args, int n) {
    if(fv.tag==DGN_FUNC&&fv.func&&fv.func->fn) {
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
    DgnVal* nargs=(DgnVal*)malloc((size_t)(n+1)*sizeof(DgnVal)); nargs[0]=obj; memcpy(nargs+1,args,(size_t)n*sizeof(DgnVal));
    DgnVal result=dgn_call_val(fn,nargs,n+1); free(nargs); return result;
}

static DgnVal dgn_class_new(const char* name, const char* parent_name) {
    DgnVal v=dgn_map_new(); v.tag=DGN_CLASS;
    if(name) dgn_map_set(&v,"__name__",dgn_str(name));
    (void)parent_name; return v;
}
static void dgn_class_add_method(DgnVal* cls, const char* name, DgnFuncPtr fn, int arity) { dgn_map_set(cls,name,dgn_func(fn,arity)); }
static void dgn_class_set_parent(DgnVal* cls, DgnVal parent) { dgn_map_set(cls,"__parent__",parent); }
static DgnVal dgn_new_instance(DgnVal cls, DgnVal* args, int n) {
    DgnVal inst; inst.tag=DGN_INSTANCE;
    inst.map=(DgnMap*)dgn_gc_alloc(sizeof(DgnMap), DGN_INSTANCE); inst.map->cap=8; inst.map->entries=(DgnMapEntry*)calloc(8,sizeof(DgnMapEntry));
    if(cls.tag==DGN_CLASS||cls.tag==DGN_MAP){ for(int i=0;i<cls.map->cap;i++) if(cls.map->entries[i].used) dgn_map_set(&inst,cls.map->entries[i].key,cls.map->entries[i].val); }
    /* Run the synthesized field-default initializer (from bare `name = expr;`
       declarations in the class body) fresh for this instance, BEFORE init()
       so an explicit init() can still override any of them. This must be a
       real per-instance call (not a value copied from the class map) so that
       mutable defaults like `threads = list();` don't get shared across
       every instance of the class. */
    DgnVal field_init_fn=dgn_map_get(inst,"__field_init__");
    if(field_init_fn.tag==DGN_FUNC){ DgnVal fi_args[1]; fi_args[0]=inst; dgn_call_val(field_init_fn,fi_args,1); }
    DgnVal init_fn=dgn_map_get(inst,"init");
    if(init_fn.tag==DGN_FUNC){ DgnVal* nargs=(DgnVal*)malloc((size_t)(n+1)*sizeof(DgnVal)); nargs[0]=inst; memcpy(nargs+1,args,(size_t)n*sizeof(DgnVal)); dgn_call_val(init_fn,nargs,n+1); free(nargs); }
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

#define MAX_TRY_DEPTH 256
static DgnTryFrame* _dgn_try_stack[MAX_TRY_DEPTH];
static int _dgn_try_top = 0;
static void dgn_try_push(DgnTryFrame* f) { if (_dgn_try_top >= MAX_TRY_DEPTH) { fprintf(stderr, "Dragon Error: Try/Catch overflow\n"); exit(1); } _dgn_try_stack[_dgn_try_top++] = f; }
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

/* ════════════════════════════════════════════════════════════
   File I/O Module
   ════════════════════════════════════════════════════════════ */
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
    
    // Allocate the string using our new Garbage Collector!
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
    free(content); // dgn_val_to_cstr uses raw malloc, so we free it safely
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
    return dgn_bool(rmdir(args[off].s) == 0);
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

/* ════════════════════════════════════════════════════════════
   Net Module
   ════════════════════════════════════════════════════════════ */

   typedef struct { char url[4096]; char method[8]; char body[65536]; char headers[4096]; } DgnHttpCtx;
typedef struct { char host[256]; int port; } DgnConnectCtx;
typedef struct { int64_t fd; char msg[65536]; } DgnSocketCtx;
typedef struct { int port; DgnVal handler; } DgnServerCtx;

static DgnVal dgn_net_do_http(void* arg);
static DgnVal dgn_make_promise(DgnVal (*fn)(void*), void* ctx);
static DgnVal dgn_net_http_server_task(void* arg);
static DgnVal dgn_socket_make(int64_t fd);
static int64_t dgn_socket_fd(DgnVal v);

/* http_get(url) → Promise<{status, body}> */
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

/* http_post(url, body, headers?) → Promise<{status, body}> */
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
    /* headers map → "Key: Value\r\n" string */
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

/* http_put(url, body, headers?) → Promise */
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

/* http_delete(url) → Promise */
static DgnVal dgn_net_http_delete(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    DgnHttpCtx* ctx = (DgnHttpCtx*)malloc(sizeof(DgnHttpCtx));
    strncpy(ctx->url, args[off].s, sizeof(ctx->url)-1);
    strcpy(ctx->method, "DELETE"); ctx->body[0]=0; ctx->headers[0]=0;
    return dgn_make_promise(dgn_net_http_get_task, ctx);
}

/* socket_connect(host, port) → Promise<socket_map> */
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

/* socket_send(sock, msg) → bool */
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

/* socket_read(sock) → string */
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

/* socket_close(sock) → bool */
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

/* start_http_server(port, handler) → bool (non-blocking — spawns thread) */
static DgnVal dgn_net_start_server(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 2) return dgn_bool(0);
    DgnServerCtx* ctx = (DgnServerCtx*)malloc(sizeof(DgnServerCtx));
    ctx->port    = (int)((args[off].tag==DGN_INT)?args[off].i:(int64_t)args[off].f);
    ctx->handler = args[off+1];
    /* Spawn detached thread — server runs forever in background */
    dgn_make_promise(dgn_net_http_server_task, ctx);
    return dgn_bool(1);
}

/* ════════════════════════════════════════════════════════════
   OS Module
   ════════════════════════════════════════════════════════════ */
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

/* ════════════════════════════════════════════════════════════
   Path Module
   ════════════════════════════════════════════════════════════ */
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

/* ════════════════════════════════════════════════════════════
   Time Module
   ════════════════════════════════════════════════════════════ */
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

/* ════════════════════════════════════════════════════════════
   JSON Module  (minimal — no external dependency)
   ════════════════════════════════════════════════════════════ */
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
            /* escape string */
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
            char* out = (char*)malloc(65536); strcpy(out, "["); int first = 1;
            for(int i = 0; i < v.list->len; i++) {
                if(!first) strcat(out, ","); first = 0;
                char* s = dgn_json_encode_val(v.list->data[i], depth+1);
                if(strlen(out)+strlen(s) < 65530) strcat(out, s); free(s);
            }
            strcat(out, "]"); return out;
        }
        case DGN_MAP: case DGN_INSTANCE: {
            char* out = (char*)malloc(65536); strcpy(out, "{"); int first = 1;
            for(int i = 0; i < v.map->cap; i++) {
                if(!v.map->entries[i].used) continue;
                if(!first) strcat(out, ","); first = 0;
                char kbuf[256]; snprintf(kbuf, sizeof(kbuf), "\"%s\":", v.map->entries[i].key);
                if(strlen(out)+strlen(kbuf) < 65520) strcat(out, kbuf);
                char* s = dgn_json_encode_val(v.map->entries[i].val, depth+1);
                if(strlen(out)+strlen(s) < 65530) strcat(out, s); free(s);
            }
            strcat(out, "}"); return out;
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

/* Minimal recursive-descent JSON parser */
static DgnVal dgn_json_parse_val(const char** p);
static void dgn_json_skip_ws(const char** p) { while(**p && **p <= ' ') (*p)++; }
static DgnVal dgn_json_parse_val(const char** p) {
    dgn_json_skip_ws(p);
    char c = **p;

    /* string */
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
                    case '\\':buf[bi++]='\\'; break; default:  buf[bi++]=**p; break;
                }
            } else { buf[bi++] = **p; }
            (*p)++;
        }
        if(**p == '"') (*p)++;
        buf[bi] = 0;
        DgnVal r = dgn_str(buf); free(buf); return r;
    }

    /* object */
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

    /* array */
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

    /* literals — MUST come before number fallback */
    if(strncmp(*p, "true",  4)==0) { (*p)+=4; return dgn_bool(1); }
    if(strncmp(*p, "false", 5)==0) { (*p)+=5; return dgn_bool(0); }
    if(strncmp(*p, "null",  4)==0) { (*p)+=4; return dgn_null(); }

    /* number */
    char nbuf[64]; int ni = 0; int is_float = 0;
    if(**p == '-') { nbuf[ni++] = **p; (*p)++; }
    while(**p && (isdigit((unsigned char)**p) || **p=='.' || **p=='e' || **p=='E' || **p=='+' || **p=='-')) {
        if(**p=='.' || **p=='e' || **p=='E') is_float = 1;
        if(ni < 62) nbuf[ni++] = **p; (*p)++;
    }
    nbuf[ni] = 0;
    if(ni == 0) { (*p)++; return dgn_null(); } /* unknown char — skip */
    return is_float ? dgn_float(atof(nbuf)) : dgn_int(atoll(nbuf));
}

static DgnVal dgn_json_decode(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    const char* p = args[off].s;
    return dgn_json_parse_val(&p);
}

/* ════════════════════════════════════════════════════════════
   Base64 Module
   ════════════════════════════════════════════════════════════ */
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

/* ════════════════════════════════════════════════════════════
   UUID Module
   ════════════════════════════════════════════════════════════ */
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
    /* pad last group to 12 hex chars */
    snprintf(buf, sizeof(buf), "%08x-%04x-4%03x-%04x-%04x%08x",
        u1, (u2>>16)&0xFFFF, u2&0xFFF,
        ((u3>>16)&0x3FFF)|0x8000,
        u3&0xFFFF, u4);
    return dgn_str(buf);
}

/* ════════════════════════════════════════════════════════════
   Promise  (lightweight thread-based async)
   A Promise is a DgnMap with:
     __promise__ = 1   (sentinel)
     __done__    = 0/1
     __result__  = the resolved DgnVal
   The worker thread writes __done__ and __result__ when finished.
   await in Dragon calls dgn_promise_await() which busy-waits with
   a short sleep — sufficient for I/O tasks.
   ════════════════════════════════════════════════════════════ */

typedef struct {
    DgnVal (*fn)(void* ctx);
    void*  ctx;
    DgnVal result;
    int    done;   /* 1 = finished */
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

/* Spawn a task and return a Promise map */
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

    /* Store task pointer in a map — we encode the pointer as an int */
    DgnVal promise = dgn_map_new();
    dgn_map_set(&promise, "__promise__", dgn_int(1));
    dgn_map_set(&promise, "__task_ptr__", dgn_int((int64_t)(uintptr_t)task));
    return promise;
}

/* Called by ExprNode::Await — resolves the promise synchronously */
static DgnVal dgn_promise_await(DgnVal v) {
    if(v.tag != DGN_MAP) return v;
    DgnVal sentinel = dgn_map_get(v, "__promise__");
    if(sentinel.tag != DGN_INT || sentinel.i != 1) return v; /* not a promise */
    DgnVal ptr_val = dgn_map_get(v, "__task_ptr__");
    if(ptr_val.tag != DGN_INT) return dgn_null();
    DgnTask* task = (DgnTask*)(uintptr_t)(uint64_t)ptr_val.i;
    /* Spin-wait (tasks are I/O — typically <1s) */
    while(!task->done) { DGN_SLEEP(1); }
#ifdef _WIN32
    if(task->thread) { WaitForSingleObject(task->thread, INFINITE); CloseHandle(task->thread); }
#endif
    DgnVal result = task->result;
    free(task);
    return result;
}

/* ════════════════════════════════════════════════════════════
   Socket handle stored in a map
   __socket__ = fd (int64)
   ════════════════════════════════════════════════════════════ */
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

/* ── net task contexts ───────────────────────────────────── */

static DgnVal dgn_net_do_http(void* arg) {
    DgnHttpCtx* ctx = (DgnHttpCtx*)arg;
    DgnVal res = dgn_map_new();

    /* Parse host and path from URL */
    const char* url = ctx->url;
    int use_port = 80;
    char host[256] = {0}; char path[2048] = "/";
    const char* p = url;
    if(strncmp(p, "http://",  7)==0) p += 7;
    else if(strncmp(p, "https://", 8)==0) { p += 8; use_port = 443; } /* HTTPS: fallback to plain for now */
    /* copy host */
    int hi = 0;
    while(*p && *p != '/' && *p != ':' && hi < 255) host[hi++] = *p++;
    host[hi] = 0;
    if(*p == ':') { p++; use_port = atoi(p); while(*p && *p != '/') p++; }
    if(*p == '/') { strncpy(path, p, sizeof(path)-1); }

    /* Connect */
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

    /* Build HTTP request */
    char req[8192];
    int rlen = snprintf(req, sizeof(req),
        "%s %s HTTP/1.0\r\nHost: %s\r\nConnection: close\r\n",
        ctx->method, path, host);
    /* add custom headers */
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

    /* Read response */
    char resp_buf[65536]; int resp_len = 0; int n;
    while((n = (int)recv(fd, resp_buf+resp_len, sizeof(resp_buf)-resp_len-1, 0)) > 0) resp_len += n;
    resp_buf[resp_len] = 0;
#ifdef _WIN32
    closesocket(fd);
#else
    close(fd);
#endif

    /* Parse status line */
    int status_code = 0;
    if(strncmp(resp_buf, "HTTP/", 5)==0) {
        const char* sp = strchr(resp_buf, ' ');
        if(sp) status_code = atoi(sp+1);
    }
    /* Find body after \r\n\r\n */
    const char* body_start = strstr(resp_buf, "\r\n\r\n");
    const char* body = body_start ? body_start + 4 : resp_buf;

    dgn_map_set(&res, "status", dgn_int(status_code));
    dgn_map_set(&res, "body",   dgn_str(body));
    free(ctx);
    return res;
}

/* ── Regex module (POSIX ERE via regcomp/regexec) ─────────── */
/*
   We store a compiled regex as a DgnMap with:
     __regex__   = 1  (sentinel)
     __pattern__ = the pattern string (for display)
   The actual compiled regex is embedded in a malloc'd block
   pointed to by __regex_ptr__ (int64 storing the pointer).
*/
#ifndef _WIN32
#  include <regex.h>
#  define DGN_HAS_POSIX_REGEX 1
#endif

#ifdef _WIN32
/* On Windows we implement a very small subset via a state machine
   rather than pulling in a third-party lib.  Full POSIX ERE is
   available if you link against a regex library; for now we provide
   the API surface with a basic implementation. */
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
        return dgn_null(); /* invalid pattern */
    }
#endif

    DgnVal m = dgn_map_new();
    dgn_map_set(&m, "__regex__",   dgn_int(1));
    dgn_map_set(&m, "__pattern__", dgn_str(pat));
    dgn_map_set(&m, "__ptr__",     dgn_int((int64_t)(uintptr_t)impl));
    
    /* Attach native methods so it behaves like a Rust DgnValue::Regex object */
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

/* regex.test(re, str) → bool */
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
    /* fallback: simple substring via strstr on pattern (no regex engine) */
    int r = strstr(s, impl->pattern) != NULL;
    free(s); return dgn_bool(r);
#endif
}

/* regex.match(re, str) → string of first match or null */
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

/* regex.find_all(re, str) → list of all matches */
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
        if(m.rm_eo == m.rm_so) { cursor++; continue; } /* zero-length guard */
        int len = m.rm_eo - m.rm_so;
        char* buf = (char*)malloc(len+1); memcpy(buf, cursor+m.rm_so, len); buf[len]=0;
        dgn_list_push(&list, dgn_str(buf)); free(buf);
        cursor += m.rm_eo;
    }
#endif
    free(s); return list;
}

/* regex.replace(re, str, replacement) → new string */
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
        /* copy prefix */
        int plen = m.rm_so;
        if(oi+plen < 65530) { memcpy(out+oi, cursor, plen); oi+=plen; }
        /* copy replacement */
        int rlen = (int)strlen(repl);
        if(oi+rlen < 65530) { memcpy(out+oi, repl, rlen); oi+=rlen; }
        cursor += m.rm_eo;
    }
    /* copy remainder */
    int rem = (int)strlen(cursor);
    if(oi+rem < 65530) { memcpy(out+oi, cursor, rem); oi+=rem; }
    out[oi]=0;
    DgnVal r = dgn_str(out); free(s); free(repl); return r;
#else
    DgnVal r = dgn_str(s); free(s); free(repl); return r;
#endif
}

/* regex.groups(re, str) → list of capture groups or null */
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

/* ── HTTP Server (blocking, single-threaded, good for scripts) */

static DgnVal dgn_net_res_send_fn(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    /* args[0] = res map, args[1] = body, args[2] = status (opt), args[3] = headers map (opt) */
    if(n < 2) return dgn_null();
    DgnVal res_map = args[0];
    char* body   = dgn_val_to_cstr(args[1]);
    int   status = (n >= 3 && args[2].tag == DGN_INT) ? (int)args[2].i : 200;
    const char* status_text = (status == 200) ? "OK" :
                              (status == 404) ? "Not Found" :
                              (status == 500) ? "Internal Server Error" : "OK";

    /* Build custom headers string */
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
        /* parse method + path */
        char method[16]={0},path[1024]={0};
        sscanf(buf,"%15s %1023s",method,path);
        /* find body */
        const char* body = strstr(buf,"\r\n\r\n"); body = body ? body+4 : "";
        /* build req object */
        DgnVal req = dgn_map_new();
        dgn_map_set(&req,"method",dgn_str(method));
        dgn_map_set(&req,"path",  dgn_str(path));
        dgn_map_set(&req,"body",  dgn_str(body));
        /* build res object */
        DgnVal res = dgn_map_new();
        /* We store the client fd in res so send() can use it */
        dgn_map_set(&res,"__client__",dgn_int((int64_t)(uintptr_t)(void*)(intptr_t)client));
        dgn_map_set(&res, "send", dgn_func(dgn_net_res_send_fn, -1));
        /* Call handler synchronously */
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

/* ── Global Built-ins ────────────────────────────────────── */
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
            i++; /* skip '}' */
        } else { if(out_idx < 8190) out[out_idx++] = fmt[i]; }
    }
    out[out_idx] = 0; return dgn_str(out);
}

/* ── Mem Module (Raw Pointer & Memory Access) ────────────── */
#define DGN_MEM_OFF(a) ((intptr_t)((a).tag == DGN_FLOAT ? (int64_t)(a).f : (a).i))


static DgnVal dgn_mem_alloc(DgnVal* args, int n, DgnEnv* e) {
    (void)e; int ao = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    int size = (n - ao > 0 && args[ao].tag == DGN_INT) ? (int)args[ao].i :
               (n - ao > 0 && args[ao].tag == DGN_FLOAT) ? (int)args[ao].f : 0;
    if (size <= 0) size = 1;
    
    /* Use plain malloc so this buffer is never freed by the GC.
       Static globals (_v_argc_buf etc.) are invisible to the conservative
       stack scanner, so dgn_gc_alloc would collect them mid-run. */
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
    /* DGN_STR: raw byte buffer returned by mem.alloc — read directly from char* */
    if (args[ao].tag == DGN_STR) {
        const unsigned char* buf = (const unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        uint32_t val = buf[off]
                     | ((uint32_t)buf[off+1] << 8)
                     | ((uint32_t)buf[off+2] << 16)
                     | ((uint32_t)buf[off+3] << 24);
        return dgn_float((double)val);
    }
    /* DGN_LIST: legacy byte-array representation */
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
    /* DGN_STR: raw byte buffer returned by mem.alloc */
    if (args[ao].tag == DGN_STR) {
        unsigned char* buf = (unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        buf[off]   = val & 0xFF;
        buf[off+1] = (val >> 8)  & 0xFF;
        buf[off+2] = (val >> 16) & 0xFF;
        buf[off+3] = (val >> 24) & 0xFF;
        return dgn_null();
    }
    /* DGN_LIST: legacy byte-array representation */
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
    /* DGN_STR: raw byte buffer returned by mem.alloc */
    if (args[ao].tag == DGN_STR) {
        const unsigned char* buf = (const unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        return dgn_int((int64_t)buf[off]);
    }
    /* DGN_LIST: legacy byte-array representation */
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
    /* DGN_STR: raw byte buffer returned by mem.alloc */
    if (args[ao].tag == DGN_STR) {
        unsigned char* buf = (unsigned char*)args[ao].s;
        if (!buf || off < 0) return dgn_null();
        int64_t val = args[ao + 2].tag == DGN_FLOAT ? (int64_t)args[ao + 2].f : args[ao + 2].i;
        buf[off] = (unsigned char)(val & 0xFF);
        return dgn_null();
    }
    /* DGN_LIST: legacy byte-array representation */
    if (args[ao].tag == DGN_LIST) {
        if (off >= 0 && off < args[ao].list->len) args[ao].list->data[off] = args[ao + 2];
    }
    return dgn_null();
}

/* ── FFI Module (Matches engine.rs API exactly) ──────────── */
#ifdef _WIN32
#  include <windows.h>
#else
#  include <dlfcn.h>
#endif

/* Global signature registry for ffi.def() — not static so dgn_gc_collect can root it via extern */
DgnVal _dgn_ffi_sigs; 

/* ── Struct Module Globals ───────────────────────────────── */
typedef struct {
    char* name;
    int offset;
    int size;   /* byte size of the field */
    int align;  /* alignment requirement */
} DgnStructField;

typedef struct {
    char* name;
    int size;
    DgnStructField* fields;
    int num_fields;
} DgnStructDef;

/* A simple dynamic array for struct definitions */
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
    // Shift if args[0] is the ffi module
    int off = (n > 0 && args[0].tag == DGN_MAP) ? 1 : 0;
    
    if(n - off < 1 || args[off].tag != DGN_STR) return dgn_null();
    void* handle = NULL;
#ifdef _WIN32
    handle = LoadLibraryA(args[off].s);
#else
    handle = dlopen(args[off].s, RTLD_LAZY);
#endif
    if(!handle) return dgn_null();
    
    DgnMap* raw = (DgnMap*)malloc(sizeof(DgnMap));
    raw->cap = 8; raw->len = 0;
    raw->entries = (DgnMapEntry*)calloc(8, sizeof(DgnMapEntry));
    DgnVal m; m.tag = DGN_MAP; m.map = raw;
    dgn_map_set(&m, "path", args[off]);
    dgn_map_set(&m, "ptr", dgn_int((int64_t)(uintptr_t)handle));
    return m;
}

static DgnVal dgn_ffi_def(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    // Shift if args[0] is the ffi module (it lacks a "ptr" key)
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

/* ── FFI argument type tags ───────────────────────────────── */
/* Each argument slot is either a pointer/int (P) or a float (F) or double (D).
   C's calling convention passes floats/doubles in XMM registers, not GP registers,
   so we must use typed function-pointer signatures.  We enumerate all combinations
   up to 6 args (3 bits per slot → impractical as a full table; instead we use a
   runtime dispatch loop that re-encodes the signature as a small bitmask). */

/* Argument union for the dispatch helpers */
typedef union { void* p; float f; double d; } DgnFfiArg;

/* Argument kind */
#define DGN_FA_PTR  0
#define DGN_FA_FLT  1
#define DGN_FA_DBL  2

/* We use libffi when available and fall back to a hand-rolled dispatcher
   for the most common OpenGL-style signatures (all-float, up to 6 args).
   The hand-rolled path covers every combination that matters in practice. */

/* Typed void-returning callers for pure-float/double GL-style functions.
   The compiler will emit the correct XMM-register ABI for each. */
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

/* Generic integer/pointer returning callers (unchanged) */
typedef void* (DGN_STDCALL *FfiFunc0)();
typedef void* (DGN_STDCALL *FfiFunc1)(void*);
typedef void* (DGN_STDCALL *FfiFunc2)(void*, void*);
typedef void* (DGN_STDCALL *FfiFunc3)(void*, void*, void*);
typedef void* (DGN_STDCALL *FfiFunc4)(void*, void*, void*, void*);
typedef void* (DGN_STDCALL *FfiFunc5)(void*, void*, void*, void*, void*);
typedef void* (DGN_STDCALL *FfiFunc6)(void*, void*, void*, void*, void*, void*);

static DgnVal dgn_ffi_call(DgnVal* args, int n, DgnEnv* env) {
    (void)env;
    /* Skip `ffi` module map if called as a module method */
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

   /* Build per-argument kind + value arrays */
    int limit = (n - 2 > 11) ? 11 : (n - 2);
    int   kinds[11]   = {0};   /* DGN_FA_PTR / DGN_FA_FLT / DGN_FA_DBL */
    DgnFfiArg cargs[11];
    memset(cargs, 0, sizeof(cargs));

    /* Scratch storage for defensive string copies */
    char* str_copies[11] = {NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL};

    for(int i = 0; i < limit; i++) {
        DgnVal val = args[2 + i];
        switch(val.tag) {
            case DGN_INT:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)(uintptr_t)val.i;
                break;
            case DGN_FLOAT:
                /* Dragon stores all numbers as double internally.
                   Default to float (single-precision) so that
                   float(x) → glVertex2f / glColor3f etc. work.
                   The caller can use double() in Dragon if a native
                   double argument is actually needed. */
                kinds[i]    = DGN_FA_FLT;
                cargs[i].f  = (float)val.f;
                break;
            case DGN_BOOL:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)(uintptr_t)(val.i ? 1 : 0);
                break;
            case DGN_STR: {
                /* Defensive copy — Win32 output-buffer APIs write into this */
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
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = (void*)(uintptr_t)ptr_val.i;
                break;
            }
            default:
                kinds[i]    = DGN_FA_PTR;
                cargs[i].p  = NULL;
                break;
        }
    }

    /* Promote DGN_FA_FLT → DGN_FA_DBL for args declared "double" in ffi.def */
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

    /* ── Dispatch ──────────────────────────────────────────────
       Check whether ALL arguments are floats/doubles so we can
       use a properly-typed caller that puts values in XMM regs.
       Mixed pointer+float cases are also handled for the most
       common patterns (e.g. glVertexAttrib* style).
       Fall through to the generic void*-only path otherwise.
    ─────────────────────────────────────────────────────────── */

    /* Count float vs pointer args */
    int n_flt = 0, n_dbl = 0, n_ptr = 0;
    for(int i = 0; i < limit; i++) {
        if(kinds[i] == DGN_FA_FLT)      n_flt++;
        else if(kinds[i] == DGN_FA_DBL) n_dbl++;
        else                             n_ptr++;
    }
        

    /* All-float signatures (most common for OpenGL) */
    if(limit > 0 && n_ptr == 0 && n_dbl == 0) {
        /* Pure float call — void return (GL-style) */
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

    /* All-double signatures */
    /* All-double signatures */
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

    /* Common mixed: first arg is pointer, rest are floats (e.g. glVertexAttrib) */
    if(n_ptr == 1 && kinds[0] == DGN_FA_PTR && n_flt > 0 && n_flt == limit - 1) {
        switch(limit) {
            case 3: ((DgnVoidFn_pff  )fn_ptr)(cargs[0].p, cargs[1].f, cargs[2].f); break;
            case 4: ((DgnVoidFn_pfff )fn_ptr)(cargs[0].p, cargs[1].f, cargs[2].f, cargs[3].f); break;
            case 5: ((DgnVoidFn_pffff)fn_ptr)(cargs[0].p, cargs[1].f, cargs[2].f, cargs[3].f, cargs[4].f); break;
            default: {
                /* Fallback: generic pointer dispatch (loses float precision) */
                void* pa[6] = { cargs[0].p, cargs[1].p, cargs[2].p, cargs[3].p, cargs[4].p, cargs[5].p };
                if(limit == 2)      ((FfiFunc2)fn_ptr)(pa[0], pa[1]);
                else if(limit >= 6) ((FfiFunc6)fn_ptr)(pa[0], pa[1], pa[2], pa[3], pa[4], pa[5]);
                break;
            }
        }
        for(int i = 0; i < limit; i++) if(str_copies[i]) free(str_copies[i]);
        return dgn_null();
    }
/* Mixed ptr+double (e.g. gluCylinder, gluSphere) */
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
    /* ── Generic pointer/int-only path (original behaviour) ── */
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
            dgn_map_set(&m, "read_all",   dgn_func(dgn_file_read_text, 1));    /* engine: read_all */
            dgn_map_set(&m, "read_bytes",  dgn_func(dgn_file_read_bytes, 1));
            dgn_map_set(&m, "write",      dgn_func(dgn_file_write_text, 2));   /* engine: write */
            dgn_map_set(&m, "append",     dgn_func(dgn_file_append_text, 2));  /* engine: append */
            dgn_map_set(&m, "exists",     dgn_func(dgn_file_exists, 1));
            dgn_map_set(&m, "delete",     dgn_func(dgn_file_delete, 1));
            dgn_map_set(&m, "mkdir",      dgn_func(dgn_file_mkdir, 1));
            dgn_map_set(&m, "rmdir",      dgn_func(dgn_file_rmdir, 1));
            dgn_map_set(&m, "rename",     dgn_func(dgn_file_rename, 2));
            dgn_map_set(&m, "size",       dgn_func(dgn_file_size, 1));
            dgn_map_set(&m, "is_dir",     dgn_func(dgn_file_is_dir, 1));
        }
        else if(strcmp(name,"os")==0) {
    /* os.name, os.cwd, os.getenv, os.setenv, os.system, os.listdir, os.chdir */
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
        /* Initialize the signature registry if empty */
        if(_dgn_ffi_sigs.tag == DGN_NULL) _dgn_ffi_sigs = dgn_map_new();
        
        dgn_map_set(&m, "loadlib",      dgn_func(dgn_ffi_loadlib,      2));  // this + path
dgn_map_set(&m, "def",          dgn_func(dgn_ffi_def,          5));  // this + 4 args
dgn_map_set(&m, "call",         dgn_func(dgn_ffi_call,        -1));  // this + variadic
dgn_map_set(&m, "symbol_addr",  dgn_func(dgn_ffi_symbol_addr,  3));  // this + 2 args
dgn_map_set(&m, "loadlib_call", dgn_func(dgn_ffi_loadlib_call, 4));  // this + 3 args
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

static char** _dgn_argv=NULL; static int _dgn_argc=0;
static void dgn_runtime_init(int argc, char** argv, void* stack_bottom) { 
    _dgn_argc=argc; _dgn_argv=argv; srand((unsigned)time(NULL)); 
    _dgn_stack_bottom = stack_bottom;
}
/* ============================================================
   End of Dragon Native Runtime
   ============================================================ */
"#;
