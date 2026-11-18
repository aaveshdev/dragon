/* Single-core stub: multi-core start-up is only implemented for x86-64. */
#include <stdint.h>
int dgn_smp_init(void) { return 1; }
int dgn_smp_count(void) { return 1; }
int dgn_smp_start(int cpu, int job, uint64_t arg) { (void)cpu; (void)job; (void)arg; return -1; }
int dgn_smp_busy(int cpu) { (void)cpu; return 0; }
uint64_t dgn_smp_result(int cpu) { (void)cpu; return 0; }
void dgn_smp_parallel(void (*f)(void*, int, int), void* ctx) { f(ctx, 0, 1); }
void dgn_smp_set_parallel(int on) { (void)on; }
