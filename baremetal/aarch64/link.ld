/* AArch64 bare-metal layout for QEMU "virt": RAM starts at 0x40000000, the image sits at 0x40080000. The heap ends where the machine's RAM ends (read at run time). */
ENTRY(_start)
MEMORY { RAM (rwx) : ORIGIN = 0x40080000, LENGTH = 1023M + 512K }
STACK_SIZE = 4M;
SECTIONS
{
    .text   : { *(.text.init) *(.text .text.*) } > RAM
    .rodata : { *(.rodata .rodata.*) } > RAM
    .data   : { *(.data .data.*) } > RAM
    .bss    : ALIGN(16) { __bss_start = .; *(.bss .bss.* COMMON) . = ALIGN(16); __bss_end = .; } > RAM
    .stack (NOLOAD) : ALIGN(4096) { __stack_lo = .; . += STACK_SIZE; __stack_top = .; } > RAM
    . = ALIGN(16);
    __heap_start = .;
    __heap_end = ORIGIN(RAM) + LENGTH(RAM);
}
