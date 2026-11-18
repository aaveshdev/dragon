#include <stdint.h>
int dgn_ata_rw(uint64_t sector, int count, void* addr, int write) { (void)sector; (void)count; (void)addr; (void)write; return 0; }
unsigned int dgn_ata_probe(void) { return 0; }
