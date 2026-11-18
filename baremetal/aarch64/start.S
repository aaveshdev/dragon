    .section .text.init
    .globl _start
_start:
    adrp    x1, dgn_boot_x0
    add     x1, x1, :lo12:dgn_boot_x0            // x0 is the firmware's boot information (UEFI loader) or the device tree (QEMU -kernel)
    str     x0, [x1]
    mrs     x0, CurrentEL
    lsr     x0, x0, #2
    cmp     x0, #2
    b.ne    1f
    mov     x0, #(1 << 31)              // EL2: EL1 runs AArch64, then drop to EL1
    msr     hcr_el2, x0
    mrs     x0, id_aa64pfr0_el1         // GICv3 system registers at EL1 need ICC_SRE_EL2 enabled
    ubfx    x0, x0, #24, #4
    cbz     x0, 4f
    mov     x0, #0xf
    msr     S3_4_C12_C9_5, x0           // ICC_SRE_EL2
    isb
4:
    mov     x0, #0x3c5
    msr     spsr_el2, x0
    adr     x0, 1f
    msr     elr_el2, x0
    eret
1:
    mrs     x1, sctlr_el1               // entered from UEFI with its own MMU and caches on: switch them off, the tables below replace them
    bic     x1, x1, #1
    bic     x1, x1, #4
    bic     x1, x1, #(1 << 12)
    msr     sctlr_el1, x1
    isb
    mrs     x1, mpidr_el1
    and     x1, x1, #0xff
    cbnz    x1, park                    // only CPU 0 runs
    adrp    x0, __stack_top
    add     x0, x0, :lo12:__stack_top
    mov     sp, x0
    mov     x0, #(3 << 20)              // CPACR_EL1.FPEN: FP and SIMD on
    msr     cpacr_el1, x0
    adr     x0, vectors
    msr     vbar_el1, x0
    adrp    x0, dgn_boot_x0
    add     x0, x0, :lo12:dgn_boot_x0
    ldr     x0, [x0]
    bl      dgn_bm_mmu_prep
    // memory map: 0-1 GB device, 1-4 GB normal cacheable RAM (identity mapped, 1 GB blocks)
    ldr     x0, =0xFF00                 // MAIR: attr0 device-nGnRnE, attr1 normal write-back
    msr     mair_el1, x0
    ldr     x0, =0x200803518            // TCR: 1 TB VA, 4 KB granule, write-back walks, 40-bit PA, no TTBR1
    msr     tcr_el1, x0
    adrp    x0, l0_table
    add     x0, x0, :lo12:l0_table
    msr     ttbr0_el1, x0
    dsb     sy
    tlbi    vmalle1
    ic      iallu
    dsb     sy
    isb
    mrs     x0, sctlr_el1
    orr     x0, x0, #1                  // MMU
    orr     x0, x0, #4                  // data cache
    orr     x0, x0, #(1 << 12)          // instruction cache
    bic     x0, x0, #2                  // no alignment check
    bic     x0, x0, #8
    msr     sctlr_el1, x0
    isb
    adrp    x0, __bss_start
    add     x0, x0, :lo12:__bss_start
    adrp    x1, __bss_end
    add     x1, x1, :lo12:__bss_end
2:  cmp     x0, x1
    b.hs    3f
    str     xzr, [x0], #8
    b       2b
3:  bl      dgn_bm_entry
park:
    wfi
    b       park

    .section .data
    .globl dgn_boot_x0
    .balign 8
dgn_boot_x0: .quad 0
    .section .text.init
    .section .data
    .balign 4096
    .globl l1_table
    .globl l1_hi
l0_table:
    .quad   l1_table + 3                       // 0 - 512 GB
    .quad   l1_hi + 3                          // 512 GB - 1 TB (the high PCIe window)
    .fill   510, 8, 0
l1_table:
    .quad   0x0000000000000601 | (3 << 53)
    .quad   0x0000000040000705
    .quad   0x0000000080000705
    .quad   0x00000000C0000705
    .fill   252, 8, 0
    .quad   0x0000004000000601 | (3 << 53)     // 0x4000000000: the high PCIe config window
    .fill   255, 8, 0
