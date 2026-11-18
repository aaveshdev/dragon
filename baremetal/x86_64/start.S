    .section .multiboot, "a"
    .align 8
mb2_start:
    .long 0xE85250D6
    .long 0
    .long mb2_end - mb2_start
    .long -(0xE85250D6 + (mb2_end - mb2_start))
    .align 8
    .short 5, 0
    .long 20
    .long 1280, 720, 32
    .align 8
    .short 0, 0
    .long 8
mb2_end:

    .section .note.Xen, "a", @note
    .align 4
    .long 4                       # namesz
    .long 4                       # descsz
    .long 18                      # XEN_ELFNOTE_PHYS32_ENTRY
    .asciz "Xen"
    .align 4
    .long _pvh_start

    .section .text.init, "ax"
    .code32
    .globl _pvh_start
_pvh_start:
    cli
    cld
    cmpl $0x36D76289, %eax
    jne 1f
    movl %ebx, mb_info
1:
    movl $boot_stack_top, %esp
    movl $pml4, %edi
    xorl %eax, %eax
    movl $(6 * 1024), %ecx
    rep stosl
    movl $pdpt, %eax
    orl $3, %eax
    movl %eax, pml4
    movl $pdpt, %edi
    movl $pd, %eax
    orl $3, %eax
    movl $4, %ecx
1:  movl %eax, (%edi)
    addl $0x1000, %eax
    addl $8, %edi
    loop 1b
    movl $pd, %edi
    movl $0x83, %eax
    movl $2048, %ecx
2:  movl %eax, (%edi)
    addl $0x200000, %eax
    addl $8, %edi
    loop 2b
    movl $pml4, %eax
    movl %eax, %cr3
    movl %cr4, %eax
    orl $0x20, %eax               # PAE
    movl %eax, %cr4
    pushl %ebx
    movl $0x80000000, %eax
    cpuid
    cmpl $0x80000001, %eax
    jb 3f
    movl $0x80000001, %eax
    cpuid
    btl $20, %edx
    jnc 3f
    movl $0x80000000, dgn_nx_mask+4
    movl $1, nx_on
3:  popl %ebx
    movl $0xC0000080, %ecx        # EFER
    rdmsr
    orl $0x100, %eax              # LME
    cmpl $0, nx_on
    je 4f
    orl $0x800, %eax              # NXE
4:  wrmsr
    movl %cr0, %eax
    orl $0x80000001, %eax         # PG | PE
    movl %eax, %cr0
    lgdt gdt_desc
    ljmp $0x08, $long_mode

    .code64
long_mode:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs
    movq $boot_stack_top, %rsp
    movq %cr0, %rax               # FPU/SSE on
    andq $-5, %rax                # EM = 0
    orq $2, %rax                  # MP = 1
    movq %rax, %cr0
    movq %cr4, %rax
    orq $0x600, %rax              # OSFXSR | OSXMMEXCPT
    movq %rax, %cr4
    leaq __bss_start(%rip), %rdi
    leaq __bss_end(%rip), %rcx
    subq %rdi, %rcx
    xorl %eax, %eax
    rep stosb
    movl $0xC0000101, %ecx        # GS base = per-CPU block of the boot CPU
    movl $dgn_pcpu, %eax
    xorl %edx, %edx
    wrmsr
    call dgn_bm_entry
halt:
    hlt
    jmp halt

    .section .data
    .globl mb_info
mb_info: .long 0
    .text
    .align 8
gdt:
    .quad 0
    .quad 0x00AF9A000000FFFF      # 0x08: 64-bit code
    .quad 0x00CF92000000FFFF      # 0x10: data
gdt_desc:
    .word gdt_desc - gdt - 1
    .long gdt

    .text
    .align 16
stub_0:
    pushq $0
    pushq $0
    jmp trap_common
    .align 16
stub_1:
    pushq $0
    pushq $1
    jmp trap_common
    .align 16
stub_2:
    pushq $0
    pushq $2
    jmp trap_common
    .align 16
