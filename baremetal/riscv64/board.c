#include <stdint.h>
#include <stddef.h>

#define UART0     0x10000000UL
#define TEST_DEV  0x100000UL
#define MTIME     0x0200BFF8UL

static volatile uint8_t* const uart = (volatile uint8_t*)UART0;

extern int dgn_bm_con_on, dgn_bm_con_fbonly, dgn_bm_con_seronly;
extern void dgn_bm_con_out(char c);
void dgn_bm_putc(char c) {
    if (!dgn_bm_con_fbonly) { while (!(uart[5] & 0x20)) {} uart[0] = (uint8_t)c; }
    if (dgn_bm_con_on && !dgn_bm_con_seronly) dgn_bm_con_out(c);
}
extern void dgn_bm_task_yield(void);

int  dgn_bm_getc_nb(void) { return (uart[5] & 1) ? uart[0] : -1; }
int  dgn_bm_getc(void) { while (!(uart[5] & 1)) dgn_bm_task_yield(); return uart[0]; }
unsigned long long dgn_bm_now_us(void) { return *(volatile uint64_t*)MTIME / 10; }
void dgn_bm_halt(int code) {
    volatile uint32_t* t = (volatile uint32_t*)TEST_DEV;
    *t = code == 0 ? 0x5555u : (((uint32_t)code << 16) | 0x3333u);
    for (;;) __asm__ volatile("wfi");
}

extern int main(int argc, char** argv);
void dgn_bm_entry(void) {
    char* argv[2] = { "dragon", 0 };
    int rc = main(1, argv);
    dgn_bm_halt(rc);
}