l1_hi:
    .fill   512, 8, 0

    .section .text.init
    .macro SAVE
    sub sp, sp, #800
    stp x0, x1, [sp, #0]
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    add x0, sp, #800
    str x0, [sp, #248]
    mrs x0, elr_el1
    mrs x1, spsr_el1
    stp x0, x1, [sp, #256]
    mrs x0, fpcr
    mrs x1, fpsr
    stp x0, x1, [sp, #272]
    stp q0, q1, [sp, #288]
    stp q2, q3, [sp, #320]
    stp q4, q5, [sp, #352]
    stp q6, q7, [sp, #384]
    stp q8, q9, [sp, #416]
    stp q10, q11, [sp, #448]
    stp q12, q13, [sp, #480]
    stp q14, q15, [sp, #512]
    stp q16, q17, [sp, #544]
    stp q18, q19, [sp, #576]
    stp q20, q21, [sp, #608]
    stp q22, q23, [sp, #640]
    stp q24, q25, [sp, #672]
    stp q26, q27, [sp, #704]
    stp q28, q29, [sp, #736]
    stp q30, q31, [sp, #768]
    .endm
    .macro RESTORE
    ldp x0, x1, [sp, #256]
    msr elr_el1, x0
    msr spsr_el1, x1
    ldp x0, x1, [sp, #272]
    msr fpcr, x0
    msr fpsr, x1
    ldp q0, q1, [sp, #288]
    ldp q2, q3, [sp, #320]
    ldp q4, q5, [sp, #352]
    ldp q6, q7, [sp, #384]
    ldp q8, q9, [sp, #416]
    ldp q10, q11, [sp, #448]
    ldp q12, q13, [sp, #480]
    ldp q14, q15, [sp, #512]
    ldp q16, q17, [sp, #544]
    ldp q18, q19, [sp, #576]
    ldp q20, q21, [sp, #608]
    ldp q22, q23, [sp, #640]
    ldp q24, q25, [sp, #672]
    ldp q26, q27, [sp, #704]
    ldp q28, q29, [sp, #736]
    ldp q30, q31, [sp, #768]
    ldp x0, x1, [sp, #0]
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    add sp, sp, #800
    eret
    .endm

    .balign 2048
vectors:
    .balign 128
    b       bad_sp0_sync
    .balign 128
    b       bad_sp0_irq
    .balign 128
    b       bad_sp0_fiq
    .balign 128
    b       bad_sp0_serr
    .balign 128
    b       h_sync
    .balign 128
    b       h_irq
    .balign 128
    b       bad_spx_fiq
    .balign 128
    b       bad_spx_serr
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128
    b       bad_lower
    .balign 128

h_sync:
    SAVE
    mov     x0, sp
    bl      dgn_bm_sync_c
    mov     sp, x0
    bl      dgn_sched_post_switch
    RESTORE

h_irq:
    SAVE
    mov     x0, sp
    bl      dgn_bm_irq_c
    mov     sp, x0
    bl      dgn_sched_post_switch
    RESTORE

bad_sp0_sync: mov x1, #0
    b bad
bad_sp0_irq:  mov x1, #1
    b bad
bad_sp0_fiq:  mov x1, #2
    b bad
bad_sp0_serr: mov x1, #3
    b bad
bad_spx_fiq:  mov x1, #6
    b bad
bad_spx_serr: mov x1, #7
    b bad
bad_lower:    mov x1, #8
bad:
    SAVE
    mov     x0, sp
    bl      dgn_bm_fatal_c
    b       park

    .globl setjmp
setjmp:
    stp     x19, x20, [x0, #0]
    stp     x21, x22, [x0, #16]
    stp     x23, x24, [x0, #32]
    stp     x25, x26, [x0, #48]
    stp     x27, x28, [x0, #64]
    stp     x29, x30, [x0, #80]
    mov     x2, sp
    str     x2, [x0, #96]
    stp     d8, d9, [x0, #104]
    stp     d10, d11, [x0, #120]
    stp     d12, d13, [x0, #136]
    stp     d14, d15, [x0, #152]
    mov     w0, #0
    ret

    .globl longjmp
longjmp:
    ldp     x19, x20, [x0, #0]
    ldp     x21, x22, [x0, #16]
    ldp     x23, x24, [x0, #32]
    ldp     x25, x26, [x0, #48]
    ldp     x27, x28, [x0, #64]
    ldp     x29, x30, [x0, #80]
    ldr     x2, [x0, #96]
    mov     sp, x2
    ldp     d8, d9, [x0, #104]
    ldp     d10, d11, [x0, #120]
    ldp     d12, d13, [x0, #136]
    ldp     d14, d15, [x0, #152]
    cmp     w1, #0
    csinc   w0, w1, wzr, ne             // value 0 becomes 1
    ret
