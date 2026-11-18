    .section .text.init, "ax"
    .globl _start
_start:
    movq %rdi, dgn_user_top(%rip)
    movq %rsi, dgn_user_stack(%rip)
    movq %rdx, dgn_user_args(%rip)
    movq %rdi, %rsp
    xorl %ebp, %ebp
    call dgn_user_main
    movl %eax, %edi
    movl $0, %eax                 # SYS_exit
    int $0x81
1:  jmp 1b

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
