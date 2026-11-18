#include <stdint.h>
uint64_t dgn_bm_disk_init(void) { return 0; }
int dgn_bm_disk_rw(uint64_t sector, void* buf, uint32_t count, int write) { (void)sector; (void)buf; (void)count; (void)write; return -1; }
