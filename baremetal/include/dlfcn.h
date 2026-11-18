#ifndef DGN_BM_DLFCN_H
#define DGN_BM_DLFCN_H
#define RTLD_LAZY 1
void* dlopen(const char*, int); void* dlsym(void*, const char*); int dlclose(void*); char* dlerror(void);
#endif
