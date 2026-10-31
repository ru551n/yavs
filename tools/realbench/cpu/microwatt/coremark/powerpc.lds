/* Linker script of the Microwatt CoreMark image (realbench harness): the hello_world layout with room for the program. */
MEMORY
{
    ram : ORIGIN = 0, LENGTH = 0x60000
}
SECTIONS
{
    . = 0;
    _start = .;
    .head : { KEEP(*(.head)) } > ram
    . = 0x1000;
    .text : { *(.text) *(.text.*) *(.rodata) *(.rodata.*) } > ram
    . = ALIGN(0x1000);
    .data : { *(.data) *(.data.*) *(.got) *(.toc) } > ram
    . = ALIGN(0x80);
    __bss_start = .;
    .bss : { *(.dynsbss) *(.sbss) *(.scommon) *(.dynbss) *(.bss) *(.common) *(.bss.*) } > ram
    . = ALIGN(0x80);
    __bss_end = .;
    . = . + 0x4000;
    __stack_top = .;
}