stub_3:
    pushq $0
    pushq $3
    jmp trap_common
    .align 16
stub_4:
    pushq $0
    pushq $4
    jmp trap_common
    .align 16
stub_5:
    pushq $0
    pushq $5
    jmp trap_common
    .align 16
stub_6:
    pushq $0
    pushq $6
    jmp trap_common
    .align 16
stub_7:
    pushq $0
    pushq $7
    jmp trap_common
    .align 16
stub_8:
    pushq $8
    jmp trap_common
    .align 16
stub_9:
    pushq $0
    pushq $9
    jmp trap_common
    .align 16
stub_10:
    pushq $10
    jmp trap_common
    .align 16
stub_11:
    pushq $11
    jmp trap_common
    .align 16
stub_12:
    pushq $12
    jmp trap_common
    .align 16
stub_13:
    pushq $13
    jmp trap_common
    .align 16
stub_14:
    pushq $14
    jmp trap_common
    .align 16
stub_15:
    pushq $0
    pushq $15
    jmp trap_common
    .align 16
stub_16:
    pushq $0
    pushq $16
    jmp trap_common
    .align 16
stub_17:
    pushq $17
    jmp trap_common
    .align 16
stub_18:
    pushq $0
    pushq $18
    jmp trap_common
    .align 16
stub_19:
    pushq $0
    pushq $19
    jmp trap_common
    .align 16
stub_20:
    pushq $0
    pushq $20
    jmp trap_common
    .align 16
stub_21:
    pushq $21
    jmp trap_common
    .align 16
stub_22:
    pushq $0
    pushq $22
    jmp trap_common
    .align 16
stub_23:
    pushq $0
    pushq $23
    jmp trap_common
    .align 16
stub_24:
    pushq $0
    pushq $24
    jmp trap_common
    .align 16
stub_25:
    pushq $0
    pushq $25
    jmp trap_common
    .align 16
stub_26:
    pushq $0
    pushq $26
    jmp trap_common
    .align 16
stub_27:
    pushq $0
    pushq $27
    jmp trap_common
    .align 16
stub_28:
    pushq $0
    pushq $28
    jmp trap_common
    .align 16
stub_29:
    pushq $29
    jmp trap_common
    .align 16
stub_30:
    pushq $30
    jmp trap_common
    .align 16
stub_31:
    pushq $0
    pushq $31
    jmp trap_common
    .align 16
stub_32:
    pushq $0
    pushq $32
    jmp trap_common
    .align 16
stub_33:
    pushq $0
    pushq $33
    jmp trap_common
    .align 16
stub_34:
    pushq $0
    pushq $34
    jmp trap_common
    .align 16
stub_35:
    pushq $0
    pushq $35
    jmp trap_common
    .align 16
stub_36:
    pushq $0
    pushq $36
    jmp trap_common
    .align 16
stub_37:
    pushq $0
    pushq $37
    jmp trap_common
    .align 16
stub_38:
    pushq $0
    pushq $38
    jmp trap_common
    .align 16
stub_39:
    pushq $0
    pushq $39
    jmp trap_common
    .align 16
stub_40:
    pushq $0
    pushq $40
    jmp trap_common
    .align 16
stub_41:
    pushq $0
    pushq $41
    jmp trap_common
    .align 16
stub_42:
    pushq $0
    pushq $42
    jmp trap_common
    .align 16
stub_43:
    pushq $0
    pushq $43
    jmp trap_common
    .align 16
stub_44:
    pushq $0
    pushq $44
    jmp trap_common
    .align 16
stub_45:
    pushq $0
    pushq $45
    jmp trap_common
    .align 16
stub_46:
    pushq $0
    pushq $46
    jmp trap_common
    .align 16
stub_47:
    pushq $0
    pushq $47
    jmp trap_common
    .align 16
stub_128:
    pushq $0
    pushq $128
    jmp trap_common
    .align 16
stub_129:
    pushq $0
    pushq $129
    jmp trap_common
    .align 16
stub_239:
    pushq $0
    pushq $239
    jmp trap_common
    .align 16
