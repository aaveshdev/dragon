#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>

typedef struct {
    uint8_t fx[512];
    uint64_t r15, r14, r13, r12, r11, r10, r9, r8, rdi, rsi, rbp, rbx, rdx, rcx, rax;
    uint64_t vec, err, rip, cs, rflags, rsp, ss;
} Frame;

extern void* dgn_sched_tick(void* f);
extern void* dgn_sched_yield_trap(void* f);
extern void* dgn_sched_tick_cpu(void* f);
extern void dgn_bm_stw_park_frame(void* f, int from_user);
extern void dgn_smp_eoi(void);
extern void dgn_smp_ap_poll(void);
extern int dgn_bm_stw_pending(void);
extern int dgn_sched_cur(void);
extern const uint64_t dgn_x86_stubs[];
void dgn_bm_halt(int code) __attribute__((noreturn));
extern void dgn_x86_gdt_init(void);
extern void dgn_x86_user_trap(Frame* f) __attribute__((noreturn));

static inline void outb(uint16_t p, uint8_t v) { __asm__ volatile("outb %0, %1" :: "a"(v), "Nd"(p)); }
static inline uint8_t inb(uint16_t p) { uint8_t v; __asm__ volatile("inb %1, %0" : "=a"(v) : "Nd"(p)); return v; }

typedef struct { uint16_t lo; uint16_t sel; uint8_t ist, attr; uint16_t mid; uint32_t hi; uint32_t zero; } __attribute__((packed)) IdtEnt;
static IdtEnt idt[256] __attribute__((aligned(16)));

void dgn_arch_yield(void) { __asm__ volatile("int $0x80" ::: "memory"); }

void* dgn_arch_make_frame(char* stack, size_t size, void (*tramp)(void*), void* a0) {
    char* top = (char*)(((uintptr_t)stack + size) & ~(uintptr_t)15);
    Frame* f = (Frame*)(top - sizeof(Frame) - 8 - 8);
    f = (Frame*)((uintptr_t)f & ~(uintptr_t)15);
    memset(f, 0, sizeof *f);
    *(uint16_t*)(f->fx + 0) = 0x037F;
    *(uint32_t*)(f->fx + 24) = 0x1F80;
    f->rdi = (uint64_t)(uintptr_t)a0;
    f->rip = (uint64_t)(uintptr_t)tramp;
    f->cs = 0x08; f->ss = 0x10; f->rflags = 0x202;
    f->rsp = (uint64_t)(uintptr_t)top - 8;
    return f;
}

void dgn_arch_start(void) {
    outb(0x21, 0xFE);
    __asm__ volatile("sti");
}
void dgn_arch_idle(void) { for (;;) __asm__ volatile("hlt"); }

void dgn_x86_init(void) {

    outb(0x20, 0x11); outb(0xA0, 0x11);
    outb(0x21, 32); outb(0xA1, 40);
    outb(0x21, 4); outb(0xA1, 2);
    outb(0x21, 1); outb(0xA1, 1);
    outb(0x21, 0xFF); outb(0xA1, 0xFF);

    outb(0x43, 0x34); outb(0x40, 11932 & 0xFF); outb(0x40, 11932 >> 8);
    for (int i = 0; i < 48; i++) {
        uint64_t a = dgn_x86_stubs[i];
        idt[i] = (IdtEnt){ (uint16_t)a, 0x08, 0, 0x8E, (uint16_t)(a >> 16), (uint32_t)(a >> 32), 0 };
    }
    uint64_t a = dgn_x86_stubs[48];
    idt[0x80] = (IdtEnt){ (uint16_t)a, 0x08, 0, 0x8E, (uint16_t)(a >> 16), (uint32_t)(a >> 32), 0 };
    a = dgn_x86_stubs[49];
    idt[0x81] = (IdtEnt){ (uint16_t)a, 0x08, 0, 0xEE, (uint16_t)(a >> 16), (uint32_t)(a >> 32), 0 };
    static const uint8_t ipi_vec[3] = { 0xEF, 0xFC, 0xFD };
    for (int i = 0; i < 3; i++) {
        a = dgn_x86_stubs[50 + i];
        idt[ipi_vec[i]] = (IdtEnt){ (uint16_t)a, 0x08, 0, 0x8E, (uint16_t)(a >> 16), (uint32_t)(a >> 32), 0 };
    }
    dgn_x86_gdt_init();
    struct { uint16_t limit; uint64_t base; } __attribute__((packed)) d = { sizeof idt - 1, (uint64_t)(uintptr_t)idt };
    __asm__ volatile("lidt %0" :: "m"(d));
}

void dgn_x86_load_idt(void) {
    struct { uint16_t limit; uint64_t base; } __attribute__((packed)) d = { sizeof idt - 1, (uint64_t)(uintptr_t)idt };
    __asm__ volatile("lidt %0" :: "m"(d));
}

Frame* dgn_x86_trap(Frame* f) {
    uint64_t v = f->vec;
    if (v == 0xFC) { dgn_smp_eoi(); dgn_bm_stw_park_frame(f, (f->cs & 3) == 3); return f; }
    if (v == 0xEF || v == 0xFD) {
        dgn_smp_eoi();
        if ((f->cs & 3) == 3) return f;
        if (dgn_bm_stw_pending()) dgn_bm_stw_park_frame(f, 0);
        return (Frame*)dgn_sched_tick_cpu(f);
    }
    if ((f->cs & 3) == 3) {
        if (v == 32) outb(0x20, 0x20);
        else if (v > 32 && v < 48) { if (v >= 40) outb(0xA0, 0x20); outb(0x20, 0x20); return f; }
        dgn_x86_user_trap(f);
    }
    if (v == 32) { outb(0x20, 0x20); return (Frame*)dgn_sched_tick(f); }
    if (v == 0x80) return (Frame*)dgn_sched_yield_trap(f);
    if (v >= 32 && v < 48) { if (v >= 40) outb(0xA0, 0x20); outb(0x20, 0x20); return f; }
    uint64_t cr2; __asm__ volatile("movq %%cr2, %0" : "=r"(cr2));
    printf("\n[dragon] fatal exception %lu: err=%lx rip=%lx cr2=%lx rsp=%lx (task %d)\n", (unsigned long)v, (unsigned long)f->err,
           (unsigned long)f->rip, (unsigned long)cr2, (unsigned long)f->rsp, dgn_sched_cur());
    dgn_bm_halt(1);
}
