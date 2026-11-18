/* A .dxe program: one flat image loaded at the start of the process's private memory (0x8000000000). */
ENTRY(_start)
SECTIONS
{
    . = 0x8000000000;
    .text   : { *(.text.init) *(.text .text.*) *(.ltext .ltext.*) }
    .rodata : { *(.rodata .rodata.* .srodata .srodata.*) *(.lrodata .lrodata.*) }
    .data   : { *(.data .data.*) *(.ldata .ldata.*) }
    .bss    : ALIGN(16) { __bss_start = .; *(.bss .bss.* COMMON) *(.lbss .lbss.*) . = ALIGN(16); __bss_end = .; }
    . = ALIGN(4096);
    __heap_start = .;
    __heap_end = __heap_start;
}
