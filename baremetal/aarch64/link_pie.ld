/* AArch64 position independent image for the UEFI loader: it picks the address, applies the relative relocations and starts the kernel there */
ENTRY(_start)
STACK_SIZE = 4M;
SECTIONS
{
    . = 0;
    .text   : { *(.text.init) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.*) }
    .data.rel.ro : { *(.data.rel.ro .data.rel.ro.*) }
    .data   : { *(.data .data.*) *(.got .got.*) }
    .bss    : ALIGN(16) { __bss_start = .; *(.bss .bss.* COMMON) . = ALIGN(16); __bss_end = .; }
    .stack (NOLOAD) : ALIGN(4096) { __stack_lo = .; . += STACK_SIZE; __stack_top = .; }
    . = ALIGN(16);
    __heap_start = .;
    __heap_end = .;
}
