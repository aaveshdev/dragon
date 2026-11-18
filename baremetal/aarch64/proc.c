#include <stdint.h>
int dgn_proc_create(uint64_t base, uint64_t size) { (void)base; (void)size; return -1; }
void dgn_proc_destroy(int pid) { (void)pid; }
int dgn_proc_alive(int pid) { (void)pid; return 0; }
uint64_t dgn_proc_base(int pid) { (void)pid; return 0; }
uint64_t dgn_proc_size(int pid) { (void)pid; return 0; }
uint64_t dgn_proc_mem(int pid) { (void)pid; return 0; }
int dgn_proc_copy(int pid, uint64_t ua, void* k, uint64_t n, int to_user) { (void)pid; (void)ua; (void)k; (void)n; (void)to_user; return -1; }
int64_t dgn_proc_strlen(int pid, uint64_t ua, uint64_t max) { (void)pid; (void)ua; (void)max; return -1; }
int dgn_proc_start(int pid, uint64_t entry, uint64_t sp, uint64_t a0, uint64_t a1) { (void)pid; (void)entry; (void)sp; (void)a0; (void)a1; return -1; }
uint64_t dgn_proc_reg(int pid, int i) { (void)pid; (void)i; return 0; }
void dgn_proc_set_reg(int pid, int i, uint64_t v) { (void)pid; (void)i; (void)v; }
int dgn_proc_run(int pid) { (void)pid; return -1; }
int dgn_proc_map_shared(int pid, uint64_t va, uint64_t kaddr, uint64_t n) { (void)pid; (void)va; (void)kaddr; (void)n; return -1; }
int dgn_proc_set_exec(int pid, uint64_t va, uint64_t n, int on) { (void)pid; (void)va; (void)n; (void)on; return 0; }
int dgn_proc_release(int pid, uint64_t va, uint64_t n) { (void)pid; (void)va; (void)n; return -1; }
int dgn_proc_fork(int pid) { (void)pid; return -1; }
uint64_t dgn_proc_frames_used(void) { return 0; }
uint64_t dgn_proc_frames_total(void) { return 0; }
int dgn_proc_clone(int pid) { (void)pid; return -1; }
void dgn_proc_mark_ctx(void (*mark)(void*)) { (void)mark; }
