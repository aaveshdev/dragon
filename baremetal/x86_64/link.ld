/* x86-64 bare-metal layout for QEMU "pc": loaded at 1 MB; the heap runs from the end of the image to the end of RAM
 * (found at run time from the CMOS memory size), the boot stack is in .bss. */
ENTRY(_pvh_start)
SECTIONS
{
    . = 1M;
    .multiboot : { KEEP(*(.multiboot)) }
    .note   : { *(.note.Xen) *(.note .note.*) }
    .bootinfo ALIGN(4096) : { __bootinfo = .; . += 4096; }
    .text   : { *(.text.init) *(.text .text.*) }
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) }
    .data   : { *(.data .data.*) }
    .bss    : ALIGN(4096) { __bss_start = .; *(.bss .bss.* COMMON) . = ALIGN(16); __bss_end = .; }
    .bootbss (NOLOAD) : ALIGN(4096) { *(.bootbss) }
    . = ALIGN(4096);
    __heap_start = .;
    __heap_end = __heap_start;       /* the real end is dgn_bm_ram_end() */
}