stub_252:
    pushq $0
    pushq $252
    jmp trap_common
    .align 16
stub_253:
    pushq $0
    pushq $253
    jmp trap_common

    .align 16
trap_common:
    pushq %rax
    pushq %rcx
    pushq %rdx
    pushq %rbx
    pushq %rbp
    pushq %rsi
    pushq %rdi
    pushq %r8
    pushq %r9
    pushq %r10
    pushq %r11
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    subq $512, %rsp
    fxsave (%rsp)
    movq %rsp, %rdi
    call dgn_x86_trap
    movq %rax, %rsp
    call dgn_sched_post_switch
    fxrstor (%rsp)
    addq $512, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rdi
    popq %rsi
    popq %rbp
    popq %rbx
    popq %rdx
    popq %rcx
    popq %rax
    addq $16, %rsp
    iretq

    .globl dgn_x86_resume
dgn_x86_resume:
    movq %rdi, %rsp
    fxrstor (%rsp)
    addq $512, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %r11
    popq %r10
    popq %r9
    popq %r8
    popq %rdi
    popq %rsi
    popq %rbp
    popq %rbx
    popq %rdx
    popq %rcx
    popq %rax
    addq $16, %rsp
    iretq

    .section .rodata
    .globl dgn_x86_stubs
    .align 8
dgn_x86_stubs:
    .quad stub_0
    .quad stub_1
    .quad stub_2
    .quad stub_3
    .quad stub_4
    .quad stub_5
    .quad stub_6
    .quad stub_7
    .quad stub_8
    .quad stub_9
    .quad stub_10
    .quad stub_11
    .quad stub_12
    .quad stub_13
    .quad stub_14
    .quad stub_15
    .quad stub_16
    .quad stub_17
    .quad stub_18
    .quad stub_19
    .quad stub_20
    .quad stub_21
    .quad stub_22
    .quad stub_23
    .quad stub_24
    .quad stub_25
    .quad stub_26
    .quad stub_27
    .quad stub_28
    .quad stub_29
    .quad stub_30
    .quad stub_31
    .quad stub_32
    .quad stub_33
    .quad stub_34
    .quad stub_35
    .quad stub_36
    .quad stub_37
    .quad stub_38
    .quad stub_39
    .quad stub_40
    .quad stub_41
    .quad stub_42
    .quad stub_43
    .quad stub_44
    .quad stub_45
    .quad stub_46
    .quad stub_47
    .quad stub_128
    .quad stub_129
    .quad stub_239
    .quad stub_252
    .quad stub_253

    .text
    .globl setjmp
setjmp:
    movq %rbx, 0(%rdi)
    movq %rbp, 8(%rdi)
    movq %r12, 16(%rdi)
    movq %r13, 24(%rdi)
    movq %r14, 32(%rdi)
    movq %r15, 40(%rdi)
    leaq 8(%rsp), %rdx
    movq %rdx, 48(%rdi)
    movq (%rsp), %rdx
    movq %rdx, 56(%rdi)
    xorl %eax, %eax
    ret

    .globl longjmp
longjmp:
    movl %esi, %eax
    testl %eax, %eax
    jnz 1f
    movl $1, %eax
1:  movq 0(%rdi), %rbx
    movq 8(%rdi), %rbp
    movq 16(%rdi), %r12
    movq 24(%rdi), %r13
    movq 32(%rdi), %r14
    movq 40(%rdi), %r15
    movq 48(%rdi), %rsp
    jmp *56(%rdi)

    .section .bootbss, "aw", @nobits
    .align 4096
    .globl pml4
pml4:   .skip 4096
    .globl pdpt
pdpt:   .skip 4096
    .globl pd
pd:     .skip 16384
    .globl pd_hi
pd_hi:  .skip 16384
    .align 16
    .globl __stack_lo
__stack_lo:
    .skip 4194304
boot_stack_top:

    .section .data
    .align 8
    .globl dgn_nx_mask
dgn_nx_mask: .quad 0
nx_on: .long 0
