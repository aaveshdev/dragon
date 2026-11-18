#ifndef DGN_BM_PTHREAD_H
#define DGN_BM_PTHREAD_H
typedef unsigned long pthread_t;
typedef int pthread_mutex_t;
int pthread_create(pthread_t*, const void*, void* (*)(void*), void*);
int pthread_detach(pthread_t);
int pthread_join(pthread_t, void**);
int pthread_mutex_init(pthread_mutex_t*, const void*);
int pthread_mutex_lock(pthread_mutex_t*);
int pthread_mutex_unlock(pthread_mutex_t*);
int pthread_mutex_destroy(pthread_mutex_t*);
#endif
