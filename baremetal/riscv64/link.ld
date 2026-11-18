/* RISC-V 64 bare-metal layout for QEMU "virt": RAM starts at 0x80000000. */
ENTRY(_start)
MEMORY { RAM (rwx) : ORIGIN = 0x80000000, LENGTH = 64M }
STACK_SIZE = 4M;
SECTIONS
{
    .text   : { *(.text.init) *(.text .text.*) } > RAM
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) } > RAM
    .data   : { *(.data .data.* .sdata .sdata.*) } > RAM
    .bss    : ALIGN(8) { __bss_start = .; *(.bss .bss.* .sbss .sbss.* COMMON) . = ALIGN(8); __bss_end = .; } > RAM
    . = ALIGN(16);
    __heap_start = .;
    __heap_end = ORIGIN(RAM) + LENGTH(RAM) - STACK_SIZE;
    __stack_top = ORIGIN(RAM) + LENGTH(RAM);
    __stack_lo = __stack_top - STACK_SIZE;
}
