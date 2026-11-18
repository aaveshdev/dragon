    .section .text.init
    .globl _start
_start:
    csrr    t0, mhartid
    bnez    t0, park              # only hart 0 runs; the others wait
    la      sp, __stack_top
    la      t0, trap_vector
    csrw    mtvec, t0
    li      t0, 0x6000            # mstatus.FS = dirty: turn the FPU on
    csrs    mstatus, t0
    la      t0, __bss_start
    la      t1, __bss_end
1:  bgeu    t0, t1, 2f
    sd      zero, 0(t0)
    addi    t0, t0, 8
    j       1b
2:  call    dgn_bm_entry
park:
    wfi
    j       park

    .align 4
trap_vector:
    addi sp, sp, -528
    sd x1, 8(sp)
    sd x3, 24(sp)
    sd x4, 32(sp)
    sd x5, 40(sp)
    sd x6, 48(sp)
    sd x7, 56(sp)
    sd x8, 64(sp)
    sd x9, 72(sp)
    sd x10, 80(sp)
    sd x11, 88(sp)
    sd x12, 96(sp)
    sd x13, 104(sp)
    sd x14, 112(sp)
    sd x15, 120(sp)
    sd x16, 128(sp)
    sd x17, 136(sp)
    sd x18, 144(sp)
    sd x19, 152(sp)
    sd x20, 160(sp)
    sd x21, 168(sp)
    sd x22, 176(sp)
    sd x23, 184(sp)
    sd x24, 192(sp)
    sd x25, 200(sp)
    sd x26, 208(sp)
    sd x27, 216(sp)
    sd x28, 224(sp)
    sd x29, 232(sp)
    sd x30, 240(sp)
    sd x31, 248(sp)
    addi t0, sp, 528
    sd t0, 16(sp)
    fsd f0, 256(sp)
    fsd f1, 264(sp)
    fsd f2, 272(sp)
    fsd f3, 280(sp)
    fsd f4, 288(sp)
    fsd f5, 296(sp)
    fsd f6, 304(sp)
    fsd f7, 312(sp)
    fsd f8, 320(sp)
    fsd f9, 328(sp)
    fsd f10, 336(sp)
    fsd f11, 344(sp)
    fsd f12, 352(sp)
    fsd f13, 360(sp)
    fsd f14, 368(sp)
    fsd f15, 376(sp)
    fsd f16, 384(sp)
    fsd f17, 392(sp)
    fsd f18, 400(sp)
    fsd f19, 408(sp)
    fsd f20, 416(sp)
    fsd f21, 424(sp)
    fsd f22, 432(sp)
    fsd f23, 440(sp)
    fsd f24, 448(sp)
    fsd f25, 456(sp)
    fsd f26, 464(sp)
    fsd f27, 472(sp)
    fsd f28, 480(sp)
    fsd f29, 488(sp)
    fsd f30, 496(sp)
    fsd f31, 504(sp)
    csrr t0, mepc
    sd t0, 512(sp)
    csrr t0, mstatus
    sd t0, 520(sp)
    mv a0, sp
    call dgn_bm_trap_c
    mv sp, a0
    call dgn_sched_post_switch
    ld t0, 512(sp)
    csrw mepc, t0
    ld t0, 520(sp)
    csrw mstatus, t0
    fld f0, 256(sp)
    fld f1, 264(sp)
    fld f2, 272(sp)
    fld f3, 280(sp)
    fld f4, 288(sp)
    fld f5, 296(sp)
    fld f6, 304(sp)
    fld f7, 312(sp)
    fld f8, 320(sp)
    fld f9, 328(sp)
    fld f10, 336(sp)
    fld f11, 344(sp)
    fld f12, 352(sp)
    fld f13, 360(sp)
    fld f14, 368(sp)
    fld f15, 376(sp)
    fld f16, 384(sp)
    fld f17, 392(sp)
    fld f18, 400(sp)
    fld f19, 408(sp)
    fld f20, 416(sp)
    fld f21, 424(sp)
    fld f22, 432(sp)
    fld f23, 440(sp)
    fld f24, 448(sp)
    fld f25, 456(sp)
    fld f26, 464(sp)
    fld f27, 472(sp)
    fld f28, 480(sp)
    fld f29, 488(sp)
    fld f30, 496(sp)
    fld f31, 504(sp)
    ld x1, 8(sp)
    ld x3, 24(sp)
    ld x4, 32(sp)
    ld x5, 40(sp)
    ld x6, 48(sp)
    ld x7, 56(sp)
    ld x8, 64(sp)
    ld x9, 72(sp)
    ld x10, 80(sp)
    ld x11, 88(sp)
    ld x12, 96(sp)
    ld x13, 104(sp)
    ld x14, 112(sp)
    ld x15, 120(sp)
    ld x16, 128(sp)
    ld x17, 136(sp)
    ld x18, 144(sp)
    ld x19, 152(sp)
    ld x20, 160(sp)
    ld x21, 168(sp)
    ld x22, 176(sp)
    ld x23, 184(sp)
    ld x24, 192(sp)
    ld x25, 200(sp)
    ld x26, 208(sp)
    ld x27, 216(sp)
    ld x28, 224(sp)
    ld x29, 232(sp)
    ld x30, 240(sp)
    ld x31, 248(sp)
    ld sp, 16(sp)
    mret

    .globl setjmp
setjmp:
    sd ra,0(a0);   sd sp,8(a0)
    sd s0,16(a0);  sd s1,24(a0);  sd s2,32(a0);  sd s3,40(a0)
    sd s4,48(a0);  sd s5,56(a0);  sd s6,64(a0);  sd s7,72(a0)
    sd s8,80(a0);  sd s9,88(a0);  sd s10,96(a0); sd s11,104(a0)
    fsd fs0,112(a0);  fsd fs1,120(a0);  fsd fs2,128(a0);  fsd fs3,136(a0)
    fsd fs4,144(a0);  fsd fs5,152(a0);  fsd fs6,160(a0);  fsd fs7,168(a0)
    fsd fs8,176(a0);  fsd fs9,184(a0);  fsd fs10,192(a0); fsd fs11,200(a0)
    li a0, 0
    ret

    .globl longjmp
longjmp:
    ld ra,0(a0);   ld sp,8(a0)
    ld s0,16(a0);  ld s1,24(a0);  ld s2,32(a0);  ld s3,40(a0)
    ld s4,48(a0);  ld s5,56(a0);  ld s6,64(a0);  ld s7,72(a0)
    ld s8,80(a0);  ld s9,88(a0);  ld s10,96(a0); ld s11,104(a0)
    fld fs0,112(a0);  fld fs1,120(a0);  fld fs2,128(a0);  fld fs3,136(a0)
    fld fs4,144(a0);  fld fs5,152(a0);  fld fs6,160(a0);  fld fs7,168(a0)
    fld fs8,176(a0);  fld fs9,184(a0);  fld fs10,192(a0); fld fs11,200(a0)
    seqz a0, a1
    add  a0, a0, a1               # value 0 becomes 1
    ret
